//! Rasterization into CPU image buffers: actions, text, blur patches, PNG I/O.

use super::action::{hex_rgba, Action, Tool};
use super::raster_shapes::*;

pub(crate) const BLUR_BLOCK: u32 = 12;

/// Width and height from the PNG IHDR, without decoding. The full
/// decode runs in the background; the window opens on these dims.
pub(crate) fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    Some((
        u32::from_be_bytes(png[16..20].try_into().ok()?),
        u32::from_be_bytes(png[20..24].try_into().ok()?),
    ))
}

pub(crate) fn png_bytes(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut bytes,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    img.write_with_encoder(encoder)
        .map_err(|e| format!("encode PNG: {e}"))?;
    Ok(bytes)
}

/// Pixelate a region of a CPU image in place.
/// Pixelate a region of `img` in place and return the same pixels as a
/// BGRA tile for the GPU sprite. One box-average pass replaces the old
/// crop + Triangle downscale + Nearest upscale + overlay chain: each
/// source pixel is read once for its cell average and each destination
/// pixel written once, with no intermediate images and no per-pixel
/// blend on overlay. The mosaic is visually identical at BLUR_BLOCK
/// scale; only block-edge weighting differs from the old filter pair.
pub(crate) fn pixelate_region_bgra(
    img: &mut image::RgbaImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Vec<u8> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let iw = img.width() as usize;
    let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
    let (sw, sh) = (
        (w / BLUR_BLOCK as usize).max(1),
        (h / BLUR_BLOCK as usize).max(1),
    );
    let src = img.as_raw();
    // Phase 1: average each cell's source rect into `small`. Cells
    // partition the region, so every source pixel is read once.
    let mut small = vec![0u8; sw * sh * 4];
    iris_lib::par::par_bands_mut_work(&mut small, sw * 4, w * h * 4, |band, start| {
        let first = start / (sw * 4);
        for (cy, row) in band.chunks_exact_mut(sw * 4).enumerate() {
            let cy = first + cy;
            let sy0 = cy * h / sh;
            let sy1 = ((cy + 1) * h / sh).max(sy0 + 1);
            for cx in 0..sw {
                let sx0 = cx * w / sw;
                let sx1 = ((cx + 1) * w / sw).max(sx0 + 1);
                let mut sum = [0u32; 4];
                for sy in sy0..sy1 {
                    let base = ((y + sy) * iw + x) * 4;
                    for sx in sx0..sx1 {
                        let i = base + sx * 4;
                        sum[0] += src[i] as u32;
                        sum[1] += src[i + 1] as u32;
                        sum[2] += src[i + 2] as u32;
                        sum[3] += src[i + 3] as u32;
                    }
                }
                let n = ((sx1 - sx0) * (sy1 - sy0)) as u32;
                let r = n / 2;
                let o = cx * 4;
                row[o] = ((sum[0] + r) / n) as u8;
                row[o + 1] = ((sum[1] + r) / n) as u8;
                row[o + 2] = ((sum[2] + r) / n) as u8;
                row[o + 3] = ((sum[3] + r) / n) as u8;
            }
        }
    });
    // Phase 2: expand cells into the region and the BGRA tile. Column
    // cell indices are row-invariant; compute them once.
    let col_cell: Vec<usize> = (0..w).map(|ox| ox * sw / w).collect();
    let data: &mut [u8] = img.as_mut();
    // Whole rows keep chunks_exact_mut aligned to iw*4; the region's
    // column range is written inside each row.
    let rows = &mut data[y * iw * 4..(y + h) * iw * 4];
    iris_lib::par::par_bands_mut_work(rows, iw * 4, w * h * 4, |band, start| {
        let first = start / (iw * 4);
        for (oy, row) in band.chunks_exact_mut(iw * 4).enumerate() {
            let cy = (first + oy) * sh / h;
            let srow = &small[cy * sw * 4..cy * sw * 4 + sw * 4];
            let row = &mut row[x * 4..x * 4 + w * 4];
            for (ox, &cx) in col_cell.iter().enumerate() {
                row[ox * 4..ox * 4 + 4].copy_from_slice(&srow[cx * 4..cx * 4 + 4]);
            }
        }
    });
    let mut bgra = Vec::with_capacity(w * h * 4);
    #[allow(clippy::uninit_vec)]
    // SAFETY: every byte is written by the banded fill below before the
    // Vec is read; the bands partition the buffer exactly.
    unsafe {
        bgra.set_len(w * h * 4);
    }
    iris_lib::par::par_bands_mut_work(&mut bgra, w * 4, w * h * 4, |band, start| {
        let first = start / (w * 4);
        for (oy, row) in band.chunks_exact_mut(w * 4).enumerate() {
            let cy = (first + oy) * sh / h;
            let srow = &small[cy * sw * 4..cy * sw * 4 + sw * 4];
            for (ox, &cx) in col_cell.iter().enumerate() {
                let s = &srow[cx * 4..cx * 4 + 4];
                let o = ox * 4;
                row[o] = s[2];
                row[o + 1] = s[1];
                row[o + 2] = s[0];
                row[o + 3] = s[3];
            }
        }
    });
    bgra
}

/// Rasterize one action into a CPU image (save path and blur sampling).
/// Strokes are stamped circles along the primitive's outline, which keeps
/// every tool at the same visual weight without a tessellation crate.
pub(crate) fn rasterize(img: &mut image::RgbaImage, action: &Action, alpha_mul: f32) {
    let c = hex_rgba(action.color);
    let px = image::Rgba([
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8,
        (c.a * alpha_mul * 255.0) as u8,
    ]);
    let w = action.width;
    match action.tool {
        Tool::Pen => stroke_polyline(img, &action.points, w, px),
        Tool::Highlight => {
            let mut px = px;
            px.0[3] = (0.35 * 255.0) as u8;
            stroke_polyline(img, &action.points, w, px);
        }
        Tool::Line => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                stamp_segment(img, *a, *b, w, px);
            }
        }
        Tool::Arrow => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                stamp_segment(img, *a, *b, w, px);
                stamp_arrow_head(img, *a, *b, w, px);
            }
        }
        Tool::Ellipse => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                let cx = (a.0 + b.0) / 2.0;
                let cy = (a.1 + b.1) / 2.0;
                let rx = (b.0 - a.0).abs() / 2.0;
                let ry = (b.1 - a.1).abs() / 2.0;
                if rx > 0.0 && ry > 0.0 {
                    if action.filled {
                        // Banded scanline fill: each row's chord is one
                        // sqrt, and the rows split across cores.
                        fill_ellipse(img, cx, cy, rx, ry, px);
                    } else {
                        let n = ((rx + ry) * 0.35).max(24.0) as usize;
                        for i in 0..n {
                            let t = i as f32 / n as f32 * std::f32::consts::TAU;
                            stamp(img, cx + rx * t.cos(), cy + ry * t.sin(), w / 2.0, px);
                        }
                    }
                }
            }
        }
        Tool::Rect => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                let (tl, br) = (*a, *b);
                if action.filled {
                    // Banded scanline fill: the rows split across cores
                    // for a large solid rect.
                    let (x0, x1) = (tl.0.min(br.0), tl.0.max(br.0));
                    let (y0, y1) = (tl.1.min(br.1), tl.1.max(br.1));
                    fill_rect(img, x0 as i64, x1 as i64, y0 as i64, y1 as i64, px);
                } else {
                    stamp_segment(img, (tl.0, tl.1), (br.0, tl.1), w, px);
                    stamp_segment(img, (br.0, tl.1), (br.0, br.1), w, px);
                    stamp_segment(img, (br.0, br.1), (tl.0, br.1), w, px);
                    stamp_segment(img, (tl.0, br.1), (tl.0, tl.1), w, px);
                }
            }
        }
        Tool::Text => {
            if let (Some(text), Some(p)) = (&action.text, action.points.first()) {
                draw_text(
                    img,
                    (p.0, p.1 - action.font_size),
                    action.font_size,
                    text,
                    px,
                );
            }
        }
        Tool::Blur => {}
        Tool::Counter => {
            // Filled disc at the point, then the step number on top.
            if let Some(p) = action.points.first() {
                let r = action.font_size * 0.9;
                stamp(img, p.0, p.1, r, px);
                let white = image::Rgba([255, 255, 255, px.0[3]]);
                let num = &action.step_label;
                draw_text(
                    img,
                    counter_label_at(*p, action.font_size, num),
                    action.font_size,
                    num,
                    white,
                );
            }
        }
        Tool::Select | Tool::Crop => {}
    }
}

/// The height of a line of annotation text over its em. The canvas and
/// the saved file both set a line in a box this tall and center the
/// face's ascent and descent in it, as GPUI lays out a line, so the
/// baseline lands on the same pixel in both.
pub(crate) const TEXT_LINE: f32 = 1.25;

/// The face annotations are set in: the bundled Inter Bold, parsed once
/// per process. Embedded, so text bakes identically on every platform
/// with no system font dependency.
fn face() -> &'static ab_glyph::FontRef<'static> {
    static FACE: std::sync::LazyLock<ab_glyph::FontRef<'static>> = std::sync::LazyLock::new(|| {
        ab_glyph::FontRef::try_from_slice(crate::theme::FONT_BOLD_TTF)
            .expect("bundled Inter Bold parses")
    });
    &FACE
}

/// The face scaled so its em is `size` pixels, as GPUI scales a font to
/// its text size. A `PxScale` of `size` would make ascent minus descent
/// `size` instead, about a sixth smaller for Inter.
fn scaled(size: f32) -> ab_glyph::PxScaleFont<&'static ab_glyph::FontRef<'static>> {
    use ab_glyph::Font as _;
    let font = face();
    let em = font.units_per_em().unwrap_or(font.height_unscaled());
    font.into_scaled(size * font.height_unscaled() / em)
}

/// The horizontal advance of `text` at `size`, in pixels.
pub(crate) fn text_advance(text: &str, size: f32) -> f32 {
    use ab_glyph::ScaleFont as _;
    let font = scaled(size);
    let mut w = 0f32;
    let mut last: Option<ab_glyph::GlyphId> = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(last) = last {
            w += font.kern(last, id);
        }
        w += font.h_advance(id);
        last = Some(id);
    }
    w
}

/// The top-left of a counter's label line box: centered on the disc's
/// center `p` in both axes.
pub(crate) fn counter_label_at(p: (f32, f32), size: f32, label: &str) -> (f32, f32) {
    (
        p.0 - text_advance(label, size) / 2.0,
        p.1 - TEXT_LINE * size / 2.0,
    )
}

/// The face's ascent-to-descent span in a line box at `size`: its offset
/// from the box's top and its height. A line's glyphs and the text
/// entry's caret both occupy it.
pub(crate) fn glyph_span(size: f32) -> (f32, f32) {
    use ab_glyph::ScaleFont as _;
    let font = scaled(size);
    let height = font.ascent() - font.descent();
    ((TEXT_LINE * size - height) / 2.0, height)
}

/// Set `text` at `size` in a line box whose top-left is `at`, blending
/// `px` over every covered pixel by coverage.
pub(crate) fn draw_text(
    img: &mut image::RgbaImage,
    at: (f32, f32),
    size: f32,
    text: &str,
    px: image::Rgba<u8>,
) {
    // Inline of imageproc's draw_text_mut so the imageproc crate (and
    // its nalgebra/rayon/rand tree) drops out of the build: rasterize
    // each outlined glyph and blend every covered pixel by coverage over
    // all four channels. Glyphs sit at their exact fractional position,
    // as GPUI places them, rather than at a whole-pixel origin.
    use ab_glyph::ScaleFont as _;
    let font = scaled(size);
    let baseline = at.1 + glyph_span(size).0 + font.ascent();
    let iw = img.width() as i32;
    let ih = img.height() as i32;
    let mut pen = at.0;
    let mut last: Option<ab_glyph::GlyphId> = None;
    for c in text.chars() {
        let id = font.glyph_id(c);
        if let Some(last) = last {
            pen += font.kern(last, id);
        }
        let glyph = id.with_scale_and_position(font.scale(), ab_glyph::point(pen, baseline));
        pen += font.h_advance(id);
        last = Some(id);
        let Some(g) = font.outline_glyph(glyph) else {
            continue;
        };
        // `px_bounds` is whole pixels; coverage is relative to its min.
        let bb = g.px_bounds();
        g.draw(|gx, gy, gv| {
            let image_x = gx as i32 + bb.min.x as i32;
            let image_y = gy as i32 + bb.min.y as i32;
            let gv = gv.clamp(0.0, 1.0);
            if (0..iw).contains(&image_x) && (0..ih).contains(&image_y) {
                let dst = img.get_pixel_mut(image_x as u32, image_y as u32);
                let inv = 1.0 - gv;
                for ch in 0..4 {
                    dst[ch] = (dst[ch] as f32 * inv + px[ch] as f32 * gv).clamp(0.0, 255.0) as u8;
                }
            }
        });
    }
}
