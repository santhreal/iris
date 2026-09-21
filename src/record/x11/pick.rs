use std::os::unix::io::AsRawFd;
use std::sync::mpsc::Receiver;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as XprotoExt, EventMask, GrabMode, GrabStatus};
use x11rb::protocol::Event;
use x11rb::CURRENT_TIME;

use crate::record::CANCELLED_PREFIX;

use super::mark::{connect, escape_keycodes, make_crosshair};
use super::PickedWindow;

/// Click-to-pick: grab pointer and keyboard, wait for a click (target) or
/// Escape (cancel). Returns the top-level window under the click. The
/// stop channel is polled between events: a stop during the pick must
/// end the wait, or the recording thread never joins and the daemon
/// wedges on the next command.
pub fn pick_window(stop: &Receiver<()>) -> Result<PickedWindow, String> {
    let (conn, screen_num) = connect()?;
    let root = conn.setup().roots[screen_num].root;
    let cursor = make_crosshair(&conn)?;
    let escapes = escape_keycodes(&conn)?;

    let status = conn
        .grab_pointer(
            false,
            root,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            root,
            cursor,
            CURRENT_TIME,
        )
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("grab_pointer: {e}"))?;
    if status.status != GrabStatus::SUCCESS {
        return Err(format!("pointer grab failed: {:?}", status.status));
    }
    let _kb = conn
        .grab_keyboard(false, root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)
        .map_err(|e| e.to_string())?
        .reply();

    let x_fd = conn.stream().as_raw_fd();
    let picked = loop {
        match stop.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                break Err(format!("{CANCELLED_PREFIX} pick stopped"));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        // Sleep on the connection fd: an X event wakes the poll
        // instantly, and the 50ms timeout re-checks the stop channel.
        // A bare sleep would burn a wake every 10ms for nothing.
        let mut pfd = libc::pollfd {
            fd: x_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe {
            libc::poll(&mut pfd, 1, 50);
        }
        let Some(event) = conn
            .poll_for_event()
            .map_err(|e| format!("poll_for_event: {e}"))?
        else {
            continue;
        };
        match event {
            Event::ButtonPress(_) => {
                let ptr = conn
                    .query_pointer(root)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| format!("query_pointer: {e}"))?;
                let child = ptr.child;
                if child == x11rb::NONE || child == root {
                    break Err(format!(
                        "{CANCELLED_PREFIX} clicked the desktop, not a window"
                    ));
                }
                // Walk up to the top-level ancestor (direct child of root).
                let mut win = child;
                loop {
                    let tree = conn
                        .query_tree(win)
                        .map_err(|e| e.to_string())?
                        .reply()
                        .map_err(|e| format!("query_tree: {e}"))?;
                    if tree.parent == root || tree.parent == x11rb::NONE {
                        break;
                    }
                    win = tree.parent;
                }
                let geom = conn
                    .get_geometry(win)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| format!("get_geometry: {e}"))?;
                break Ok(PickedWindow {
                    id: win,
                    width: u32::from(geom.width),
                    height: u32::from(geom.height),
                });
            }
            Event::KeyPress(ev) if escapes.contains(&ev.detail) => {
                break Err(format!("{CANCELLED_PREFIX} pick aborted"));
            }
            _ => {}
        }
    };

    let _ = conn.ungrab_pointer(CURRENT_TIME);
    let _ = conn.ungrab_keyboard(CURRENT_TIME);
    let _ = conn.free_cursor(cursor);
    picked
}
