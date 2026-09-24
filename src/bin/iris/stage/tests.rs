use std::{
    path::{Path, PathBuf},
    prelude::v1::test,
};

use iris_lib::config::{Config, ToastPosition};

use super::*;

/// Roots the config in a fresh tempdir: `store` otherwise rewrites the
/// real config. The dir must outlive the test's config reads.
fn isolated_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(iris_lib::dirs::HOME_ENV, dir.path());
    dir
}

/// A `w`x`h` capture of one color in `dir`, written once per size.
fn capture(dir: &Path, w: u32, h: u32, color: [u8; 4]) -> PathBuf {
    let path = dir.join(format!("cap-{w}x{h}.png"));
    if !path.exists() {
        image::RgbaImage::from_pixel(w, h, image::Rgba(color))
            .save(&path)
            .unwrap();
    }
    path
}

#[test]
#[serial_test::serial]
fn card_rest_rect_anchors_all_four_corners() {
    let _home = isolated_home();
    let at = |toast_position| {
        Config {
            toast_position,
            ..Config::default()
        }
        .store()
        .unwrap();
        card_rest_rect(1920.0, 1080.0, 400, 280)
    };

    let (x, y, w, h) = at(ToastPosition::BottomRight);
    assert_eq!((x, y), (1920.0 - MARGIN - w, 1080.0 - MARGIN - h));

    let (x, y, _w, h) = at(ToastPosition::BottomLeft);
    assert_eq!((x, y), (MARGIN, 1080.0 - MARGIN - h));

    let (x, y, w, _h) = at(ToastPosition::TopRight);
    assert_eq!((x, y), (1920.0 - MARGIN - w, MARGIN));

    let (x, y, _w, _h) = at(ToastPosition::TopLeft);
    assert_eq!((x, y), (MARGIN, MARGIN));
}

// WHY: the class closed here is "the toast paints pixels sized for a
// scale other than its display's". Pixels sized in logical units are
// upscaled on a HiDPI display, which blurs every card there. Each case
// pins the pixel size to the card's box in device pixels (never above
// the capture's own size) and the card to card_rest_rect's size, so
// the flight lands on the card exactly. Not covered: the re-scale a
// live window runs when it renders at another scale than the guess.
#[test]
#[serial_test::serial]
fn thumb_fills_the_cards_device_pixels() {
    let _home = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    // (capture size, display scale) -> (card size, pixel size)
    let cases = [
        ((1600, 900), 1.0, (200.0, 113.0), (200, 113)),
        ((1600, 900), 1.5, (200.0, 113.0), (300, 170)),
        ((1600, 900), 2.0, (200.0, 113.0), (400, 226)),
        ((700, 1400), 2.0, (70.0, 140.0), (140, 280)),
        ((300, 150), 1.0, (200.0, 100.0), (200, 100)),
        // The card's device box exceeds the capture: its own pixels.
        ((300, 150), 2.0, (200.0, 100.0), (300, 150)),
        ((120, 80), 2.0, (120.0, 80.0), (120, 80)),
    ];
    for ((iw, ih), scale, dims, px) in cases {
        let path = capture(dir.path(), iw, ih, [10, 20, 30, 255]);
        let thumb = prepare_thumb(&path, scale).unwrap();
        let case = format!("{iw}x{ih} at {scale}");
        assert_eq!(thumb.dims, dims, "card size, {case}");
        assert_eq!(thumb.px, px, "pixel size, {case}");
        assert_eq!(thumb.scale, scale, "{case}");
        assert_eq!(thumb.rgba.len(), (px.0 * px.1 * 4) as usize, "{case}");
        let size = thumb.render.size(0);
        assert_eq!((size.width.0 as u32, size.height.0 as u32), px, "{case}");
        let (_, _, w, h) = card_rest_rect(1920.0, 1080.0, iw, ih);
        assert_eq!((w, h), dims, "flight landing rect, {case}");
    }
}

#[test]
#[serial_test::serial]
fn prepare_thumb_reuses_stashed_decode() {
    // The toast must read finalize's stashed pixels, not re-decode
    // the PNG. Prove it by stashing a different color than the file
    // holds: the thumb must match the stash, not the disk bytes.
    let dir = tempfile::tempdir().unwrap();
    let img_path = capture(dir.path(), 8, 8, [255, 0, 0, 255]);
    let stashed = std::sync::Arc::new(image::RgbaImage::from_pixel(
        8,
        8,
        image::Rgba([0, 255, 0, 255]),
    ));
    crate::pipeline::stash_decoded(img_path.clone(), stashed);
    let thumb = prepare_thumb(&img_path, 1.0).unwrap();
    // First pixel green proves the stash fed the thumb, not the
    // red PNG on disk.
    assert_eq!(&thumb.rgba[..4], &[0, 255, 0, 255]);
}
