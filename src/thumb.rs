//! Banded `image::imageops::thumbnail` for `RgbaImage`.
//!
//! The stock thumbnail is a single-threaded box-average scan: every
//! source pixel is read exactly once, so a 4K capture pays a 33MB
//! serial pass to produce a 216px card. Output rows are independent —
//! each reads only source pixels — so the row loop bands across cores.
//! The sampler arithmetic is copied verbatim from the image crate
//! (block, horizontal-fraction, vertical-fraction, both-fraction) so
//! the output is pixel-identical; the test pins that.

use crate::par::par_bands_mut_work;

/// Pixel-identical, banded `image::imageops::thumbnail` for RGBA8.
/// Bands split output rows; the size gate keys on the INPUT bytes
/// because the work is the source scan, not the small output.
pub fn thumbnail_rgba(
    img: &image::RgbaImage,
    new_width: u32,
    new_height: u32,
) -> image::RgbaImage {
    let (width, height) = img.dimensions();
    let mut out = image::RgbaImage::new(new_width, new_height);
    if width == 0 || height == 0 || new_width == 0 || new_height == 0 {
        return out;
    }
    let src = img.as_raw();
    let w = width as usize;
    let px = |x: u32, y: u32| -> [u8; 4] {
        let i = (y as usize * w + x as usize) * 4;
        [src[i], src[i + 1], src[i + 2], src[i + 3]]
    };
    let x_ratio = width as f32 / new_width as f32;
    let y_ratio = height as f32 / new_height as f32;
    let stride = new_width as usize * 4;
    par_bands_mut_work(
        out.as_mut(),
        stride,
        src.len(),
        |band, start| {
            let first = start / stride;
            for (row, out_row) in band.chunks_exact_mut(stride).enumerate() {
                let outy = first as u32 + row as u32;
                let bottomf = outy as f32 * y_ratio;
                let topf = bottomf + y_ratio;
                let bottom = (bottomf.ceil() as u32).min(height - 1);
                let top = (topf.ceil() as u32).clamp(bottom, height);
                for outx in 0..new_width {
                    let leftf = outx as f32 * x_ratio;
                    let rightf = leftf + x_ratio;
                    let left = (leftf.ceil() as u32).min(width - 1);
                    let right = (rightf.ceil() as u32).clamp(left, width);
                    let avg = if bottom != top && left != right {
                        sample_block(&px, left, right, bottom, top)
                    } else if bottom != top {
                        // left == right: interpolate the two columns
                        // flanking the empty window.
                        let frac = (leftf.fract() + rightf.fract()) / 2.;
                        sample_fraction_h(&px, right - 1, frac, bottom, top)
                    } else if left != right {
                        // bottom == top: interpolate the two rows
                        // flanking the empty window.
                        let frac = (topf.fract() + bottomf.fract()) / 2.;
                        sample_fraction_v(&px, left, right, top - 1, frac)
                    } else {
                        // Empty window both ways: bilinear over the
                        // four surrounding pixels.
                        let frac_h = (topf.fract() + bottomf.fract()) / 2.;
                        let frac_v = (leftf.fract() + rightf.fract()) / 2.;
                        sample_fraction_both(&px, right - 1, frac_h, top - 1, frac_v)
                    };
                    let i = outx as usize * 4;
                    out_row[i..i + 4].copy_from_slice(&avg);
                }
            }
        },
    );
    out
}

/// Whole-pixel window: straight box average with round-half-up.
fn sample_block(
    px: &impl Fn(u32, u32) -> [u8; 4],
    left: u32,
    right: u32,
    bottom: u32,
    top: u32,
) -> [u8; 4] {
    let mut sum = [0u32; 4];
    for y in bottom..top {
        for x in left..right {
            let p = px(x, y);
            for (s, &v) in sum.iter_mut().zip(p.iter()) {
                *s += v as u32;
            }
        }
    }
    let n = (right - left) * (top - bottom);
    let round = n / 2;
    [
        ((sum[0] + round) / n) as u8,
        ((sum[1] + round) / n) as u8,
        ((sum[2] + round) / n) as u8,
        ((sum[3] + round) / n) as u8,
    ]
}

/// Empty-width window: column averages of the two flanking columns,
/// mixed by the window's horizontal position.
fn sample_fraction_h(
    px: &impl Fn(u32, u32) -> [u8; 4],
    left: u32,
    fract: f32,
    bottom: u32,
    top: u32,
) -> [u8; 4] {
    let mut sum_left = [0u32; 4];
    let mut sum_right = [0u32; 4];
    for y in bottom..top {
        let l = px(left, y);
        let r = px(left + 1, y);
        for c in 0..4 {
            sum_left[c] += l[c] as u32;
            sum_right[c] += r[c] as u32;
        }
    }
    let fact_right = fract / (top - bottom) as f32;
    let fact_left = (1. - fract) / (top - bottom) as f32;
    let mix = |l: u32, r: u32| (fact_left * l as f32 + fact_right * r as f32) as u8;
    [
        mix(sum_left[0], sum_right[0]),
        mix(sum_left[1], sum_right[1]),
        mix(sum_left[2], sum_right[2]),
        mix(sum_left[3], sum_right[3]),
    ]
}

/// Empty-height window: row averages of the two flanking rows, mixed
/// by the window's vertical position.
fn sample_fraction_v(
    px: &impl Fn(u32, u32) -> [u8; 4],
    left: u32,
    right: u32,
    bottom: u32,
    fract: f32,
) -> [u8; 4] {
    let mut sum_bot = [0u32; 4];
    let mut sum_top = [0u32; 4];
    for x in left..right {
        let b = px(x, bottom);
        let t = px(x, bottom + 1);
        for c in 0..4 {
            sum_bot[c] += b[c] as u32;
            sum_top[c] += t[c] as u32;
        }
    }
    let fact_top = fract / (right - left) as f32;
    let fact_bot = (1. - fract) / (right - left) as f32;
    let mix = |b: u32, t: u32| (fact_bot * b as f32 + fact_top * t as f32) as u8;
    [
        mix(sum_bot[0], sum_top[0]),
        mix(sum_bot[1], sum_top[1]),
        mix(sum_bot[2], sum_top[2]),
        mix(sum_bot[3], sum_top[3]),
    ]
}

/// Window with no enclosed pixel: bilinear mix of the four pixels at
/// the window's corners.
fn sample_fraction_both(
    px: &impl Fn(u32, u32) -> [u8; 4],
    left: u32,
    frac_v: f32,
    bottom: u32,
    frac_h: f32,
) -> [u8; 4] {
    let k_bl = px(left, bottom);
    let k_tl = px(left, bottom + 1);
    let k_br = px(left + 1, bottom);
    let k_tr = px(left + 1, bottom + 1);
    let fact_tr = frac_v * frac_h;
    let fact_tl = frac_v * (1. - frac_h);
    let fact_br = (1. - frac_v) * frac_h;
    let fact_bl = (1. - frac_v) * (1. - frac_h);
    let mix = |c: usize| {
        (fact_br * k_br[c] as f32
            + fact_tr * k_tr[c] as f32
            + fact_bl * k_bl[c] as f32
            + fact_tl * k_tl[c] as f32) as u8
    };
    [mix(0), mix(1), mix(2), mix(3)]
}

// WHY: the class closed here is "the banded replica drifts from the
// stock sampler": the thumbnail is the library card and the toast
// image, so a divergence shows as a subtly different card for the
// same capture depending on which path produced it. The test pins
// byte equality against image::imageops::thumbnail across downscale,
// upscale, and fractional-window shapes. Not covered: non-RGBA pixel
// types (the replica is specialized to RGBA8 by signature).
#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic non-uniform pixels: gradients plus a hash so
    /// block averages and fraction mixes all get exercised.
    fn test_image(w: u32, h: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([
                ((x * 37 + y * 11) % 251) as u8,
                ((x * 13 + y * 29) % 241) as u8,
                ((x * 7 ^ y * 5) % 233) as u8,
                (128 + (x * 3 + y * 17) % 128) as u8,
            ])
        })
    }

    #[test]
    fn banded_thumbnail_matches_stock() {
        // (source w, h, thumb w, h): big downscale, mild downscale,
        // upscale both axes, upscale one axis, fractional windows,
        // and a 1px output.
        for (sw, sh, tw, th) in [
            (640u32, 480u32, 216u32, 162u32),
            (640, 480, 320, 240),
            (100, 80, 200, 160),
            (640, 40, 160, 80),
            (333, 257, 100, 77),
            (17, 13, 5, 5),
            (640, 480, 1, 1),
            (3, 3, 7, 7),
        ] {
            let img = test_image(sw, sh);
            let stock = image::imageops::thumbnail(&img, tw, th);
            let banded = thumbnail_rgba(&img, tw, th);
            assert_eq!(
                stock.as_raw(),
                banded.as_raw(),
                "diverged at {sw}x{sh} -> {tw}x{th}"
            );
        }
    }
}
