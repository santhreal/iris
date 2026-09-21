//! Regression tests for rasterization.

use std::rc::Rc;

use super::action::{Action, Tool};
use super::raster::rasterize;

fn action(tool: Tool, points: Vec<(f32, f32)>, filled: bool) -> Action {
    Action {
        tool,
        color: "#ff0000",
        width: 2.0,
        points: Rc::new(points),
        text: None,
        font_size: 16.0,
        filled,
        blur_patch: None,
        blur_rect: (0.0, 0.0, 0.0, 0.0),
        step: 0,
        step_label: "0".into(),
        bbox: None,
        cached_path: std::cell::RefCell::new(None),
    }
}

fn painted(img: &image::RgbaImage, x: u32, y: u32) -> bool {
    img.get_pixel(x, y).0[3] > 0
}

#[test]
fn filled_rect_paints_interior() {
    let mut img = image::RgbaImage::new(20, 20);
    rasterize(
        &mut img,
        &action(Tool::Rect, vec![(2.0, 2.0), (18.0, 18.0)], true),
        1.0,
    );
    assert!(painted(&img, 10, 10), "interior must be painted");
    assert!(painted(&img, 2, 2), "corner must be painted");
}

#[test]
fn outline_rect_leaves_interior_clear() {
    let mut img = image::RgbaImage::new(20, 20);
    rasterize(
        &mut img,
        &action(Tool::Rect, vec![(2.0, 2.0), (18.0, 18.0)], false),
        1.0,
    );
    assert!(!painted(&img, 10, 10), "interior must stay clear");
    assert!(painted(&img, 2, 10), "edge must be painted");
}

#[test]
fn line_stamps_both_endpoints() {
    let mut img = image::RgbaImage::new(30, 10);
    rasterize(
        &mut img,
        &action(Tool::Line, vec![(2.0, 5.0), (28.0, 5.0)], false),
        1.0,
    );
    assert!(painted(&img, 2, 5));
    assert!(painted(&img, 28, 5));
    assert!(painted(&img, 15, 5));
}

#[test]
fn text_action_paints_glyph_pixels() {
    // draw_text was reimplemented on ab_glyph directly (imageproc
    // dropped); a regression that silently renders nothing would
    // otherwise pass every test. Assert a Text action darkens
    // pixels inside its glyph box.
    let mut img = image::RgbaImage::new(80, 40);
    let mut a = action(Tool::Text, vec![(4.0, 30.0)], false);
    a.text = Some("Hi".into());
    a.font_size = 24.0;
    rasterize(&mut img, &a, 1.0);
    let painted = (0..80)
        .flat_map(|x| (0..40).map(move |y| (x, y)))
        .any(|(x, y)| img.get_pixel(x, y).0[3] > 0);
    assert!(painted, "text action must paint glyph pixels");
}

#[test]
fn arrow_paints_head_at_tip_not_tail() {
    let mut img = image::RgbaImage::new(60, 30);
    rasterize(
        &mut img,
        &action(Tool::Arrow, vec![(5.0, 15.0), (50.0, 15.0)], false),
        1.0,
    );
    // The head fans out behind the tip: its base sits ~14px back
    // from (50,15) and spans y 8..22 there, narrowing to the tip.
    assert!(painted(&img, 36, 8), "upper fan base must be painted");
    assert!(painted(&img, 36, 22), "lower fan base must be painted");
    assert!(painted(&img, 44, 15), "fan interior must be painted");
    assert!(painted(&img, 50, 15), "tip must be painted");
    assert!(!painted(&img, 10, 8), "tail must not fan out");
}

#[test]
fn filled_ellipse_paints_center_not_corners() {
    let mut img = image::RgbaImage::new(30, 30);
    rasterize(
        &mut img,
        &action(Tool::Ellipse, vec![(5.0, 5.0), (25.0, 25.0)], true),
        1.0,
    );
    assert!(painted(&img, 15, 15), "center must be painted");
    assert!(!painted(&img, 5, 5), "bounding corner must stay clear");
}

// WHY: the banded fill must not drop or shift a row at a band
// boundary. Below par_bands_mut's 1MB gate the fill runs inline;
// a 1024x512 image (2MB) splits into ~64-row bands, so a fill
// spanning several bands must paint every covered row including
// the boundary rows and leave the rest clear.
#[test]
fn banded_fill_covers_band_boundaries() {
    // A rect covering rows 100..=400 crosses band boundaries at
    // 128, 192, 256, 320, 384. Every interior pixel must be
    // painted and the 1px border around it clear: a band that
    // drops, shifts, or over-fills a row fails one of these.
    let mut img = image::RgbaImage::new(1024, 512);
    rasterize(
        &mut img,
        &action(Tool::Rect, vec![(100.0, 100.0), (900.0, 400.0)], true),
        1.0,
    );
    for y in 100..=400 {
        for x in 100..=900 {
            assert!(painted(&img, x, y), "interior pixel ({x},{y}) dropped");
        }
    }
    for x in 99..=901 {
        assert!(!painted(&img, x, 99), "top border ({x},99) painted");
        assert!(!painted(&img, x, 401), "bottom border ({x},401) painted");
    }
    for y in 99..=401 {
        assert!(!painted(&img, 99, y), "left border (99,{y}) painted");
        assert!(!painted(&img, 901, y), "right border (901,{y}) painted");
    }

    // A full-image fill must paint every pixel: any dropped band
    // shows as a clear row.
    let mut img = image::RgbaImage::new(1024, 512);
    rasterize(
        &mut img,
        &action(Tool::Rect, vec![(0.0, 0.0), (1023.0, 511.0)], true),
        1.0,
    );
    assert!(
        img.pixels().all(|p| p.0[3] > 0),
        "banded full fill dropped a row"
    );

    // A large filled ellipse: center painted, bounding corner clear.
    let mut img = image::RgbaImage::new(1024, 512);
    rasterize(
        &mut img,
        &action(Tool::Ellipse, vec![(200.0, 100.0), (800.0, 400.0)], true),
        1.0,
    );
    assert!(painted(&img, 500, 250), "ellipse center must be painted");
    assert!(
        painted(&img, 500, 128),
        "ellipse band-boundary row must be painted"
    );
    assert!(
        !painted(&img, 210, 110),
        "ellipse bounding corner must stay clear"
    );
}

#[test]
fn highlight_joint_blends_once() {
    // WHY: a translucent polyline must blend every covered pixel
    // exactly once. Adjacent capsules overlap at joints; two
    // blends compound alpha 0.35->~0.58 and the RGB toward the
    // stroke color, so a corner read as a darker dot in the saved
    // PNG while the canvas showed a uniform wash. The mask path
    // stamps coverage first, then blends once.
    let mut img = image::RgbaImage::new(40, 40);
    // L-shaped highlight: the joint at (10,10) is covered by both
    // segments, the arm at (5,10) by only the horizontal one.
    rasterize(
        &mut img,
        &action(
            Tool::Highlight,
            vec![(2.0, 10.0), (10.0, 10.0), (10.0, 30.0)],
            false,
        ),
        1.0,
    );
    let joint = img.get_pixel(10, 10).0;
    let arm = img.get_pixel(5, 10).0;
    // Single blend: rgb = 255*0.35 = 89, alpha = 89. A double
    // blend compounds to rgb ~147, alpha ~147: the joint must
    // match the arm, not exceed it.
    assert_eq!(
        joint, arm,
        "joint must blend exactly once: {joint:?} vs {arm:?}"
    );
    assert!(joint[0] < 110, "joint rgb {} shows compounding", joint[0]);
}

#[test]
fn pixelate_region_averages_cells_and_swizzles_bgra() {
    // WHY: the fused mosaic replaced crop+Triangle+Nearest+overlay.
    // The contract: every output pixel is its cell's box average, the
    // composite region and the returned BGRA tile carry the same
    // pixels, and pixels outside the region are untouched.
    use super::raster::pixelate_region_bgra;
    // 48x24 image: left half red, right half blue, alpha 255.
    let mut img = image::RgbaImage::from_fn(48, 24, |x, _| {
        if x < 24 {
            image::Rgba([200, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 200, 255])
        }
    });
    // Region (0,0,48,24) -> cells 4x2 of 12x12. Cells 0-1 red, 2-3 blue.
    let bgra = pixelate_region_bgra(&mut img, 0, 0, 48, 24);
    assert_eq!(bgra.len(), 48 * 24 * 4);
    // Cell interior: (6,6) is cell (0,0) = red.
    assert_eq!(img.get_pixel(6, 6).0, [200, 0, 0, 255]);
    // BGRA tile at same pixel: B and R swapped.
    let i = (6 * 48 + 6) * 4;
    assert_eq!(&bgra[i..i + 4], &[0, 0, 200, 255]);
    // (30,6) is cell (2,0) = blue.
    assert_eq!(img.get_pixel(30, 6).0, [0, 0, 200, 255]);
    let i = (6 * 48 + 30) * 4;
    assert_eq!(&bgra[i..i + 4], &[200, 0, 0, 255]);
}

#[test]
fn pixelate_region_respects_offsets_and_leaves_outside() {
    use super::raster::pixelate_region_bgra;
    let mut img = image::RgbaImage::from_fn(32, 32, |x, y| {
        image::Rgba([(x * 8) as u8, (y * 8) as u8, 0, 255])
    });
    let before = img.clone();
    // Region (8,8,16,16): one 16x16 cell (16/12 -> sw=sh=1).
    let bgra = pixelate_region_bgra(&mut img, 8, 8, 16, 16);
    // The single cell averages x in 8..24, y in 8..24:
    // mean of (x*8) over 8..24 = 8*15.5 = 124; same for y.
    assert_eq!(img.get_pixel(12, 12).0, [124, 124, 0, 255]);
    assert_eq!(img.get_pixel(23, 23).0, [124, 124, 0, 255]);
    // Outside the region is untouched.
    assert_eq!(img.get_pixel(4, 4), before.get_pixel(4, 4));
    assert_eq!(img.get_pixel(28, 28), before.get_pixel(28, 28));
    // Tile is the region only: 16x16 BGRA of the averaged cell.
    assert_eq!(bgra.len(), 16 * 16 * 4);
    assert_eq!(&bgra[0..4], &[0, 124, 124, 255]);
}
