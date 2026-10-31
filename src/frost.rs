//! Frosted glass from a frozen frame: the pixels under a surface,
//! averaged into cells and blurred, for the surface to draw under its
//! tint. A surface over a still image needs no compositor blur: the
//! image under it is known.

use std::ops::Range;

/// A frosted copy of `rect` (x, y, width, height in pixels) of a
/// `width`x`height` frame of 4-byte pixels. Each `step`x`step` block
/// averages into one cell, and three box passes of `radius` cells each
/// way blur the cells toward a Gaussian. Cells beside the rect draw on
/// the frame beyond it, repeated at the frame's edge, so the glass
/// keeps its brightness up to its border. The result is one pixel per
/// cell, `ceil(w / step)` by `ceil(h / step)`, in the frame's own byte
/// order. None for an empty frame or rect, a zero `step`, or a buffer
/// shorter than the frame.
pub fn frost(
    frame: &[u8],
    width: u32,
    height: u32,
    rect: (u32, u32, u32, u32),
    step: u32,
    radius: u32,
) -> Option<image::RgbaImage> {
    let (x, y, w, h) = rect;
    if width == 0 || height == 0 || w == 0 || h == 0 || step == 0 {
        return None;
    }
    let stride = width as usize * 4;
    if frame.len() < stride * height as usize {
        return None;
    }
    let (cw, ch) = (w.div_ceil(step) as usize, h.div_ceil(step) as usize);
    // Three passes each reach `radius` cells: the cells that far past
    // the rect feed the ones on its edge.
    let pad = 3 * radius as usize;
    let (gw, gh) = (cw + 2 * pad, ch + 2 * pad);
    let origin = |at: u32, cell: usize| at as i64 + (cell as i64 - pad as i64) * step as i64;
    let cols: Vec<Range<usize>> = (0..gw).map(|gx| span(origin(x, gx), step, width)).collect();
    let mut grid = vec![[0.0f32; 4]; gw * gh];
    for gy in 0..gh {
        let rows = span(origin(y, gy), step, height);
        for (gx, cols) in cols.iter().enumerate() {
            let mut sum = [0u64; 4];
            for row in rows.clone() {
                let line = &frame[row * stride + cols.start * 4..row * stride + cols.end * 4];
                for px in line.as_chunks::<4>().0 {
                    for (s, v) in sum.iter_mut().zip(px) {
                        *s += u64::from(*v);
                    }
                }
            }
            let n = (rows.len() * cols.len()) as f32;
            grid[gy * gw + gx] = sum.map(|s| s as f32 / n);
        }
    }
    for _ in 0..3 {
        box_pass(&mut grid, gw, gh, radius as usize, true);
        box_pass(&mut grid, gw, gh, radius as usize, false);
    }
    let mut out = image::RgbaImage::new(cw as u32, ch as u32);
    for (cy, row) in out.rows_mut().enumerate() {
        for (cx, px) in row.enumerate() {
            let cell = grid[(cy + pad) * gw + cx + pad];
            px.0 = cell.map(|v| v.round().clamp(0.0, 255.0) as u8);
        }
    }
    Some(out)
}

/// The pixels of a `step` long block starting at `start`, inside
/// `0..limit`; a block wholly outside takes the edge pixel nearest it.
fn span(start: i64, step: u32, limit: u32) -> Range<usize> {
    let limit = i64::from(limit);
    let lo = start.clamp(0, limit - 1);
    let hi = (start + i64::from(step)).clamp(lo + 1, limit);
    lo as usize..hi as usize
}

/// One box blur pass of `radius` along rows (`across`) or columns of a
/// `w`x`h` grid, repeating the edge cells past the grid's border.
fn box_pass(grid: &mut [[f32; 4]], w: usize, h: usize, radius: usize, across: bool) {
    if radius == 0 {
        return;
    }
    let (lines, len) = if across { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| {
        if across {
            line * w + i
        } else {
            i * w + line
        }
    };
    let n = (2 * radius + 1) as f32;
    let mut src = vec![[0.0f32; 4]; len];
    for line in 0..lines {
        for (i, cell) in src.iter_mut().enumerate() {
            *cell = grid[at(line, i)];
        }
        for i in 0..len {
            let mut sum = [0.0f32; 4];
            for k in i as i64 - radius as i64..=i as i64 + radius as i64 {
                let cell = src[k.clamp(0, len as i64 - 1) as usize];
                for (s, v) in sum.iter_mut().zip(cell) {
                    *s += v;
                }
            }
            grid[at(line, i)] = sum.map(|s| s / n);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    /// A `w`x`h` frame whose pixel at (x, y) is `f(x, y)`.
    fn frame(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| f(x, y))
            .collect()
    }

    /// WHY: the class closed is "the glass changes brightness toward its
    /// border": a blur that pads with black, or reads past the frame,
    /// darkens or garbles the bar's edge. A flat frame must frost to the
    /// same flat color for every rect, at the frame's corners too, and
    /// the channels must stay apart in their own order.
    #[test]
    fn a_flat_frame_frosts_to_its_own_color_up_to_every_edge() {
        let px = [200, 40, 90, 255];
        let f = frame(64, 48, |_, _| px);
        for rect in [
            (0, 0, 64, 48),
            (0, 0, 10, 7),
            (50, 40, 14, 8),
            (20, 10, 9, 9),
        ] {
            for (step, radius) in [(1, 0), (3, 2), (4, 3)] {
                let out = frost(&f, 64, 48, rect, step, radius).unwrap();
                let want = (rect.2.div_ceil(step), rect.3.div_ceil(step));
                assert_eq!(out.dimensions(), want, "{rect:?} step {step}");
                assert!(out.pixels().all(|p| p.0 == px), "{rect:?} {step} {radius}");
            }
        }
    }

    /// WHY: the class closed is "the glass shows the wrong part of the
    /// frame, or no blur": a cell must average its own block, and the
    /// blur must spread a hard edge into a ramp centered where the edge
    /// is, black on its left and white on its right.
    #[test]
    fn cells_average_their_block_and_the_blur_ramps_a_hard_edge() {
        // Black left of x = 32, white from it on.
        let f = frame(64, 8, |x, _| if x < 32 { [0; 4] } else { [255; 4] });
        // No blur: a cell is its block's mean, and the 2-pixel block
        // straddling the edge is half of each.
        let sharp = frost(&f, 64, 8, (29, 0, 6, 2), 2, 0).unwrap();
        let row: Vec<u8> = (0..3).map(|x| sharp.get_pixel(x, 0).0[0]).collect();
        assert_eq!(row, [0, 128, 255]);
        let flat = frost(&f, 64, 8, (0, 0, 64, 8), 4, 0).unwrap();
        let row: Vec<u8> = (0..16).map(|x| flat.get_pixel(x, 0).0[0]).collect();
        assert_eq!(&row[..8], &[0; 8]);
        assert_eq!(&row[8..], &[255; 8]);
        // Blurred: a monotone ramp, mid-gray at the edge, still black
        // and white far from it.
        let soft = frost(&f, 64, 8, (0, 0, 64, 8), 4, 2).unwrap();
        let row: Vec<u8> = (0..16).map(|x| soft.get_pixel(x, 0).0[0]).collect();
        assert!(row.windows(2).all(|p| p[0] <= p[1]), "{row:?}");
        assert!(
            row[7] > 60 && row[7] < 128 && row[8] > 128 && row[8] < 195,
            "{row:?}"
        );
        assert!(row[1] < 5 && row[14] > 250, "{row:?}");
        // A rect that starts at the edge blurs in the black beside it,
        // as the whole frame's blur does there: the glass shows what is
        // around the surface, not its own rect repeated.
        let beside = frost(&f, 64, 8, (32, 0, 32, 8), 4, 2).unwrap();
        assert_eq!(beside.get_pixel(0, 0), soft.get_pixel(8, 0));
    }

    #[test]
    fn a_frame_shorter_than_its_size_or_an_empty_rect_has_no_glass() {
        let f = frame(8, 8, |_, _| [9; 4]);
        assert!(frost(&f[..f.len() - 1], 8, 8, (0, 0, 4, 4), 2, 1).is_none());
        assert!(frost(&f, 8, 8, (0, 0, 0, 4), 2, 1).is_none());
        assert!(frost(&f, 8, 8, (0, 0, 4, 4), 0, 1).is_none());
        assert!(frost(&f, 0, 8, (0, 0, 4, 4), 2, 1).is_none());
    }
}
