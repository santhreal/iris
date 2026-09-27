//! WHY: a second open of a surface with one window activates the window
//! already open, and on X11 the path of that activation depends on the
//! server's window manager. With an EWMH window manager running, the
//! window sends it a _NET_ACTIVE_WINDOW request and takes no focus
//! itself, so the window manager's focus policy applies. With none,
//! nothing handles that request, and the window raises itself and takes
//! the input focus. The class closed here is an activation that takes
//! the path of the wrong state, for each state in `STATES`, set up on a
//! private Xvfb before the daemon connects:
//!
//! - no _NET_SUPPORTING_WM_CHECK on the root;
//! - a root _NET_SUPPORTING_WM_CHECK that names a destroyed window, as a
//!   window manager that exits leaves it;
//! - a running window manager: the root property names a window whose
//!   own property names itself. The case's connection stands in for the
//!   window manager: it reads the request and grants no focus.
//!
//! Not covered: a window manager that starts or exits after the daemon
//! connects, which the daemon reads only at its start; the stacking
//! order of the raise; a real window manager's handling of the request;
//! Wayland, Windows, and macOS. The case runs only with
//! `IRIS_X11_TEST_DISPLAY` set and needs `Xvfb`; without it the case
//! prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask,
    InputFocus, PropMode, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

use daemon::{enabled, xvfb, Daemon};
use x11::{focus, windows_of};

/// The window manager state of the server a daemon connects to.
#[derive(Clone, Copy, Debug)]
enum Wm {
    None,
    Exited,
    Running,
}

const STATES: &[Wm] = &[Wm::None, Wm::Exited, Wm::Running];

/// The option that opens the window, and the window's title.
const OPEN: (&str, &str) = ("--settings", "Settings - iris");

/// How long a focus change that should not happen has to happen.
const SETTLE: Duration = Duration::from_millis(800);

#[test]
fn a_second_open_activates_through_the_window_manager_only_when_one_runs() {
    if !enabled() {
        return;
    }
    let (option, title) = OPEN;
    for &state in STATES {
        let case = format!("a second open with window manager state {state:?}");
        let Some((_xvfb, display)) = xvfb(&case) else {
            return;
        };
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
        let (check, active) = (
            atom(b"_NET_SUPPORTING_WM_CHECK"),
            atom(b"_NET_ACTIVE_WINDOW"),
        );
        match state {
            Wm::None => {}
            Wm::Exited | Wm::Running => {
                let wm = conn.generate_id().unwrap();
                conn.create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    wm,
                    root,
                    -1,
                    -1,
                    1,
                    1,
                    0,
                    WindowClass::INPUT_ONLY,
                    x11rb::COPY_FROM_PARENT,
                    &CreateWindowAux::new(),
                )
                .unwrap();
                for window in [root, wm] {
                    conn.change_property32(
                        PropMode::REPLACE,
                        window,
                        check,
                        AtomEnum::WINDOW,
                        &[wm],
                    )
                    .unwrap();
                }
                if let Wm::Exited = state {
                    conn.destroy_window(wm).unwrap();
                }
            }
        }
        // A request to the window manager reaches every client that
        // selects substructure notification on the root.
        let notify = ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY);
        conn.change_window_attributes(root, &notify).unwrap();
        // The round trip orders the state before the daemon's connection.
        focus(&conn);

        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        daemon.send(&[option]);
        let first = daemon.until(
            &format!("{option} to open a window titled {title:?}"),
            || match windows_of(&conn, root, title).as_slice() {
                [window] => Some(*window),
                _ => None,
            },
        );
        conn.set_input_focus(InputFocus::POINTER_ROOT, root, x11rb::CURRENT_TIME)
            .unwrap();
        assert_eq!(focus(&conn), root);
        while conn.poll_for_event().unwrap().is_some() {}
        daemon.send(&[option]);
        match state {
            Wm::None | Wm::Exited => {
                daemon.until(
                    &format!("a second {option} to focus the open window, {state:?}"),
                    || (focus(&conn) == first).then_some(()),
                );
            }
            Wm::Running => {
                daemon.until(
                    &format!("a second {option} to request _NET_ACTIVE_WINDOW for the open window"),
                    || loop {
                        match conn.poll_for_event().unwrap()? {
                            Event::ClientMessage(e) if e.type_ == active && e.window == first => {
                                return Some(());
                            }
                            _ => {}
                        }
                    },
                );
                std::thread::sleep(SETTLE);
                assert_eq!(
                    focus(&conn),
                    root,
                    "a second {option} took the focus past the running window manager"
                );
            }
        }
    }
}
