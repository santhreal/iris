//! WHY: the daemon opens the capture overlay as it starts and parks it,
//! and parks it again after each capture, so the next capture reuses the
//! live window. A park minimized the window, and a minimize is a request
//! to the window manager (ICCCM WM_CHANGE_STATE): on an X11 screen with
//! no window manager the overlay stayed mapped over every monitor, black
//! where no compositing manager ran. The class closed here is a parked
//! overlay that stays on a screen with no window manager, after the
//! warmup or after a capture, and a parked overlay that a capture then
//! fails to bring back.
//!
//! The case runs on a private Xvfb with no window manager. Not covered:
//! the minimize through a window manager, which needs one running;
//! Wayland, Windows, and macOS, which destroy the overlay in place of a
//! park. The case runs only with `IRIS_X11_TEST_DISPLAY` set and needs
//! `Xvfb`; without it the case prints that it did not run.

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

use daemon::{enabled, xvfb, Daemon};
use x11::{all_windows_of, focus, tap, windows_of};

/// The title of the capture overlay's window.
const OVERLAY: &str = "Capture - iris";

/// The keysym of `Escape`, which cancels a capture.
const ESCAPE: u32 = 0xff1b;

#[test]
fn a_parked_overlay_leaves_a_screen_with_no_window_manager() {
    if !enabled() {
        return;
    }
    let case = "a_parked_overlay_leaves_a_screen_with_no_window_manager";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let warm = daemon.until("the warmup to open the overlay", || {
        all_windows_of(&conn, root, OVERLAY).first().copied()
    });
    daemon.settle();
    let shown = windows_of(&conn, root, OVERLAY);
    assert!(
        shown.is_empty(),
        "the warmup left the overlay mapped: {shown:x?}"
    );

    for capture in 1..=2 {
        daemon.send(&["--capture"]);
        let focused = daemon.until(
            &format!("capture {capture}'s overlay to take the focus"),
            || {
                let focused = focus(&conn);
                windows_of(&conn, root, OVERLAY)
                    .contains(&focused)
                    .then_some(focused)
            },
        );
        assert_eq!(
            focused, warm,
            "capture {capture} opened another overlay window"
        );
        tap(&conn, root, ESCAPE);
        daemon.until(&format!("capture {capture}'s overlay to park"), || {
            windows_of(&conn, root, OVERLAY).is_empty().then_some(())
        });
    }
}
