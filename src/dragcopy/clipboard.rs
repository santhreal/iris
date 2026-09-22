use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as XprotoExt, EventMask, PropMode, SelectionNotifyEvent, Window,
    SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

use super::CbAtoms;

pub(super) fn serve_x11_clipboard(
    conn: RustConnection,
    win: Window,
    atoms: CbAtoms,
    uri_list: String,
    abs_str: String,
) {
    let (clipboard_atom, targets_atom, uri_list_atom, utf8_atom) =
        (atoms.clipboard, atoms.targets, atoms.uri_list, atoms.utf8);
    let deadline = Instant::now() + Duration::from_secs(300);
    // Event-driven: poll() the connection fd so a paste request is
    // answered the instant it arrives instead of up to a sleep
    // interval late, and an idle clipboard costs no wakeups.
    let x_fd = conn.stream().as_raw_fd();
    while Instant::now() < deadline {
        let event = match conn.poll_for_event() {
            Ok(Some(ev)) => ev,
            Ok(None) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let mut pfd = libc::pollfd {
                    fd: x_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                unsafe {
                    libc::poll(
                        &mut pfd,
                        1,
                        remaining.as_millis().min(i32::MAX as u128) as i32,
                    );
                }
                continue;
            }
            Err(_) => break,
        };

        match event {
            Event::SelectionClear(ev) => {
                if ev.selection == clipboard_atom {
                    break;
                }
            }
            Event::XfixesSelectionNotify(ev) => {
                if ev.selection == clipboard_atom && ev.owner != win {
                    break;
                }
            }
            Event::SelectionRequest(ev) if ev.selection == clipboard_atom => {
                let property = if ev.property == x11rb::NONE {
                    ev.target
                } else {
                    ev.property
                };

                let mut satisfied = false;
                if ev.target == targets_atom {
                    let targets = [targets_atom, uri_list_atom, utf8_atom];
                    if conn
                        .change_property32(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            AtomEnum::ATOM,
                            &targets,
                        )
                        .is_ok()
                    {
                        satisfied = true;
                    }
                } else if ev.target == uri_list_atom {
                    if conn
                        .change_property8(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            uri_list_atom,
                            uri_list.as_bytes(),
                        )
                        .is_ok()
                    {
                        satisfied = true;
                    }
                } else if ev.target == utf8_atom
                    && conn
                        .change_property8(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            utf8_atom,
                            abs_str.as_bytes(),
                        )
                        .is_ok()
                {
                    satisfied = true;
                }

                let notify = SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: ev.time,
                    requestor: ev.requestor,
                    selection: ev.selection,
                    target: ev.target,
                    property: if satisfied { property } else { x11rb::NONE },
                };

                let _ = conn.send_event(false, ev.requestor, EventMask::NO_EVENT, notify);
                let _ = conn.flush();
            }
            _ => {}
        }
    }

    let _ = conn.destroy_window(win);
    let _ = conn.flush();
}
