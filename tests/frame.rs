//! WHY: an iris window draws its own frame and asks the window manager
//! for none (`_MOTIF_WM_HINTS` decorations 0) when a compositing manager
//! runs to blend its transparent corners, and asks for the window
//! manager's frame (decorations 1) when none runs. A window in the window
//! manager's frame draws no window controls and no resize strips of its
//! own. The classes closed here:
//!
//! - a frame decision read from anything but a compositing manager
//!   running when the window opens: a window manager that does not list
//!   `_GTK_FRAME_EXTENTS` in `_NET_SUPPORTED`, as Openbox, i3, and
//!   Fluxbox do, took a window manager frame under a running compositor;
//!   a compositor that started after the daemon connected, as one started
//!   beside the daemon at login does, was never seen; a compositor that
//!   exited after the daemon connected left windows with no frame and
//!   corners nothing blends;
//! - a window that draws window controls or resize strips inside the
//!   window manager's frame, beside the frame's own;
//! - a selection overlay in the window manager's frame, whose client area
//!   is then off the screen's origin, so that a point on the overlay is
//!   not the same point on the screen.
//!
//! Each state in `STATES` is set up on a private Xvfb with no window
//! manager: the case's connection stands in for the compositing manager
//! (it owns the screen's `_NET_WM_CM_S<n>` selection, EWMH) and writes the
//! root `_NET_SUPPORTED` list a window manager would. The states with a
//! compositor are the controls for the pointer checks: in them a drag on
//! the left strip resizes the window and a click on the close control
//! closes it.
//!
//! Not covered: a compositor that starts or exits while a window is open,
//! whose frame stays as it opened; how a real window manager draws the
//! frame; the square corners; Wayland, where the compositor draws the
//! frame when the window asks for one; Windows and macOS. The cases run
//! only with `IRIS_X11_TEST_DISPLAY` set and need `Xvfb`; without it a
//! case prints that it did not run.

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
use x11rb::protocol::xproto::{self, AtomEnum, ConnectionExt as _, PropMode};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{windows_of, Compositor};

/// A server's compositing manager and window manager, before the daemon
/// connects and when the window opens.
#[derive(Clone, Copy, Debug)]
struct State {
    /// A compositing manager runs when the daemon connects.
    at_connect: bool,
    /// A compositing manager runs when the window opens.
    at_open: bool,
    /// The window manager lists `_GTK_FRAME_EXTENTS` in `_NET_SUPPORTED`.
    frame_extents: bool,
}

const STATES: &[State] = &[
    // No compositor: the window manager's frame, whatever it supports.
    State {
        at_connect: false,
        at_open: false,
        frame_extents: true,
    },
    // A compositor under a window manager without _GTK_FRAME_EXTENTS.
    State {
        at_connect: true,
        at_open: true,
        frame_extents: false,
    },
    // A compositor that starts after the daemon connects.
    State {
        at_connect: false,
        at_open: true,
        frame_extents: false,
    },
    // A compositor that exits after the daemon connects.
    State {
        at_connect: true,
        at_open: false,
        frame_extents: true,
    },
];

/// The option that opens the window, and the window's title: a
/// resizable window, so that a frame the window draws has resize strips.
const OPEN: (&str, &str) = ("--library", "Library - iris");

/// The title of the capture overlay's window.
const OVERLAY: &str = "Capture - iris";

/// The screen: room around the window, which opens centered, for a drag
/// on its left strip.
const SCREEN: &str = "1280x1024x24";

/// The `_MOTIF_WM_HINTS` decorations field of a window the window manager
/// frames, and of a window that draws its own frame.
const FRAMED: u32 = 1;
const UNFRAMED: u32 = 0;

/// The close control of a frame the window draws, from the window's
/// top-left corner in pixels: the first control, 20 px in and centered
/// in the 52 px toolbar.
const CLOSE: (i16, i16) = (26, 26);

/// A point on the left resize strip, 6 px wide, from the window's left
/// edge, and how far a drag takes it leftward.
const STRIP: i16 = 2;
const DRAG: i16 = 60;

#[test]
fn a_window_draws_its_own_frame_exactly_when_a_compositor_runs_as_it_opens() {
    if !enabled() {
        return;
    }
    let (option, title) = OPEN;
    let mut wrong = Vec::new();
    for &state in STATES {
        let case = format!("a window frame with {state:?}");
        let Some((_xvfb, display)) = xvfb_sized(&case, SCREEN) else {
            return;
        };
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let atom = |name: &str| {
            conn.intern_atom(false, name.as_bytes())
                .unwrap()
                .reply()
                .unwrap()
                .atom
        };
        let supported: Vec<u32> = ["_NET_SUPPORTED", "_NET_WM_NAME", "_NET_WM_STATE"]
            .into_iter()
            .chain(state.frame_extents.then_some("_GTK_FRAME_EXTENTS"))
            .map(atom)
            .collect();
        conn.change_property32(
            PropMode::REPLACE,
            root,
            atom("_NET_SUPPORTED"),
            AtomEnum::ATOM,
            &supported,
        )
        .unwrap();
        let selection = atom(&format!("_NET_WM_CM_S{screen}"));
        let hints = atom("_MOTIF_WM_HINTS");

        let compositor = state
            .at_connect
            .then(|| Compositor::start(&conn, root, selection));
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        // The round trip of the compositor's change orders it before the
        // window's open.
        let _compositor = match (compositor, state.at_open) {
            (None, true) => Some(Compositor::start(&conn, root, selection)),
            (Some(running), false) => {
                running.exit(&conn);
                None
            }
            (running, _) => running,
        };
        daemon.send(&[option]);
        let window = daemon.until(
            &format!("{option} to open a window titled {title:?}, {state:?}"),
            || match windows_of(&conn, root, title).as_slice() {
                [window] => Some(*window),
                _ => None,
            },
        );
        let decorations = conn
            .get_property(false, window, hints, AtomEnum::ANY, 0, 5)
            .unwrap()
            .reply()
            .unwrap()
            .value32()
            .and_then(|mut fields| fields.nth(2));
        let own = state.at_open;
        let expected = if own { UNFRAMED } else { FRAMED };
        if decorations != Some(expected) {
            wrong.push(format!(
                "{state:?}: decorations {decorations:?}, expected {expected}; daemon log:\n{}",
                daemon.log()
            ));
        }

        // The window's resize strips and window controls, through the
        // pointer, once the window has drawn its first frame.
        daemon.settle();
        let before = rect(&conn, root, window);
        drag(
            &conn,
            root,
            (before.0 + STRIP, before.1 + (before.3 / 2) as i16),
            -DRAG,
            &daemon,
        );
        let after = rect(&conn, root, window);
        if (after != before) != own {
            wrong.push(format!(
                "{state:?}: a drag on the left strip took the window from {before:?} to \
                 {after:?}; the window draws its own strips: {own}"
            ));
        }
        x11::click(&conn, root, (after.0 + CLOSE.0, after.1 + CLOSE.1));
        daemon.settle();
        let closed = windows_of(&conn, root, title).is_empty();
        if closed != own {
            wrong.push(format!(
                "{state:?}: a click on the close control closed the window: {closed}; the \
                 window draws its own controls: {own}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the frames of windows opened in these states:\n{}",
        wrong.join("\n")
    );
}

/// The overlay's windows, the one a capture opens and the one the daemon
/// keeps ready, span the screen with their client area: no window manager
/// frame, with or without a compositor.
#[test]
fn the_overlay_asks_for_no_window_manager_frame() {
    if !enabled() {
        return;
    }
    for compositor in [false, true] {
        let case = format!("an overlay frame, compositor {compositor}");
        let Some((_xvfb, display)) = xvfb_sized(&case, SCREEN) else {
            return;
        };
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let atom = |name: &str| {
            conn.intern_atom(false, name.as_bytes())
                .unwrap()
                .reply()
                .unwrap()
                .atom
        };
        let hints = atom("_MOTIF_WM_HINTS");
        let _compositor = compositor
            .then(|| Compositor::start(&conn, root, atom(&format!("_NET_WM_CM_S{screen}"))));
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        daemon.send(&["--capture"]);
        daemon.until("the overlay to take the focus", || {
            windows_of(&conn, root, OVERLAY)
                .contains(&x11::focus(&conn))
                .then_some(())
        });
        daemon.until(
            &format!("every overlay window to ask for no frame, compositor {compositor}"),
            || {
                let windows = windows_of(&conn, root, OVERLAY);
                let unframed = |&window: &u32| {
                    conn.get_property(false, window, hints, AtomEnum::ANY, 0, 5)
                        .unwrap()
                        .reply()
                        .ok()
                        .and_then(|p| p.value32().and_then(|mut fields| fields.nth(2)))
                        == Some(UNFRAMED)
                };
                (!windows.is_empty() && windows.iter().all(unframed)).then_some(())
            },
        );
    }
}

/// The window's client area in root pixels: x, y, width, height.
fn rect(conn: &RustConnection, root: u32, window: u32) -> (i16, i16, u16, u16) {
    let at = conn
        .translate_coordinates(window, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let size = conn.get_geometry(window).unwrap().reply().unwrap();
    (at.dst_x, at.dst_y, size.width, size.height)
}

/// Press the first button at `from`, move the pointer `dx` pixels in
/// steps, and release it, through XTest. The daemon settles after the
/// press and after the moves, so it has read each before the next.
fn drag(conn: &RustConnection, root: u32, from: (i16, i16), dx: i16, daemon: &Daemon) {
    let motion = |x: i16| {
        conn.xtest_fake_input(
            xproto::MOTION_NOTIFY_EVENT,
            0,
            x11rb::CURRENT_TIME,
            root,
            x,
            from.1,
            0,
        )
        .unwrap();
        conn.flush().unwrap();
    };
    let button = |kind: u8| {
        conn.xtest_fake_input(kind, 1, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .unwrap();
        conn.flush().unwrap();
    };
    motion(from.0);
    button(xproto::BUTTON_PRESS_EVENT);
    daemon.settle();
    for step in 1..=6 {
        motion(from.0 + dx * step / 6);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    daemon.settle();
    button(xproto::BUTTON_RELEASE_EVENT);
    daemon.settle();
}
