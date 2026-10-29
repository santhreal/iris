//! Where the editor's zoom keeps the image still, read back from the
//! screen.
//!
//! WHY: the class closed here is "a zoom that moves the part of the
//! image the user is looking at". The wheel zooms about the pointer and
//! the `+` and `-` keys about the stage's center, so the image point
//! there stays put. The keys zoomed about the image's own center
//! instead, which after a pan or a wheel zoom lies elsewhere on the
//! stage, so each press slid the viewed part of the image away. The
//! case wheels in over a marker off the image's center, reads that the
//! marker stayed under the pointer, then presses `=` and reads that the
//! marker moved away from the stage's center by the key's step.
//!
//! Not covered: the touchpad's fractional scroll and the pinch, which
//! XTest cannot send; the unit tests in `editor/zoom_tests.rs` read
//! their factors. Windows and macOS, which share the zoom math. The case
//! runs only with `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without
//! them it prints that it did not run.

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
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{click_with, client, extent, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The editor's title on `plain.png`.
const EDITOR: &str = "plain.png - iris";
/// The image's color and its marker's, [r, g, b], far apart.
const IMAGE: [u8; 3] = [60, 140, 200];
const MARK: [u8; 3] = [250, 230, 40];
/// The image's size, and the marker's square and its top-left corner:
/// off the image's center on both axes.
const IMAGE_W: u32 = 600;
const IMAGE_H: u32 = 400;
const MARK_SIDE: u32 = 24;
const MARK_AT: (u32, u32) = (378, 118);
/// The wheel-up button.
const WHEEL_UP: u8 = 4;
/// The keysym of `=`, which zooms in by `KEY_STEP`.
const EQUAL: u32 = 0x3d;
const KEY_STEP: f32 = 1.25;
/// The editor's toolbar and tool options bar, which the stage sits
/// under: `theme::TOOLBAR_H` plus `OPTIONS_H`.
const STAGE_TOP: f32 = 52.0 + 40.0;
/// How far a measured center may read from the expected one: the
/// marker's scaled edges and the pointer's whole pixel.
const SLACK: f32 = 4.0;

/// The marker's center in the editor's client area, and its width.
fn marker(conn: &impl Connection, root: u32, editor: u32) -> Option<((f32, f32), f32)> {
    let (left, top, right, bottom) = extent(&client(conn, root, editor), MARK)?;
    let center = ((left + right) as f32 / 2.0, (top + bottom) as f32 / 2.0);
    Some((center, (right - left + 1) as f32))
}

fn far(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

#[test]
fn the_wheel_zooms_about_the_pointer_and_the_keys_about_the_stage_center() {
    let case = "the_wheel_zooms_about_the_pointer_and_the_keys_about_the_stage_center";
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
    let mut img = image::RgbImage::from_pixel(IMAGE_W, IMAGE_H, image::Rgb(IMAGE));
    for y in MARK_AT.1..MARK_AT.1 + MARK_SIDE {
        for x in MARK_AT.0..MARK_AT.0 + MARK_SIDE {
            img.put_pixel(x, y, image::Rgb(MARK));
        }
    }
    img.save(&file).unwrap();
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
    let (start, side) = marker(&conn, root, editor).expect("the marker is on the stage");
    let at = conn
        .translate_coordinates(editor, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let pointer = (
        at.dst_x + start.0.round() as i16,
        at.dst_y + start.1.round() as i16,
    );

    // Wheel in over the marker until it is twice its size.
    daemon.until("the wheel to double the marker", || {
        click_with(&conn, root, pointer, WHEEL_UP);
        std::thread::sleep(Duration::from_millis(60));
        let (_, width) = marker(&conn, root, editor)?;
        (width >= 2.0 * side).then_some(())
    });
    daemon.settle();
    let (wheeled, width) = marker(&conn, root, editor).expect("the marker stays on the stage");
    let pointed = (
        f32::from(pointer.0 - at.dst_x),
        f32::from(pointer.1 - at.dst_y),
    );
    assert!(
        far(wheeled, pointed) <= SLACK,
        "the wheel moved the marker from under the pointer at {pointed:?} to {wheeled:?}"
    );

    // One key step about the stage's center.
    tap(&conn, root, EQUAL);
    daemon.until("the key to zoom", || {
        let (_, w) = marker(&conn, root, editor)?;
        (w >= width * (KEY_STEP - 0.1)).then_some(())
    });
    daemon.settle();
    let (keyed, _) = marker(&conn, root, editor).expect("the marker stays on the stage");
    let (w, h, _) = client(&conn, root, editor);
    let stage = (w as f32 / 2.0, STAGE_TOP + (h as f32 - STAGE_TOP) / 2.0);
    let want = (
        stage.0 + KEY_STEP * (wheeled.0 - stage.0),
        stage.1 + KEY_STEP * (wheeled.1 - stage.1),
    );
    assert!(
        far(keyed, want) <= SLACK,
        "a key step about the stage's center {stage:?} moves the marker from {wheeled:?} \
         to {want:?}; it is at {keyed:?}"
    );
}
