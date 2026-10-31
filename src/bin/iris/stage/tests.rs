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
        card_rest_rect(1920.0, 1080.0, 400, 280).0
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
// pins the pixel size to the capture's box on the card in device pixels
// (never above the capture's own size), and the card and that box to
// card_rest_rect's, so the flight lands on the card exactly. Not
// covered: the re-scale a live window runs when it renders at another
// scale than the guess.
#[test]
#[serial_test::serial]
fn thumb_fills_the_cards_device_pixels() {
    let _home = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    // (capture size, display scale) -> (card size, capture's rect on
    // the card, pixel size)
    let cases = [
        (
            (1600, 900),
            1.0,
            (200.0, 113.0),
            (0.0, 0.0, 200.0, 113.0),
            (200, 113),
        ),
        (
            (1600, 900),
            1.5,
            (200.0, 113.0),
            (0.0, 0.0, 200.0, 113.0),
            (300, 170),
        ),
        (
            (1600, 900),
            2.0,
            (200.0, 113.0),
            (0.0, 0.0, 200.0, 113.0),
            (400, 226),
        ),
        (
            (700, 1400),
            2.0,
            (154.0, 140.0),
            (42.0, 0.0, 70.0, 140.0),
            (140, 280),
        ),
        (
            (300, 150),
            1.0,
            (200.0, 100.0),
            (0.0, 0.0, 200.0, 100.0),
            (200, 100),
        ),
        // The capture's device box exceeds the capture: its own pixels.
        (
            (300, 150),
            2.0,
            (200.0, 100.0),
            (0.0, 0.0, 200.0, 100.0),
            (300, 150),
        ),
        (
            (120, 80),
            2.0,
            (154.0, 88.0),
            (17.0, 4.0, 120.0, 80.0),
            (120, 80),
        ),
        (
            (40, 30),
            1.0,
            (154.0, 88.0),
            (57.0, 29.0, 40.0, 30.0),
            (40, 30),
        ),
        (
            (1920, 120),
            1.0,
            (200.0, 88.0),
            (0.0, 38.0, 200.0, 13.0),
            (200, 13),
        ),
    ];
    for ((iw, ih), scale, card, image, px) in cases {
        let path = capture(dir.path(), iw, ih, [10, 20, 30, 255]);
        let thumb = prepare_thumb(&path, scale).unwrap();
        let case = format!("{iw}x{ih} at {scale}");
        assert_eq!((thumb.card.w, thumb.card.h), card, "card size, {case}");
        assert_eq!(
            thumb.card.image, image,
            "capture's rect on the card, {case}"
        );
        assert_eq!(thumb.px, px, "pixel size, {case}");
        assert_eq!(thumb.scale, scale, "{case}");
        assert_eq!(thumb.rgba.len(), (px.0 * px.1 * 4) as usize, "{case}");
        let size = thumb.render.size(0);
        assert_eq!((size.width.0 as u32, size.height.0 as u32), px, "{case}");
        let ((_, _, w, h), rest) = card_rest_rect(1920.0, 1080.0, iw, ih);
        assert_eq!((w, h), card, "flight landing rect, {case}");
        assert_eq!(rest, thumb.card, "flight landing card, {case}");
    }
}

// WHY: the class closed here is "a hover action cut off by its own
// card, or drawn over another". The card clips what it holds, so the
// actions of a square, tall, or short capture lost their end buttons
// or their top. Every capture size the card can show must give a card
// that holds every action of ToastAction::ALL inside the inset, in
// whole pixels, no two of them overlapping, and the capture centered
// on it, aspect-true, at the size it had before, in whole pixels. A new
// action is checked with the rest. Not covered: the actions' paint, and
// a label wider than its button.
#[test]
fn every_card_holds_the_hover_actions_apart_and_centers_its_capture() {
    use actions::{ToastAction, ACTIONS_MIN, SLOT_INSET};
    assert_eq!(ACTIONS_MIN, (154.0, 88.0));
    let sizes = (1..=40u32)
        .chain((41..=4000).step_by(37))
        .chain([7680, 20000]);
    for iw in sizes.clone() {
        for ih in sizes.clone() {
            let card = Card::of(iw as f32, ih as f32);
            let case = format!("{iw}x{ih}: {card:?}");
            let rects = ToastAction::ALL.map(|a| (a, a.rect(card.w, card.h)));
            for (a, (x, y, w, h)) in rects {
                assert!(
                    x >= SLOT_INSET
                        && y >= SLOT_INSET
                        && x + w <= card.w - SLOT_INSET
                        && y + h <= card.h - SLOT_INSET,
                    "{a:?} at {:?} leaves the card's inset, {case}",
                    (x, y, w, h)
                );
                assert_eq!((x.fract(), y.fract()), (0.0, 0.0), "{a:?}, {case}");
            }
            for (i, (a, (ax, ay, aw, ah))) in rects.iter().enumerate() {
                for (b, (bx, by, bw, bh)) in &rects[i + 1..] {
                    assert!(
                        ax + aw <= *bx || bx + bw <= *ax || ay + ah <= *by || by + bh <= *ay,
                        "{a:?} overlaps {b:?}, {case}"
                    );
                }
            }
            let (x, y, w, h) = card.image;
            let fit = (MAX_W / iw as f32).min(MAX_H / ih as f32).min(1.0);
            assert_eq!(
                (w, h),
                (
                    (iw as f32 * fit).round().max(1.0),
                    (ih as f32 * fit).round().max(1.0)
                ),
                "the capture's size, {case}"
            );
            assert!(
                (x - (card.w - w) / 2.0).abs() <= 0.5 && (y - (card.h - h) / 2.0).abs() <= 0.5,
                "the capture is off center, {case}"
            );
            assert_eq!((x.fract(), y.fract()), (0.0, 0.0), "{case}");
            assert_eq!(
                (card.w, card.h),
                (w.max(ACTIONS_MIN.0), h.max(ACTIONS_MIN.1)),
                "the card is larger than it needs, {case}"
            );
            assert_eq!(card.letterboxed(), (w, h) != (card.w, card.h), "{case}");
        }
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

// WHY: the class closed here is "the hover actions jump or stick". A
// fade that restarts from 0 or 1 at every edge crossing flickers when the
// pointer grazes the card, and one that ignores the pointer's side leaves
// the actions over the capture after the pointer leaves. Each case pins
// the start, the end, the clamp past the end, and a reversal in mid-fade
// for both sides. Not covered: the frame scheduling that advances `t`.
#[test]
fn hover_reveal_eases_from_the_crossing_toward_the_pointer_side() {
    let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
    for hovered in [false, true] {
        let to = if hovered { 1.0 } else { 0.0 };
        for from in [0.0, 0.4, 1.0] {
            assert!(close(hover_reveal(from, hovered, 0.0), from));
            assert!(close(hover_reveal(from, hovered, 1.0), to));
            assert!(
                close(hover_reveal(from, hovered, 7.0), to),
                "t past the fade clamps"
            );
            let mid = hover_reveal(from, hovered, 0.5);
            assert!(mid >= from.min(to) - 1e-6 && mid <= from.max(to) + 1e-6);
        }
    }
    // Left at 40%: the fade continues down from 0.4, and re-entering at
    // 40% continues up from it.
    assert!(hover_reveal(0.4, false, 0.25) < 0.4);
    assert!(hover_reveal(0.4, true, 0.25) > 0.4);
}

// WHY: the class closed here is "a status that buries the actions, or
// actions that bury a status before it is read". The status holds the
// slot off the pointer, while an action runs, and for STATUS_HOLD after
// it was set; past the hold, the pointer gets the actions back. Each
// branch and the hold's boundary is a case. Not covered: the timer that
// repaints at the hold's end.
#[test]
fn status_holds_the_slot_off_pointer_while_busy_and_through_the_hold() {
    let fresh = Duration::ZERO;
    let held = STATUS_HOLD - Duration::from_millis(1);
    let stale = STATUS_HOLD + Duration::from_millis(1);
    // No status: the slot is the actions', busy or not.
    assert!(!status_holds_slot(false, false, false, fresh));
    assert!(!status_holds_slot(false, true, true, fresh));
    // Off the pointer the status shows, however old.
    assert!(status_holds_slot(true, false, false, stale));
    // Under the pointer: through the hold, then the actions.
    assert!(status_holds_slot(true, true, false, fresh));
    assert!(status_holds_slot(true, true, false, held));
    assert!(!status_holds_slot(true, true, false, STATUS_HOLD));
    assert!(!status_holds_slot(true, true, false, stale));
    // A running action (OCR) keeps its "Reading text" line up.
    assert!(status_holds_slot(true, true, true, stale));
}

// WHY: the class closed here is "the toast's context menu cut off by
// its own window". The window was the card plus its bleed and margin,
// so the card of a wide capture (a short card) or a tall one (a narrow
// card) had a window smaller than the menu, and the menu opened past
// its edge. Every card size the toast can have and a right click at
// each of the card's corners must place the whole menu inside the
// window. A card whose window already holds the menu keeps that window,
// so the transparent area around a common capture's card does not
// grow. Not covered: the menu's paint, and a toast window the platform
// sizes differently than asked.
#[test]
fn the_toast_menu_fits_its_window_for_every_card_size() {
    use crate::widgets::MENU_W;
    use actions::{menu_place, MENU_EDGE, MENU_H};
    for w in (1..=MAX_W as u32).step_by(3) {
        for h in (1..=MAX_H as u32).step_by(3) {
            let card = (w as f32, h as f32);
            let win = window::window_size(card);
            // The card rests in the window's bottom-right corner; the
            // other corners mirror it.
            let (x0, y0) = (win.0 - MARGIN - card.0, win.1 - MARGIN - card.1);
            for at in [
                (x0, y0),
                (x0 + card.0, y0),
                (x0, y0 + card.1),
                (win.0 - MARGIN, win.1 - MARGIN),
            ] {
                let (mx, my) = menu_place(at, win);
                assert!(
                    mx >= MENU_EDGE
                        && my >= MENU_EDGE
                        && mx + MENU_W <= win.0 - MENU_EDGE
                        && my + MENU_H <= win.1 - MENU_EDGE,
                    "card {card:?}: a menu opened at {at:?} lands at ({mx}, {my}) in a {win:?} window"
                );
            }
        }
    }
    // A 16:9 capture's card already holds the menu.
    assert_eq!(window::window_size((200.0, 113.0)), (256.0, 169.0));
}
