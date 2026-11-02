//! Where the editor draws the capture, read back from the screen.
//!
//! WHY: the class closed here is "the editor shows the capture anywhere
//! but the rect it maps image points to". The stage's hairline was a
//! border of the element that clips the image, so the image was laid
//! out inside it: a pixel in from each side of that rect and two pixels
//! short of its size, while marks, the crop, and the pointer used the
//! whole rect. The image's corners were rounded at rest too, hiding
//! corner pixels the saved file keeps. The case opens a one-colour
//! image, computes the rect the editor fits it to, and reads that the
//! colour covers every pixel wholly inside the rect, the four corner
//! pixels among them, and no pixel wholly outside it.
//!
//! Not covered: a zoomed or panned image, whose rect moves by the same
//! view; the morph from the toast, whose rect `tests/morph.rs` does not
//! read; Wayland, Windows, and macOS, which share the stage's layout.
//! The case runs only with `IRIS_X11_TEST_DISPLAY` set and needs Xvfb;
//! without them it prints that it did not run.

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
/// The image's color, [r, g, b], and its size: a fit that is not a
/// whole number, so the rect's edges fall between pixels.
const IMAGE: [u8; 3] = [60, 140, 200];
const IMAGE_W: u32 = 600;
const IMAGE_H: u32 = 400;
/// The editor's toolbar and tool options bar, which the stage sits
/// under (`theme::TOOLBAR_H` plus `OPTIONS_H`), and the room the fit
/// keeps clear on each side of the image (`STAGE_MARGIN`).
const STAGE_TOP: f32 = 52.0 + 40.0;
const MARGIN: f32 = 24.0;

/// The image's rect in the client area, as the editor fits it at rest:
/// left, top, right, bottom.
fn fit(w: usize, h: usize) -> (f32, f32, f32, f32) {
    let (w, h) = (w as f32, h as f32);
    let (iw, ih) = (IMAGE_W as f32, IMAGE_H as f32);
    let scale = ((w - 2.0 * MARGIN) / iw)
        .min((h - STAGE_TOP - 2.0 * MARGIN) / ih)
        .min(4.0);
    let (left, top) = (
        (w - iw * scale) / 2.0,
        STAGE_TOP + (h - STAGE_TOP - ih * scale) / 2.0,
    );
    (left, top, left + iw * scale, top + ih * scale)
}

#[test]
fn the_editor_shows_every_pixel_of_the_capture_over_its_rect() {
    let case = "the_editor_shows_every_pixel_of_the_capture_over_its_rect";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.png");
    image::RgbImage::from_pixel(IMAGE_W, IMAGE_H, image::Rgb(IMAGE))
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
            .filter(|&(left, top, right, bottom)| right - left >= 200 && bottom - top >= 100)
    });
    daemon.settle();

    let shot = client(&conn, root, editor);
    let (w, h, px) = &shot;
    let (left, top, right, bottom) = fit(*w, *h);
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
