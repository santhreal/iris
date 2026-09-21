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
pub(crate) fn pixelated_patch_rgba(
    img: &image::RgbaImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Result<image::RgbaImage, String> {
    if w == 0 || h == 0 {
        return Err("empty blur region".to_string());
    }
    // crop_imm returns a SubImage view; resize accepts any
    // GenericImageView, so materializing it with to_image() was an
    // O(region) copy the resample never needed.
    let sub = image::imageops::crop_imm(img, x, y, w, h);
    let small = image::imageops::resize(
        &*sub,
        (w / BLUR_BLOCK).max(1),
        (h / BLUR_BLOCK).max(1),
        image::imageops::FilterType::Triangle,
    );
    Ok(image::imageops::resize(
        &small,
        w,
        h,
        image::imageops::FilterType::Nearest,
    ))
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
                draw_text(img, *p, action.font_size, text, px);
            }
        }
        Tool::Blur => {}
        Tool::Counter => {
            // Filled disc at the point, then the step number on top.
            if let Some(p) = action.points.first() {
                let r = action.font_size * 0.9;
                stamp(img, p.0, p.1, r, px);
                let num = &action.step_label;
                let nw = num.chars().count() as f32 * action.font_size * 0.6;
                let white = image::Rgba([255, 255, 255, px.0[3]]);
                draw_text(
                    img,
                    (p.0 - nw / 2.0, p.1 + action.font_size * 0.35),
                    action.font_size,
                    num,
                    white,
                );
            }
        }
        Tool::Select | Tool::Crop => {}
    }
}

pub(crate) fn draw_text(
    img: &mut image::RgbaImage,
    p: (f32, f32),
    size: f32,
    text: &str,
    px: image::Rgba<u8>,
) {
    // The font is read and parsed once per process: rasterize replays
    // this for every text action on every rebuild, and a disk read +
    // font parse per replay is pure waste.
    static FONT: std::sync::LazyLock<Option<ab_glyph::FontVec>> = std::sync::LazyLock::new(|| {
        let data = std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf")
            .or_else(|_| std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"))
            .ok()?;
        ab_glyph::FontVec::try_from_vec(data).ok()
    });
    let Some(font) = FONT.as_ref() else {
        return;
    };
    // Inline of imageproc's draw_text_mut so the imageproc crate (and
    // its nalgebra/rayon/rand tree) drops out of the build. Same
    // semantics: advance by h_advance + kern, rasterize each outlined
    // glyph, and blend every covered pixel by coverage over all four
    // channels.
    use ab_glyph::{Font as _, ScaleFont as _};
    let scale = ab_glyph::PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let x = p.0 as i32;
    let y = (p.1 - size) as i32;
    let iw = img.width() as i32;
    let ih = img.height() as i32;
    let mut w = 0f32;
    let mut last: Option<ab_glyph::GlyphId> = None;
    for c in text.chars() {
        let glyph_id = scaled.glyph_id(c);
        let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(w, scaled.ascent()));
        w += scaled.h_advance(glyph_id);
        if let Some(g) = scaled.outline_glyph(glyph) {
            if let Some(last) = last {
                w += scaled.kern(glyph_id, last);
            }
            last = Some(glyph_id);
            let bb = g.px_bounds();
            g.draw(|gx, gy, gv| {
                let image_x = gx as i32 + x + bb.min.x.round() as i32;
                let image_y = gy as i32 + y + bb.min.y.round() as i32;
                let gv = gv.clamp(0.0, 1.0);
                if (0..iw).contains(&image_x) && (0..ih).contains(&image_y) {
                    let dst = img.get_pixel_mut(image_x as u32, image_y as u32);
                    let inv = 1.0 - gv;
                    for ch in 0..4 {
                        dst[ch] =
                            (dst[ch] as f32 * inv + px[ch] as f32 * gv).clamp(0.0, 255.0) as u8;
                    }
                }
            });
        }
    }
}
