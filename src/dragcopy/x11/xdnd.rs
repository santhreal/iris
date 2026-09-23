use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConnectionExt as XprotoExt, EventMask, KeyButMask,
    PropMode, SelectionNotifyEvent, SelectionRequestEvent, Window, SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;
use x11rb::CURRENT_TIME;

use super::atom_cached;
use super::icon::IconWindow;
use crate::dragcopy::DragIcon;

pub(super) struct XdndAtoms {
    pub(super) selection: Atom,
    pub(super) aware: Atom,
    pub(super) proxy: Atom,
    pub(super) enter: Atom,
    pub(super) position: Atom,
    pub(super) status: Atom,
    pub(super) leave: Atom,
    pub(super) drop: Atom,
    pub(super) finished: Atom,
    pub(super) action_copy: Atom,
    pub(super) uri_list: Atom,
    pub(super) targets: Atom,
    pub(super) utf8: Atom,
}

impl XdndAtoms {
    pub(super) fn intern<C: Connection>(conn: &C) -> Result<Self, String> {
        let get = |name: &'static [u8]| {
            atom_cached(conn, name)
                .ok_or_else(|| format!("intern {} failed", String::from_utf8_lossy(name)))
        };
        Ok(Self {
            selection: get(b"XdndSelection")?,
            aware: get(b"XdndAware")?,
            proxy: get(b"XdndProxy")?,
            enter: get(b"XdndEnter")?,
            position: get(b"XdndPosition")?,
            status: get(b"XdndStatus")?,
            leave: get(b"XdndLeave")?,
            drop: get(b"XdndDrop")?,
            finished: get(b"XdndFinished")?,
            action_copy: get(b"XdndActionCopy")?,
            uri_list: get(b"text/uri-list")?,
            targets: get(b"TARGETS")?,
            utf8: get(b"UTF8_STRING")?,
        })
    }
}

/// Deepest XdndAware window containing (x, y) in root coordinates, and
/// its protocol version. `aware_cache` memoizes per-window awareness:
/// the drag polls this every 16ms and a get_property per window per
/// poll is a round-trip storm over a busy desktop.
fn xdnd_target_at<C: Connection>(
    conn: &C,
    win: Window,
    first_child: Window,
    aware: Atom,
    proxy: Atom,
    skip: Option<Window>,
    aware_cache: &mut HashMap<Window, Option<(Window, u32)>>,
) -> Option<(Window, u32)> {
    // The chain from `win` to the deepest window under the pointer:
    // query_pointer reports the immediate child containing the pointer,
    // so one round trip per level replaces a query_tree plus a
    // geometry+attributes pair per sibling. Unmapped windows cannot
    // contain the pointer, so no map-state check is needed. The
    // caller's own query_pointer on `win` supplies the first link:
    // re-asking it here was a duplicate round trip per pointer move.
    let mut chain = vec![win];
    let mut cur = first_child;
    if cur != x11rb::NONE && Some(cur) != skip {
        chain.push(cur);
    }
    for _ in 0..32 {
        if cur == x11rb::NONE || Some(cur) == skip {
            break;
        }
        let Some(ptr) = conn.query_pointer(cur).ok().and_then(|c| c.reply().ok()) else {
            break;
        };
        let child = ptr.child;
        if child == x11rb::NONE || Some(child) == skip {
            break;
        }
        chain.push(child);
        cur = child;
    }

    // XDnD targets the deepest XdndAware window on the chain. Awareness
    // is cached per window across the 16ms drag polls.
    let aware_of = |conn: &C,
                    w: Window,
                    cache: &mut HashMap<Window, Option<(Window, u32)>>|
     -> Option<(Window, u32)> {
        *cache.entry(w).or_insert_with(|| {
            let mut t = None;
            if let Some(reply) = conn
                .get_property(false, w, proxy, AtomEnum::ANY, 0, 2)
                .ok()
                .and_then(|c| c.reply().ok())
            {
                if let Some(mut it) = reply.value32() {
                    if let (Some(proxy_win), Some(version)) = (it.next(), it.next()) {
                        t = Some((proxy_win, version.min(5)));
                    }
                }
            }
            if t.is_none() {
                if let Some(reply) = conn
                    .get_property(false, w, aware, AtomEnum::ANY, 0, 1)
                    .ok()
                    .and_then(|c| c.reply().ok())
                {
                    if let Some(version) = reply.value32().and_then(|mut it| it.next()) {
                        t = Some((w, version.min(5)));
                    }
                }
            }
            t
        })
    };

    for w in chain.iter().rev() {
        if let Some(target) = aware_of(conn, *w, aware_cache) {
            return Some(target);
        }
    }
    None
}

fn send_client<C: Connection>(conn: &C, target: Window, type_: Atom, data: [u32; 5]) {
    let _ = conn.send_event(
        false,
        target,
        EventMask::NO_EVENT,
        ClientMessageEvent::new(32, target, type_, data),
    );
}

/// Sleep until the connection has an event or `ms` elapse: the drag
/// loop's pointer cadence stays 16ms, but XdndFinished and
/// SelectionRequest wake it the instant they land.
fn wait_event_or(conn: &RustConnection, ms: i32) {
    let mut pfd = libc::pollfd {
        fd: conn.stream().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe {
        libc::poll(&mut pfd, 1, ms);
    }
}

/// XDnD source state machine. Runs until the drop completes, the drag is
/// cancelled, or the safety deadline passes.
pub(super) fn run_xdnd_drag(
    conn: RustConnection,
    win: Window,
    root: Window,
    atoms: XdndAtoms,
    uri_list: String,
    icon: Option<DragIcon>,
) {
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut target: Option<Window> = None;
    let mut accepted = false;
    let mut dropped = false;
    let mut last_pos = (i32::MIN, i32::MIN);
    let mut last_icon_pos = (i32::MIN, i32::MIN);
    let mut aware_cache = HashMap::new();
    let icon = icon.and_then(|icon| IconWindow::show(&conn, root, icon));

    while Instant::now() < deadline {
        while let Ok(Some(event)) = conn.poll_for_event() {
            match event {
                Event::ClientMessage(ev) => {
                    if ev.type_ == atoms.status {
                        // XDND: bit 0 of l[1] is the accept flag; bit 1 only
                        // asks for idle position updates.
                        accepted = ev.data.as_data32()[1] & 1 != 0;
                    } else if ev.type_ == atoms.finished {
                        // l[0] is the accepted flag: a refused drop
                        // still ends the drag, but the log distinguishes
                        // "target rejected" from a real copy.
                        let ok = ev.data.as_data32()[0] != 0;
                        crate::ilog!("iris: dragcopy: XdndFinished accepted={ok}");
                        if let Some(icon) = &icon {
                            icon.destroy(&conn);
                        }
                        let _ = conn.destroy_window(win);
                        let _ = conn.flush();
                        return;
                    }
                }
                Event::SelectionRequest(ev) => {
                    if ev.selection == atoms.selection {
                        serve_drag_request(&conn, &atoms, &uri_list, &ev);
                    }
                }
                Event::SelectionClear(ev) if ev.selection == atoms.selection => {
                    if let Some(icon) = &icon {
                        icon.destroy(&conn);
                    }
                    let _ = conn.destroy_window(win);
                    let _ = conn.flush();
                    return;
                }
                _ => {}
            }
        }

        let Some(pointer) = conn.query_pointer(root).ok().and_then(|c| c.reply().ok()) else {
            break;
        };
        let held = pointer.mask.contains(KeyButMask::BUTTON1);
        let (px, py) = (i32::from(pointer.root_x), i32::from(pointer.root_y));

        if !held {
            if dropped {
                // Drop sent: stay alive until XdndFinished (or the
                // deadline) so the target can convert the selection.
                // poll() wakes the instant the reply lands instead of
                // up to 16ms late.
                wait_event_or(&conn, 16);
                continue;
            }
            if let Some(icon) = &icon {
                icon.destroy(&conn);
            }
            if let Some(t) = target.take() {
                if accepted {
                    send_client(&conn, t, atoms.drop, [win, 0, CURRENT_TIME, 0, 0]);
                    dropped = true;
                    let _ = conn.flush();
                    continue;
                }
                send_client(&conn, t, atoms.leave, [win, 0, 0, 0, 0]);
            }
            break;
        }
        // A stationary pointer cannot change the target chain: skip
        // the per-level query_pointer walk (a round trip each) and
        // the position resend. The 16ms cadence still polls for the
        // button release.
        if (px, py) != last_pos {
            let under = xdnd_target_at(
                &conn,
                root,
                pointer.child,
                atoms.aware,
                atoms.proxy,
                icon.as_ref().map(|i| i.win),
                &mut aware_cache,
            );
            let under_win = under.map(|(w, _)| w);
            if under_win != target {
                if let Some(old) = target.take() {
                    send_client(&conn, old, atoms.leave, [win, 0, 0, 0, 0]);
                }
                accepted = false;
                if let Some((w, version)) = under {
                    send_client(
                        &conn,
                        w,
                        atoms.enter,
                        [win, version << 24, atoms.uri_list, x11rb::NONE, x11rb::NONE],
                    );
                    target = Some(w);
                }
            }
            if let Some(t) = target {
                send_client(
                    &conn,
                    t,
                    atoms.position,
                    [
                        win,
                        0,
                        ((px as u32) << 16) | (py as u32 & 0xffff),
                        CURRENT_TIME,
                        atoms.action_copy,
                    ],
                );
            }
            last_pos = (px, py);
        }
        if let Some(icon) = &icon {
            if (px, py) != last_icon_pos {
                last_icon_pos = (px, py);
                icon.follow(&conn, px, py);
            }
        }
        let _ = conn.flush();
        // Events (XdndFinished, SelectionRequest) wake the loop early;
        // the timeout keeps the 16ms pointer cadence.
        wait_event_or(&conn, 16);
    }

    if let Some(icon) = &icon {
        icon.destroy(&conn);
    }
    let _ = conn.destroy_window(win);
    let _ = conn.flush();
}

/// Answers a selection conversion for the in-flight XDnD payload.
fn serve_drag_request<C: Connection>(
    conn: &C,
    atoms: &XdndAtoms,
    uri_list: &str,
    ev: &SelectionRequestEvent,
) {
    let property = if ev.property == x11rb::NONE {
        ev.target
    } else {
        ev.property
    };
    let satisfied = if ev.target == atoms.targets {
        conn.change_property32(
            PropMode::REPLACE,
            ev.requestor,
            property,
            AtomEnum::ATOM,
            &[atoms.targets, atoms.uri_list, atoms.utf8],
        )
        .is_ok()
    } else if ev.target == atoms.uri_list {
        conn.change_property8(
            PropMode::REPLACE,
            ev.requestor,
            property,
            atoms.uri_list,
            uri_list.as_bytes(),
        )
        .is_ok()
    } else if ev.target == atoms.utf8 {
        conn.change_property8(
            PropMode::REPLACE,
            ev.requestor,
            property,
            atoms.utf8,
            uri_list.as_bytes(),
        )
        .is_ok()
    } else {
        false
    };
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
