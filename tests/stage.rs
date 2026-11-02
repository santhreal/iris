//! Where and how large the editor draws the capture, read back from the
//! screen.
//!
//! WHY: the classes closed here:
//!
//! - "the editor shows the capture anywhere but the rect it maps image
//!   points to". The stage's hairline was a border of the element that
//!   clips the image, so the image was laid out inside it: a pixel in
//!   from each side of that rect and two pixels short of its size, while
//!   marks, the crop, and the pointer used the whole rect. The image's
//!   corners were rounded at rest too, hiding corner pixels the saved
//!   file keeps. A capture larger than the stage, fit at a scale that is
//!   not a whole number, shows its colour over every pixel wholly inside
//!   the rect, the four corner pixels among them, and over no pixel
//!   wholly outside it.
//! - "a small capture opens magnified". The fit grew a capture smaller
//!   than the stage up to four times, filtered soft. A capture smaller
//!   than the stage opens at one screen pixel per image pixel.
//!
//! Not covered: a zoomed or panned image, whose rect moves by the same
//! view; the morph from the toast, whose rect `tests/morph.rs` does not
//! read; a scale factor other than 1, which Xvfb does not have; Wayland,
//! Windows, and macOS, which share the stage's layout. The cases run
//! only with `IRIS_X11_TEST_DISPLAY` set and need Xvfb; without them
//! they print that they did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::ffi::OsStr;

use x11rb::connection::Connection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{client, extent, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The editor's title on `plain.png`.
const EDITOR: &str = "plain.png - iris";
/// The image's color, [r, g, b].
const IMAGE: [u8; 3] = [60, 140, 200];
/// The editor's toolbar and tool options bar, which the stage sits
/// under (`theme::TOOLBAR_H` plus `OPTIONS_H`), and the room the fit
/// keeps clear on each side of the image (`STAGE_MARGIN`).
const STAGE_TOP: f32 = 52.0 + 40.0;
const MARGIN: f32 = 24.0;

/// The rect, left, top, right, and bottom, of a `iw` x `ih` image in a
/// `w` x `h` client area, as the editor fits it at rest on a screen of
/// scale 1: inside the margins and at most one pixel per image pixel.
fn fit((w, h): (usize, usize), (iw, ih): (u32, u32)) -> (f32, f32, f32, f32) {
    let (w, h, iw, ih) = (w as f32, h as f32, iw as f32, ih as f32);
    let scale = ((w - 2.0 * MARGIN) / iw)
        .min((h - STAGE_TOP - 2.0 * MARGIN) / ih)
        .min(1.0);
    let (left, top) = (
        (w - iw * scale) / 2.0,
        STAGE_TOP + (h - STAGE_TOP - ih * scale) / 2.0,
    );
    (left, top, left + iw * scale, top + ih * scale)
}

/// The editor's client area, as `client` reads it, once it shows a
/// `size` image of one colour; `None` when the case does not run.
fn open(case: &str, size: (u32, u32)) -> Option<(usize, usize, Vec<[u8; 3]>)> {
    if !enabled() {
        return None;
    }
    let (_xvfb, display) = xvfb_sized(case, SCREEN)?;
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.png");
    image::RgbImage::from_pixel(size.0, size.1, image::Rgb(IMAGE))
        .save(&file)
        .unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&[OsStr::new("--annotate"), file.as_os_str()]);
    let editor = daemon.until("the editor to open", || {
        windows_of(&conn, root, EDITOR).first().copied()
    });
    daemon.until("the editor to draw the image", || {
        extent(&client(&conn, root, editor), IMAGE)
            .filter(|&(left, top, right, bottom)| right - left >= 60 && bottom - top >= 40)
    });
    daemon.settle();
    Some(client(&conn, root, editor))
}

#[test]
fn the_editor_shows_every_pixel_of_the_capture_over_its_rect() {
    // Larger than the stage on both axes, at a fit that is not a whole
    // number, so the rect's edges fall between pixels.
    let size = (1300, 900);
    let Some(shot) = open(
        "the_editor_shows_every_pixel_of_the_capture_over_its_rect",
        size,
    ) else {
        return;
    };
    let (w, h, px) = &shot;
    let (left, top, right, bottom) = fit((*w, *h), size);
    // The pixels wholly inside the rect, and those it touches at all.
    let inside = (
        left.ceil() as usize,
        top.ceil() as usize,
        right.floor() as usize - 1,
        bottom.floor() as usize - 1,
    );
    let touched = (
        left.floor() as usize,
        top.floor() as usize,
        right.ceil() as usize - 1,
        bottom.ceil() as usize - 1,
    );
    let shown = extent(&shot, IMAGE).expect("the image is on the stage");
    assert!(
        shown.0 <= inside.0 && shown.1 <= inside.1 && shown.2 >= inside.2 && shown.3 >= inside.3,
        "the image shows over {shown:?}, short of the pixels {inside:?} wholly inside its rect \
         {:?} in a {w}x{h} client area",
        (left, top, right, bottom)
    );
    assert!(
        shown.0 >= touched.0
            && shown.1 >= touched.1
            && shown.2 <= touched.2
            && shown.3 <= touched.3,
        "the image shows over {shown:?}, past the pixels {touched:?} its rect {:?} touches",
        (left, top, right, bottom)
    );
    let [r, g, b] = IMAGE;
    for (x, y) in [
        (inside.0, inside.1),
        (inside.2, inside.1),
        (inside.0, inside.3),
        (inside.2, inside.3),
    ] {
        let [pb, pg, pr] = px[y * w + x];
        assert_eq!(
            [pr, pg, pb],
            [r, g, b],
            "the image's corner pixel at ({x}, {y}) of its rect {:?} shows another color",
            (left, top, right, bottom)
        );
    }
}

#[test]
fn a_capture_smaller_than_the_stage_opens_at_its_own_pixels() {
    let size = (200, 120);
    let Some(shot) = open(
        "a_capture_smaller_than_the_stage_opens_at_its_own_pixels",
        size,
    ) else {
        return;
    };
    let (left, top, right, bottom) = extent(&shot, IMAGE).expect("the image is on the stage");
    let shown = (right + 1 - left, bottom + 1 - top);
    assert_eq!(
        shown,
        (size.0 as usize, size.1 as usize),
        "a {size:?} capture shows {shown:?} pixels, from ({left}, {top})"
    );
    let (want_left, want_top, ..) = fit((shot.0, shot.1), size);
    assert_eq!(
        (left, top),
        (want_left.ceil() as usize, want_top.ceil() as usize),
        "a {size:?} capture is not centered on the stage of a {}x{} client area",
        shot.0,
        shot.1
    );
}
