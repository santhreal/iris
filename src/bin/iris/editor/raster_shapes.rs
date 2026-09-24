//! CPU rasterization primitives: scanline fills, stamps, and polyline coverage.

/// Fill the inclusive span [x0, x1] of one row's byte slice with
/// `src`. `blend_row` borrows the row out of the image; the banded
/// fills borrow it out of a par_bands_mut chunk, so the blend itself
/// works on a bare slice either way.
pub(crate) fn blend_span(row: &mut [u8], x0: i64, x1: i64, src: image::Rgba<u8>) {
    if x0 > x1 {
        return;
    }
    let w = (row.len() / 4) as i64;
    let (x0, x1) = (x0.max(0), x1.min(w - 1));
    if x0 > x1 {
        return;
    }
    let span = &mut row[(x0 as usize * 4)..=(x1 as usize * 4) + 3];
    let a = src.0[3] as f32 / 255.0;
    if a >= 1.0 {
        span.as_chunks_mut::<4>().0.fill(src.0);
        return;
    }
    let inv = 1.0 - a;
    for px in span.as_chunks_mut::<4>().0 {
        px[0] = (src.0[0] as f32 * a + px[0] as f32 * inv) as u8;
        px[1] = (src.0[1] as f32 * a + px[1] as f32 * inv) as u8;
        px[2] = (src.0[2] as f32 * a + px[2] as f32 * inv) as u8;
        px[3] = px[3].max(src.0[3]);
    }
}

/// Fill the inclusive span [x0, x1] of row `y` with `src`, bounds
/// already clamped by the caller. One slice borrow per row instead of
/// `blend`'s per-pixel as_mut + bounds check.
pub(crate) fn blend_row(
    dst: &mut image::RgbaImage,
    y: i64,
    x0: i64,
    x1: i64,
    src: image::Rgba<u8>,
) {
    if x0 > x1 || y < 0 || y >= dst.height() as i64 {
        return;
    }
    let stride = dst.width() as usize * 4;
    let row_start = y as u32 as usize * stride;
    let buf: &mut [u8] = dst.as_mut();
    blend_span(&mut buf[row_start..row_start + stride], x0, x1, src);
}

/// Fill the rect [x0,x1]x[y0,y1] banded across rows. A large solid
/// fill is O(area) blends; below par_bands_mut's 1MB gate it runs
/// inline, above it the rows split across cores.
pub(crate) fn fill_rect(
    img: &mut image::RgbaImage,
    x0: i64,
    x1: i64,
    y0: i64,
    y1: i64,
    src: image::Rgba<u8>,
) {
    let stride = img.width() as usize * 4;
    let (x0, x1) = (x0.max(0), x1.min(img.width() as i64 - 1));
    let (y0, y1) = (y0.max(0), y1.min(img.height() as i64 - 1));
    if x0 > x1 || y0 > y1 {
        return;
    }
    iris_lib::par::par_bands_mut(img.as_mut(), stride, |band, start| {
        let first_row = (start / stride) as i64;
        let band_rows = (band.len() / stride) as i64;
        for y in first_row.max(y0)..=(first_row + band_rows - 1).min(y1) {
            let row =
                &mut band[(y - first_row) as usize * stride..(y - first_row + 1) as usize * stride];
            blend_span(row, x0, x1, src);
        }
    });
}

/// Fill an ellipse banded across rows: each row's chord is one sqrt,
/// and the rows split across cores for a large fill.
pub(crate) fn fill_ellipse(
    img: &mut image::RgbaImage,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    src: image::Rgba<u8>,
) {
    let stride = img.width() as usize * 4;
    let w = img.width() as i64;
    let (y0, y1) = (
        (cy - ry).max(0.0) as i64,
        (cy + ry).min(img.height() as f32 - 1.0) as i64,
    );
    iris_lib::par::par_bands_mut(img.as_mut(), stride, |band, start| {
        let first_row = (start / stride) as i64;
        let band_rows = (band.len() / stride) as i64;
        for y in first_row.max(y0)..=(first_row + band_rows - 1).min(y1) {
            let t = (y as f32 - cy) / ry;
            let half = rx * (1.0 - t * t).max(0.0).sqrt();
            let x0 = (cx - half).max(0.0) as i64;
            let x1 = (cx + half).min(w as f32 - 1.0) as i64;
            let row =
                &mut band[(y - first_row) as usize * stride..(y - first_row + 1) as usize * stride];
            blend_span(row, x0, x1, src);
        }
    });
}

/// Per-row chord spans of a disc: each row's covered interval is one
/// sqrt, instead of testing dx*dx+dy*dy per pixel. `f` receives
/// (y, x_lo, x_hi) unclamped; the sink clamps to its target.
pub(crate) fn disc_spans(cx: f32, cy: f32, r: f32, mut f: impl FnMut(i64, f32, f32)) {
    let (y0, y1) = ((cy - r).floor() as i64, (cy + r).ceil() as i64);
    for y in y0..=y1 {
        let dy = y as f32 - cy;
        let half = (r * r - dy * dy).max(0.0).sqrt();
        f(y, cx - half, cx + half);
    }
}

pub(crate) fn stamp(img: &mut image::RgbaImage, cx: f32, cy: f32, r: f32, px: image::Rgba<u8>) {
    let r = r.max(0.5);
    disc_spans(cx, cy, r, |y, x0, x1| {
        blend_row(img, y, x0.floor() as i64, x1.ceil() as i64, px);
    });
}

/// Per-row chord spans of the capsule (stadium) around segment a-b
/// with radius r: the body's offset edges are a±r*n to b±r*n, plus a
/// disc chord at each end cap. `f` receives (y, x_lo, x_hi) unclamped.
pub(crate) fn segment_spans(
    a: (f32, f32),
    b: (f32, f32),
    r: f32,
    mut f: impl FnMut(i64, f32, f32),
) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    if len < f32::EPSILON {
        disc_spans(a.0, a.1, r, f);
        return;
    }
    // Unit normal: the body's offset edges are a±r*n to b±r*n.
    let (nx, ny) = (-dy / len, dx / len);
    let y0 = (a.1.min(b.1) - r).floor() as i64;
    let y1 = (a.1.max(b.1) + r).ceil() as i64;
    for y in y0..=y1 {
        let yf = y as f32;
        // Body interval: |signed distance to the line| <= r AND the
        // projection parameter t in [0,1]. Both are linear in x, so
        // each is one interval; the body is their intersection.
        let body = (|| {
            let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
            if nx.abs() > f32::EPSILON {
                let base = a.0 - (yf - a.1) * ny / nx;
                let half = (r / nx).abs();
                lo = lo.max(base - half);
                hi = hi.min(base + half);
            } else if ((yf - a.1) * ny).abs() > r {
                return None;
            }
            if dx.abs() > f32::EPSILON {
                let x_t0 = a.0 - (yf - a.1) * dy / dx;
                let x_t1 = x_t0 + len * len / dx;
                lo = lo.max(x_t0.min(x_t1));
                hi = hi.min(x_t0.max(x_t1));
            } else {
                let t = (yf - a.1) * dy / (len * len);
                if !(0.0..=1.0).contains(&t) {
                    return None;
                }
            }
            (lo <= hi).then_some((lo, hi))
        })();
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        if let Some((bl, bh)) = body {
            lo = lo.min(bl);
            hi = hi.max(bh);
        }
        // End-cap chords.
        for &(cx, cy) in &[a, b] {
            let dyc = yf - cy;
            if dyc.abs() <= r {
                let half = (r * r - dyc * dyc).max(0.0).sqrt();
                lo = lo.min(cx - half);
                hi = hi.max(cx + half);
            }
        }
        if lo <= hi {
            f(y, lo, hi);
        }
    }
}

/// Fill the capsule (stadium) around segment a-b with radius w/2.
/// The old per-point disc stamps covered the same region but blended
/// each pixel ~3x along every straight run: opaque strokes were
/// idempotent, but a semi-transparent highlight compounded to ~0.73
/// alpha where the canvas shows a uniform 0.35. One blend per pixel
/// is both faster and matches the GPU path.
pub(crate) fn stamp_segment(
    img: &mut image::RgbaImage,
    a: (f32, f32),
    b: (f32, f32),
    w: f32,
    px: image::Rgba<u8>,
) {
    let r = (w / 2.0).max(0.5);
    segment_spans(a, b, r, |y, lo, hi| {
        blend_row(img, y, lo.floor() as i64, hi.ceil() as i64, px);
    });
}

/// Mark the capsule's coverage into a byte mask (bw-wide, origin at
/// (ox, oy)): translucent strokes stamp every segment's coverage
/// first, then blend each covered pixel exactly once.
pub(crate) fn cover_segment(
    mask: &mut [u8],
    bw: usize,
    ox: i64,
    oy: i64,
    a: (f32, f32),
    b: (f32, f32),
    r: f32,
) {
    let bh = mask.len() / bw.max(1);
    segment_spans(a, b, r, |y, lo, hi| {
        let my = y - oy;
        if my < 0 || my >= bh as i64 {
            return;
        }
        let s = (lo.floor() as i64 - ox).clamp(0, bw as i64) as usize;
        let e = (hi.ceil() as i64 - ox + 1).clamp(0, bw as i64) as usize;
        mask[my as usize * bw + s..my as usize * bw + e].fill(1);
    });
}

pub(crate) fn stroke_polyline(
    img: &mut image::RgbaImage,
    points: &[(f32, f32)],
    w: f32,
    px: image::Rgba<u8>,
) {
    if points.len() == 1 {
        stamp(img, points[0].0, points[0].1, w / 2.0, px);
        return;
    }
    if px.0[3] < 255 {
        // Translucent stroke (highlight): adjacent capsules overlap
        // at every joint, and two blends compound both the alpha and
        // the RGB toward the stroke color; a corner reads as a
        // darker dot in the saved PNG while the canvas tessellates
        // the polyline once and shows a uniform wash. Stamp each
        // segment's coverage into a mask over the stroke's bbox,
        // then blend every covered pixel exactly once.
        let r = (w / 2.0).max(0.5);
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in points {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let ih = img.height() as i64;
        let ox = (x0 - r).floor() as i64;
        let oy = (y0 - r).floor() as i64;
        let bx = (x1 + r).ceil() as i64;
        let by = (y1 + r).ceil() as i64;
        let (bw, bh) = ((bx - ox).max(0) as usize, (by - oy).max(0) as usize);
        if bw == 0 || bh == 0 {
            return;
        }
        let mut mask = vec![0u8; bw * bh];
        for seg in points.windows(2) {
            cover_segment(&mut mask, bw, ox, oy, seg[0], seg[1], r);
        }
        for my in 0..bh as i64 {
            let y = oy + my;
            if y < 0 || y >= ih {
                continue;
            }
            let row = &mask[my as usize * bw..my as usize * bw + bw];
            let mut run: Option<usize> = None;
            for (i, &m) in row.iter().enumerate() {
                match (m != 0, run) {
                    (true, None) => run = Some(i),
                    (false, Some(s)) => {
                        blend_row(img, y, ox + s as i64, ox + i as i64 - 1, px);
                        run = None;
                    }
                    _ => {}
                }
            }
            if let Some(s) = run {
                blend_row(img, y, ox + s as i64, ox + bw as i64 - 1, px);
            }
        }
        return;
    }
    for seg in points.windows(2) {
        stamp_segment(img, seg[0], seg[1], w, px);
    }
}

pub(crate) fn stamp_arrow_head(
    img: &mut image::RgbaImage,
    a: (f32, f32),
    b: (f32, f32),
    w: f32,
    px: image::Rgba<u8>,
) {
    let head = 14.0 + w;
    let angle = (b.1 - a.1).atan2(b.0 - a.0);
    let p1 = (
        b.0 - head * (angle - 0.45).cos(),
        b.1 - head * (angle - 0.45).sin(),
    );
    let p2 = (
        b.0 - head * (angle + 0.45).cos(),
        b.1 - head * (angle + 0.45).sin(),
    );
    fill_triangle(img, b, p1, p2, px);
}

/// Scanline fill of triangle (v0, v1, v2): each row's chord is the
/// intersection of the two edges the row crosses. The old head fill
/// stamped ~10 overlapping 2px capsules across the span, blending
/// every pixel several times; one blend per pixel is faster and
/// matches the GPU path's single coverage.
pub(crate) fn fill_triangle(
    img: &mut image::RgbaImage,
    v0: (f32, f32),
    v1: (f32, f32),
    v2: (f32, f32),
    px: image::Rgba<u8>,
) {
    // Sort vertices by y: top, middle, bottom.
    let mut vs = [v0, v1, v2];
    vs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let (t, m, b) = (vs[0], vs[1], vs[2]);
    if (b.1 - t.1).abs() < f32::EPSILON {
        // Degenerate: a flat line. Stamp it as a thin segment.
        stamp_segment(img, t, b, 1.0, px);
        return;
    }
    let y0 = t.1.floor().max(0.0) as i64;
    let y1 = b.1.ceil().min(img.height() as f32 - 1.0) as i64;
    for y in y0..=y1 {
        let yf = y as f32;
        // Long edge t->b is always crossed; the short edge switches
        // from t->m to m->b at m's row.
        let u_long = (yf - t.1) / (b.1 - t.1);
        let xl = t.0 + (b.0 - t.0) * u_long;
        let xs = if yf <= m.1 {
            if (m.1 - t.1).abs() < f32::EPSILON {
                m.0
            } else {
                t.0 + (m.0 - t.0) * ((yf - t.1) / (m.1 - t.1))
            }
        } else if (b.1 - m.1).abs() < f32::EPSILON {
            m.0
        } else {
            m.0 + (b.0 - m.0) * ((yf - m.1) / (b.1 - m.1))
        };
        blend_row(
            img,
            y,
            xl.min(xs).floor() as i64,
            xl.max(xs).ceil() as i64,
            px,
        );
    }
}
