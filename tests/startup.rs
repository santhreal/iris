//! A daemon binds its single-instance socket before it opens a window.
//!
//! WHY: the class closed here is "the socket waits on the display". A
//! client that finds no daemon starts one, forwards its command once the
//! socket accepts, and fails after 8 s; a second `iris` that finds no
//! socket starts a daemon of its own. The daemon bound its socket after
//! the overlay warmup, whose renderer init on lavapipe took 0.2 s with a
//! warm shader cache and seconds with a cold one.
//!
//! The case grabs the daemon's X server once the daemon prints its start
//! line. From then on every request the daemon sends the server waits for
//! the grab's release, and the socket must bind while the grab holds. Not
//! covered: startup work before the start line (the X connection, the GPU
//! device, the fonts), and work that delays the bind without a request to
//! the server. The case runs only with `IRIS_X11_TEST_DISPLAY` set and
//! needs `Xvfb`; without it the case prints that it did not run.

#![cfg(target_os = "linux")]

// The exit cases use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;

use daemon::{enabled, xvfb, Daemon};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::ConnectionExt as _;

/// The daemon's first line, printed once its platform has connected to
/// the display and before the rest of its start runs.
const START: &str = "iris: daemon start";

#[test]
fn a_daemon_binds_its_socket_before_it_opens_a_window() {
    if !enabled() {
        return;
    }
    let case = "a_daemon_binds_its_socket_before_it_opens_a_window";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, _) = x11rb::connect(Some(&display)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::spawn(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.until("the daemon's start line", || {
        daemon.log().contains(START).then_some(())
    });
    conn.grab_server().unwrap();
    // The reply comes after the grab: the server holds every other
    // client from here on.
    conn.get_input_focus().unwrap().reply().unwrap();
    daemon.bound();
    conn.ungrab_server().unwrap();
    conn.flush().unwrap();
}
