//! Every window an `iris --help` option opens takes keys from its first
//! frame, before any click.
//!
//! WHY: the class closed here is "a window opens with nothing focused".
//! A window's key handler runs only for its focused element, so the
//! Library opened with every key dead: the arrows, Return, Space, and
//! Escape did nothing until a click landed outside every card. The case
//! opens each window in `OPTIONS` that has one subject, presses Escape
//! with the pointer over it, and waits for the window to close, which
//! every one of them does on Escape. A window added to `OPTIONS` joins
//! the case.
//!
//! Not covered: windows no option opens (the capture overlay, the
//! toast, the recording chip), which take keys through their own cases;
//! a window manager's focus policy, since the test server has none;
//! Wayland, Windows, and macOS. The case runs only with
//! `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without them it prints
//! that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the table's helpers.
#[allow(dead_code)]
#[path = "support/options.rs"]
mod options;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

use daemon::{enabled, xvfb_sized, Daemon};
use options::{open_args, single_png, Kind, OPTIONS};
use x11::{move_to, tap, windows_of};

const ESCAPE: u32 = 0xff1b;

#[test]
fn every_window_takes_escape_before_any_click() {
    if !enabled() {
        return;
    }
    let case = "every_window_takes_escape_before_any_click";
    // A screen every window fits.
    let Some((_xvfb, display)) = xvfb_sized(case, "1600x1200x24") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let shots = dir.path().join("shots");
    std::fs::create_dir_all(&shots).unwrap();
    let shot = single_png(&shots);
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    for &(option, kind) in OPTIONS {
        let Kind::Single(title) = kind else {
            continue;
        };
        daemon.send(&open_args(option, &shot));
        let window = daemon.until(&format!("{option} to open {title:?}"), || {
            windows_of(&conn, root, title).first().copied()
        });
        daemon.settle();
        let at = conn
            .translate_coordinates(window, root, 0, 0)
            .unwrap()
            .reply()
            .unwrap();
        let size = conn.get_geometry(window).unwrap().reply().unwrap();
        move_to(
            &conn,
            root,
            (
                at.dst_x + (size.width / 2) as i16,
                at.dst_y + (size.height / 2) as i16,
            ),
        );
        daemon.settle();
        tap(&conn, root, ESCAPE);
        daemon.until(
            &format!("Escape to close {title:?}, opened by {option}, before any click"),
            || windows_of(&conn, root, title).is_empty().then_some(()),
        );
    }
}
