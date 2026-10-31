//! The library's New Screenshot button and the overlay's opening
//! hover-snap, driven on a private Xvfb and read back from the screen.
//!
//! WHY: two classes close here. "A surface iris opens is part of the
//! picture it takes": the library's New Screenshot button ran the
//! capture with the library still on screen, so the frozen frame held
//! the library under the overlay. A compositing manager fades a closed
//! window out over its last frame, so the first case runs picom with a
//! two-second fade: the window must leave an empty frame behind it, not
//! its content. It paints the root white, clicks the button, and reads
//! the dimmed frozen frame where the window stood against the dimmed
//! root beside it. "The overlay ignores the pointer until it moves":
//! the overlay opened with no hover-snap and no readout until the first
//! motion, so a capture from a hotkey with the pointer resting on a
//! window showed that window dimmed. The second case rests the pointer
//! on a window, opens the overlay, and reads the window revealed
//! undimmed under the pick tint. It then commits a selection beside the
//! window and moves the pointer back over it: a press there clears the
//! selection and does not capture the window, so the window shows
//! dimmed, not picked, and a click captures nothing.
//!
//! Not covered: Wayland, where the pointer is unknown until it moves;
//! a window manager's frame or close animation; Windows and macOS; the
//! Record button, whose window pick reads the screen the same way
//! through the same close. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need Xvfb, and the first needs
//! picom; without them they print that they did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _, CreateWindowAux, WindowClass};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, fading_compositor, xvfb_sized, Daemon};
use x11::{click, client, paint_root, pixel, rect, windows_of};

const OVERLAY: &str = "Capture - iris";
const SCREEN: &str = "1280x1024x24";
const WHITE: u32 = 0x00ff_ffff;
/// The library's title.
const LIBRARY: &str = "Library - iris";
/// The library's New Screenshot button's centre from the window's top
/// right corner, at scale 1: the last of the toolbar's trailing
/// controls, 28px square, 12px left of the window controls in the 52px
/// band. With no desktop button layout to read, the window controls are
/// minimize, maximize, and close: three 24px buttons 8px apart, 12px
/// from the right edge.
const NEW_SCREENSHOT: (i16, i16) = (12 + 3 * 24 + 2 * 8 + 12 + 14, 26);
/// How far inside the library's left edge the frozen frame is read, and
/// twice how far outside it the root beside it is.
const INSET: i16 = 48;
/// The pointer's window: x, y, width, height, and its colour.
const TARGET: (i16, i16, u16, u16) = (200, 200, 320, 240);
const TARGET_RGB: u32 = 0x0033_66cc;
/// The window a click would capture, revealed undimmed under the
/// overlay's pick tint: systemBlue, #0A84FF, at 18% over `TARGET_RGB`.
const PICKED_RGB: u32 = 0x002c_6bd5;
/// `TARGET_RGB` under the overlay's dim: #1E1E1E at 32%.
const DIMMED_RGB: u32 = 0x002c_4f94;

/// The largest difference between two 0xRRGGBB colours in one channel.
fn channel_gap(a: u32, b: u32) -> u32 {
    [16, 8, 0]
        .into_iter()
        .map(|s| ((a >> s) & 0xff).abs_diff((b >> s) & 0xff))
        .max()
        .unwrap_or(0)
}

/// Wait until the overlay holds the focus and its frozen frame landed.
fn overlay_up(conn: &RustConnection, root: u32, daemon: &Daemon) {
    daemon.until("the overlay to take the focus", || {
        windows_of(conn, root, OVERLAY)
            .contains(&x11::focus(conn))
            .then_some(())
    });
    daemon.until("the frozen frame to land", || {
        daemon.log().contains("frame landed").then_some(())
    });
}

/// Where in the library window `press` clicks: x, y from the window's
/// top-left corner, given its width, height, and pixels.
type Press = fn(usize, usize, &[[u8; 3]]) -> (i16, i16);

/// Open an empty library over a white root under a fading compositor,
/// click where `press` says, and require the overlay's frozen frame to
/// show the root where the library stood.
fn leaves_the_library_out(case: &str, press: Press) {
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, WHITE);
    let Some(_picom) = fading_compositor(case, &conn, &display) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&["--library"]);
    let library = daemon.until("the library to open", || {
        windows_of(&conn, root, LIBRARY).first().copied()
    });
    daemon.settle();
    let (x, y, width, _) = rect(&conn, root, library);
    assert!(x >= INSET, "the library at x {x} leaves no root beside it");
    let inside = (x + INSET, y + 300);
    assert!(
        pixel(&conn, root, inside) != WHITE,
        "the library did not draw its body"
    );
    let (w, h, px) = client(&conn, root, library);
    let at = press(w, h, &px);
    click(&conn, root, (x + at.0, y + at.1));
    overlay_up(&conn, root, &daemon);
    assert!(
        windows_of(&conn, root, LIBRARY).is_empty(),
        "the library stayed open under the overlay"
    );
    let beside = (x - INSET / 2, y + 300);
    let far = (x + width as i16 + INSET / 2, y + 300);
    daemon.until(
        "the frozen frame to show the root where the library stood",
        || {
            let (b, i, f) = (
                pixel(&conn, root, beside),
                pixel(&conn, root, inside),
                pixel(&conn, root, far),
            );
            (b != WHITE && b == i && i == f).then_some(())
        },
    );
}

#[test]
fn new_screenshot_leaves_the_library_out_of_the_frozen_frame() {
    leaves_the_library_out(
        "new_screenshot_leaves_the_library_out_of_the_frozen_frame",
        |w, _, _| (w as i16 - NEW_SCREENSHOT.0, NEW_SCREENSHOT.1),
    );
}

/// The empty library's New Screenshot button is its one accent fill,
/// #0A84FF: the click lands on the centre of the accent pixels.
#[test]
fn the_empty_librarys_new_screenshot_leaves_it_out_of_the_frozen_frame() {
    leaves_the_library_out(
        "the_empty_librarys_new_screenshot_leaves_it_out_of_the_frozen_frame",
        |w, _, px| {
            let accent = |p: &[u8; 3]| p[2] < 30 && (120..=145).contains(&p[1]) && p[0] > 240;
            let hits: Vec<(usize, usize)> = px
                .iter()
                .enumerate()
                .filter(|(_, p)| accent(p))
                .map(|(i, _)| (i % w, i / w))
                .collect();
            assert!(
                hits.len() > 400,
                "the empty library shows no New Screenshot button: {} accent pixels",
                hits.len()
            );
            let n = hits.len();
            let (sx, sy) = hits
                .iter()
                .fold((0, 0), |(sx, sy), (x, y)| (sx + x, sy + y));
            ((sx / n) as i16, (sy / n) as i16)
        },
    );
}

#[test]
fn the_overlay_snaps_to_the_window_under_the_pointer_until_a_selection_is_committed() {
    let case = "the_overlay_snaps_to_the_window_under_the_pointer_until_a_selection_is_committed";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, WHITE);
    let target = conn.generate_id().unwrap();
    let (left, top, width, height) = TARGET;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        target,
        root,
        left,
        top,
        width,
        height,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new().background_pixel(TARGET_RGB),
    )
    .unwrap();
    conn.map_window(target).unwrap();
    // Rest the pointer on the window's centre before the overlay opens;
    // nothing moves it afterwards.
    let centre = (left + width as i16 / 2, top + height as i16 / 2);
    conn.xtest_fake_input(
        xproto::MOTION_NOTIFY_EVENT,
        0,
        x11rb::CURRENT_TIME,
        root,
        centre.0,
        centre.1,
        0,
    )
    .unwrap();
    conn.flush().unwrap();

    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&["--capture"]);
    overlay_up(&conn, root, &daemon);
    // A quarter in from the window's corner: off the pointer's row and
    // column. The root beside the window stays dimmed.
    let inside = (left + width as i16 / 4, top + height as i16 / 4);
    let outside = (left / 2, top + height as i16 / 4);
    daemon.until("the window under the pointer to show picked", || {
        let (i, o) = (pixel(&conn, root, inside), pixel(&conn, root, outside));
        (channel_gap(i, PICKED_RGB) <= 6 && o != WHITE).then_some(())
    });

    // A selection committed beside the window, then the pointer back on
    // the window: the window stays dimmed.
    x11::drag(&conn, root, (700, 600), (900, 800), 20);
    daemon.settle();
    x11::move_to(&conn, root, centre);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let i = pixel(&conn, root, inside);
    assert!(
        channel_gap(i, DIMMED_RGB) <= 6,
        "the window under the pointer shows {i:06x} over a committed selection, not dimmed \
         {DIMMED_RGB:06x}"
    );

    // A click there clears the selection and captures nothing; the next
    // motion picks the window again.
    x11::click(&conn, root, centre);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let saved = daemon::entries(dir.path()).unwrap_or_default().len();
    assert_eq!(
        saved, 0,
        "a click that cleared the selection captured the window"
    );
    x11::move_to(&conn, root, (centre.0 + 2, centre.1 + 2));
    daemon.until("the window to show picked again", || {
        (channel_gap(pixel(&conn, root, inside), PICKED_RGB) <= 6).then_some(())
    });
}
