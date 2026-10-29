//! The home window's capture tile and the overlay's opening hover-snap,
//! driven on a private Xvfb and read back from the screen.
//!
//! WHY: two classes close here. "A surface iris opens is part of the
//! picture it takes": the New Screenshot tile ran the capture with Home
//! still on screen, so the frozen frame held Home under the overlay. A
//! compositing manager fades a closed window out over its last frame,
//! so the first case runs picom with a two-second fade: Home must leave
//! an empty frame behind it, not its tiles. It paints the root white,
//! clicks the tile, and reads the dimmed frozen frame where Home stood
//! against the dimmed root beside it.
//! "The overlay ignores the pointer until it moves": the overlay
//! opened with no hover-snap and no readout until the first motion, so
//! a capture from a hotkey with the pointer resting on a window showed
//! that window dimmed. The second case rests the pointer on a window,
//! opens the overlay, and reads the window revealed at full colour.
//!
//! Not covered: Wayland, where the pointer is unknown until it moves;
//! a window manager's frame or close animation; Windows and macOS. The
//! cases run only with `IRIS_X11_TEST_DISPLAY` set and need Xvfb, and
//! the first needs picom; without them they print that they did not
//! run.

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
use x11::{click, paint_root, pixel, rect, windows_of};

const HOME: &str = "iris";
const OVERLAY: &str = "Capture - iris";
const SCREEN: &str = "1280x1024x24";
const WHITE: u32 = 0x00ff_ffff;
/// The New Screenshot tile's centre in the home window, at scale 1
/// (home.rs: 36px margins, 168x124 tiles from y 214).
const CAPTURE_TILE: (i16, i16) = (120, 276);
/// A point on Home's header, away from the pointer's row and column
/// and from the readout beside the pointer.
const HEADER: (i16, i16) = (48, 60);
/// The pointer's window: x, y, width, height, and its colour.
const TARGET: (i16, i16, u16, u16) = (200, 200, 320, 240);
const TARGET_RGB: u32 = 0x0033_66cc;

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

#[test]
fn the_capture_tile_leaves_home_out_of_the_frozen_frame() {
    let case = "the_capture_tile_leaves_home_out_of_the_frozen_frame";
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
    daemon.send(&["--home"]);
    let home = daemon.until("home to open", || {
        windows_of(&conn, root, HOME).first().copied()
    });
    daemon.settle();
    let (x, y, width, _) = rect(&conn, root, home);
    assert!(x >= HEADER.0, "home at x {x} leaves no root beside it");
    assert!(
        pixel(&conn, root, (x + HEADER.0, y + HEADER.1)) != WHITE,
        "home did not draw its header"
    );
    click(&conn, root, (x + CAPTURE_TILE.0, y + CAPTURE_TILE.1));
    overlay_up(&conn, root, &daemon);
    assert!(
        windows_of(&conn, root, HOME).is_empty(),
        "home stayed open under the overlay"
    );
    // The dimmed root beside Home's rect, and the frozen frame where
    // Home stood: the same white, dimmed the same, once Home is out.
    let beside = (x - HEADER.0 / 2, y + HEADER.1);
    let inside = (x + HEADER.0, y + HEADER.1);
    let far = (x + width as i16 + HEADER.0 / 2, y + HEADER.1);
    daemon.until("the frozen frame to show the root where home stood", || {
        let (b, i, f) = (
            pixel(&conn, root, beside),
            pixel(&conn, root, inside),
            pixel(&conn, root, far),
        );
        (b != WHITE && b == i && i == f).then_some(())
    });
}

#[test]
fn the_overlay_opens_snapped_to_the_window_under_a_resting_pointer() {
    let case = "the_overlay_opens_snapped_to_the_window_under_a_resting_pointer";
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
    daemon.until("the window under the pointer to show undimmed", || {
        let (i, o) = (pixel(&conn, root, inside), pixel(&conn, root, outside));
        (i == TARGET_RGB && o != WHITE).then_some(())
    });
}
