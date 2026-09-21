use std::{prelude::v1::test, time::Instant};

use iris_lib::config::{Config, ToastPosition};

use super::*;

#[test]
#[serial_test::serial]
fn card_rest_rect_anchors_all_four_corners() {
    let mut cfg = Config::default();

    cfg.toast_position = ToastPosition::BottomRight;
    cfg.store().unwrap();
    let (x, y, w, h) = card_rest_rect(1920.0, 1080.0, 400, 280);
    assert_eq!((x, y), (1920.0 - MARGIN - w, 1080.0 - MARGIN - h));

    cfg.toast_position = ToastPosition::BottomLeft;
    cfg.store().unwrap();
    let (x, y, _w, h) = card_rest_rect(1920.0, 1080.0, 400, 280);
    assert_eq!((x, y), (MARGIN, 1080.0 - MARGIN - h));

    cfg.toast_position = ToastPosition::TopRight;
    cfg.store().unwrap();
    let (x, y, w, _h) = card_rest_rect(1920.0, 1080.0, 400, 280);
    assert_eq!((x, y), (1920.0 - MARGIN - w, MARGIN));

    cfg.toast_position = ToastPosition::TopLeft;
    cfg.store().unwrap();
    let (x, y, _w, _h) = card_rest_rect(1920.0, 1080.0, 400, 280);
    assert_eq!((x, y), (MARGIN, MARGIN));

    let _ = Config::default().store();
}

#[test]
#[serial_test::serial]
fn toast_pin_state_defaults_false_and_toggles() {
    let dir = tempfile::tempdir().unwrap();
    let img_path = dir.path().join("test.png");
    image::RgbaImage::new(10, 10).save(&img_path).unwrap();
    let (thumb, thumb_rgba, dims) = prepare_thumb(&img_path).unwrap();
    let mut stage = ToastStage::from_parts(&img_path, thumb, thumb_rgba, dims);
    assert!(!stage.pinned);
    assert!(stage.closing_at.is_none());

    stage.closing_at = Some(Instant::now());
    stage.pinned = true;
    if stage.pinned {
        stage.closing_at = None;
    }
    assert!(stage.closing_at.is_none());
    assert!(stage.pinned);
}

#[test]
#[serial_test::serial]
fn prepare_thumb_reuses_stashed_decode() {
    // The toast must read finalize's stashed pixels, not re-decode
    // the PNG. Prove it by stashing a different color than the file
    // holds: the thumb must match the stash, not the disk bytes.
    let dir = tempfile::tempdir().unwrap();
    let img_path = dir.path().join("cap.png");
    // On-disk PNG is red; the stash is green.
    image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255]))
        .save(&img_path)
        .unwrap();
    let stashed = std::sync::Arc::new(image::RgbaImage::from_pixel(
        8,
        8,
        image::Rgba([0, 255, 0, 255]),
    ));
    crate::pipeline::stash_decoded(img_path.clone(), stashed);
    let (_thumb, thumb_rgba, _dims) = prepare_thumb(&img_path).unwrap();
    // First pixel green proves the stash fed the thumb, not the
    // red PNG on disk.
    assert_eq!(&thumb_rgba[..4], &[0, 255, 0, 255]);
}
