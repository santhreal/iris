//! WHY: the daemon opens the capture overlay as it starts and parks it,
//! and parks it again after each capture, so the next capture reuses the
//! live window. A park minimized the window, and a minimize is a request
//! to the window manager (ICCCM WM_CHANGE_STATE): on an X11 screen with
//! no window manager the overlay stayed mapped over every monitor, black
//! where no compositing manager ran. The class closed here is a parked
//! overlay that stays on a screen with no window manager, after the
//! warmup or after a capture, and a parked overlay that a capture then
//! fails to bring back. It also closes "the warm overlay takes input
//! it was never shown for": the warmup maps the overlay over the whole
//! screen before the park, and while mapped it took a click meant for
//! Home, and a record pick's click resolved to it and recorded the
//! invisible overlay. The warm overlay's input shape must be empty, and
//! a capture must restore it.
//!
//! The first case runs on a private Xvfb with no window manager. The
//! second puts a window manager on it, reduced to what a park meets: the
//! warmup's overlay was minimized, and iris then raised it again, once
//! the window manager listed the window and again 200 ms later, to span
//! the screen. xfwm4 restores a minimized window that is raised while
//! another iris window has the focus, so the library opened during the
//! warmup showed the dimmed capture overlay over the desktop. The class
//! closed there is a restack of the overlay after its park. Not covered:
//! a real window manager's response to a restack; a restack after a
//! capture's park; a park before the window manager lists the window,
//! which this window manager answers too fast to reach, and which the
//! fixup queue's unit test covers; Wayland, Windows, and macOS, which
//! destroy the overlay in place of a park. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need `Xvfb`; without it a case prints
//! that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK};
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigWindow, ConfigureWindowAux, ConnectionExt as _,
    EventMask, PropMode,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

use daemon::{enabled, xvfb, Daemon};
use x11::{all_windows_of, focus, tap, windows_of};

/// The title of the capture overlay's window.
const OVERLAY: &str = "Capture - iris";

/// The keysym of `Escape`, which cancels a capture.
const ESCAPE: u32 = 0xff1b;

/// The rectangles of `window`'s input shape: none passes every click
/// through, one is the default, the window's whole rect.
fn input_shape(conn: &impl Connection, window: u32) -> usize {
    conn.shape_get_rectangles(window, SK::INPUT)
        .unwrap()
        .reply()
        .unwrap()
        .rectangles
        .len()
}

/// What the stand-in window manager (`window_manager`) saw.
#[derive(Default)]
struct Seen {
    /// Windows minimized through ICCCM WM_CHANGE_STATE, and not mapped
    /// since.
    iconic: Vec<u32>,
    /// Each restack request for a minimized window: its window and the
    /// request.
    restacks: Vec<(u32, &'static str)>,
}

/// A window manager on `display`, on a thread of its own until `stop`
/// is set: it maps a window that asks and lists it in the root's
/// `_NET_CLIENT_LIST`, unmaps a window that asks to be iconic, and
/// records a restack of an iconic window: a ConfigureWindow with a stack
/// mode, or a `_NET_WM_STATE` request. It holds SubstructureRedirect on
/// the root before it returns.
fn window_manager(display: &str, stop: Arc<AtomicBool>) -> Arc<Mutex<Seen>> {
    let (wm, screen) = x11rb::connect(Some(display)).unwrap();
    let root = wm.setup().roots[screen].root;
    let redirect = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    wm.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(redirect))
        .unwrap()
        .check()
        .unwrap();
    let atom = |name: &[u8]| wm.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let (client_list, change_state, net_state) = (
        atom(b"_NET_CLIENT_LIST"),
        atom(b"WM_CHANGE_STATE"),
        atom(b"_NET_WM_STATE"),
    );
    let seen = Arc::new(Mutex::new(Seen::default()));
    let log = seen.clone();
    std::thread::spawn(move || {
        let mut clients = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let Some(event) = wm.poll_for_event().unwrap() else {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            };
            let mut seen = log.lock();
            match event {
                Event::MapRequest(ev) => {
                    wm.map_window(ev.window).unwrap();
                    seen.iconic.retain(|&w| w != ev.window);
                    if !clients.contains(&ev.window) {
                        clients.push(ev.window);
                    }
                    wm.change_property32(
                        PropMode::REPLACE,
                        root,
                        client_list,
                        AtomEnum::WINDOW,
                        &clients,
                    )
                    .unwrap();
                }
                Event::ConfigureRequest(ev) => {
                    if ev.value_mask.contains(ConfigWindow::STACK_MODE)
                        && seen.iconic.contains(&ev.window)
                    {
                        seen.restacks
                            .push((ev.window, "ConfigureWindow stack mode"));
                    }
                    let aux = ConfigureWindowAux::from_configure_request(&ev);
                    wm.configure_window(ev.window, &aux).unwrap();
                }
                Event::ClientMessage(ev) => {
                    let iconic = seen.iconic.contains(&ev.window);
                    if ev.type_ == change_state && ev.data.as_data32()[0] == 3 {
                        wm.unmap_window(ev.window).unwrap();
                        if !iconic {
                            seen.iconic.push(ev.window);
                        }
                    } else if ev.type_ == net_state && iconic {
                        seen.restacks.push((ev.window, "_NET_WM_STATE"));
                    }
                }
                _ => {}
            }
            wm.flush().unwrap();
        }
    });
    seen
}

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
    assert_eq!(
        input_shape(&conn, warm),
        0,
        "the warm overlay takes pointer input while it is not in use"
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
        assert_ne!(
            input_shape(&conn, warm),
            0,
            "capture {capture}'s overlay lets pointer input through"
        );
        tap(&conn, root, ESCAPE);
        daemon.until(&format!("capture {capture}'s overlay to park"), || {
            windows_of(&conn, root, OVERLAY).is_empty().then_some(())
        });
    }
}

#[test]
fn a_parked_overlay_is_not_raised_again_under_a_window_manager() {
    if !enabled() {
        return;
    }
    let case = "a_parked_overlay_is_not_raised_again_under_a_window_manager";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let stop = Arc::new(AtomicBool::new(false));
    let seen = window_manager(&display, stop.clone());
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let warm = daemon.until("the warmup to open the overlay", || {
        all_windows_of(&conn, root, OVERLAY).first().copied()
    });
    daemon.until("the warmup to minimize the overlay", || {
        seen.lock().iconic.contains(&warm).then_some(())
    });
    // Past the second span assertion, 200 ms after the window manager
    // lists the window, and the 50 ms rescan bound.
    std::thread::sleep(Duration::from_millis(1000));
    stop.store(true, Ordering::Relaxed);
    let seen = seen.lock();
    assert_eq!(
        seen.restacks,
        Vec::<(u32, &str)>::new(),
        "iris restacked the parked overlay {warm:#x}"
    );
    assert_eq!(
        seen.iconic,
        vec![warm],
        "the warmup did not leave the overlay minimized"
    );
    assert!(
        windows_of(&conn, root, OVERLAY).is_empty(),
        "the parked overlay is mapped"
    );
}
