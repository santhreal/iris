//! Window manager state for mapped windows: stacking above other
//! windows, a span over the virtual screen, and a place outside taskbars
//! and window switchers; and withdrawal for a window taken off the
//! screen. A window manager applies a _NET_WM_STATE request only to a
//! window it manages, so a fixup waits on one thread until its window is
//! in _NET_CLIENT_LIST, and the same thread withdraws a parked window
//! the list holds again.

use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageData, ClientMessageEvent, ConfigureWindowAux,
    ConnectionExt, EventMask, InputFocus, PropMode, StackMode, UnmapNotifyEvent,
    CLIENT_MESSAGE_EVENT, UNMAP_NOTIFY_EVENT,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

#[cfg(test)]
mod tests;

/// What a fixup asserts on its window.
#[derive(Clone, Copy)]
enum Fixup {
    /// _NET_WM_STATE_ABOVE.
    AlwaysOnTop,
    /// The union rect of the monitors in root pixels, raised, above.
    Span { x: i32, y: i32, w: u32, h: u32 },
    /// _NET_WM_STATE_SKIP_TASKBAR and _NET_WM_STATE_SKIP_PAGER: no entry
    /// in a taskbar, a window switcher, or an overview.
    Unlisted,
}

/// A fixup waiting for its window to be managed.
struct Pending {
    xid: u32,
    fixup: Fixup,
    deadline: Instant,
}

/// How long a fixup waits for its window to be managed.
const PATIENCE: Duration = Duration::from_secs(6);

/// The longest the dispatcher goes without reading the client list
/// while a fixup waits: the bound on a missed PropertyNotify.
const RESCAN: Duration = Duration::from_millis(50);

/// When a span is asserted again: a window manager that reframes the
/// window after managing it applies its own geometry once more.
const SPAN_AGAIN: Duration = Duration::from_millis(200);

static PENDING: parking_lot::Mutex<Vec<Pending>> = parking_lot::Mutex::new(Vec::new());
/// The spans' second assertions: when each is due, its window, its span.
static AGAIN: parking_lot::Mutex<Vec<(Instant, u32, Fixup)>> = parking_lot::Mutex::new(Vec::new());
/// Held while the dispatcher sends fixups, so `forget` returns only
/// once no request of a forgotten fixup is still to be sent.
static SENDING: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
/// Windows withdrawn by `withdraw` and not brought back by
/// `unpark_span`.
static PARKED: parking_lot::Mutex<Vec<u32>> = parking_lot::Mutex::new(Vec::new());
/// The write end of the dispatcher's wake pipe, -1 until it runs.
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

/// Keep window `xid` above other windows once it is managed.
pub fn always_on_top_after_map(xid: u32) {
    enqueue(xid, Fixup::AlwaysOnTop);
}

/// Span the virtual screen with window `xid` once it is managed: the
/// union rect (`x`, `y`, `w`, `h`, root pixels), raised and above, with
/// no window manager frame, and with no entry in a taskbar or a window
/// switcher. The frame goes at once, from the calling thread; the rest
/// waits until the window is managed.
pub fn span_after_map(xid: u32, x: i32, y: i32, w: u32, h: u32) {
    if let Ok((conn, _)) = iris_lib::sys::capture::x11::shared_conn() {
        unframe(conn, xid);
        let _ = conn.flush();
    }
    enqueue(xid, Fixup::Unlisted);
    enqueue(xid, Fixup::Span { x, y, w, h });
}

/// Withdraw window `xid` (ICCCM 4.1.4): unmapped, and a synthetic
/// UnmapNotify on the root for a window manager. A window manager stops
/// managing a withdrawn window, so the window has no frame and no entry
/// in a taskbar, a window switcher, or an overview, where a minimized
/// window keeps all three. With no window manager the unmap alone takes
/// it off the screen. A window manager that starts as the window is
/// withdrawn can miss both and manage it anyway, so the dispatcher
/// withdraws the window again each time the client list holds it, until
/// `unpark_span`.
pub fn withdraw(xid: u32) {
    let Ok((conn, screen)) = iris_lib::sys::capture::x11::shared_conn() else {
        return;
    };
    dispatcher();
    {
        let mut parked = PARKED.lock();
        if !parked.contains(&xid) {
            parked.push(xid);
        }
    }
    send_withdraw(conn, conn.setup().roots[screen].root, xid);
    let _ = conn.flush();
}

/// Send the requests that withdraw `win`: its unmap, and the synthetic
/// UnmapNotify a window manager reads for a window it has a map request
/// for and has not mapped yet. Unflushed.
fn send_withdraw(conn: &impl Connection, root: u32, win: u32) {
    let _ = conn.unmap_window(win);
    let event = UnmapNotifyEvent {
        response_type: UNMAP_NOTIFY_EVENT,
        sequence: 0,
        event: root,
        window: win,
        from_configure: false,
    };
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
}

/// Drop every fixup still due on window `xid`, waiting or to be asserted
/// again. A withdrawn window takes no more window manager requests: a
/// window manager shows a window it is asked to raise. On return the
/// dispatcher sends no further request for `xid`.
pub fn forget(xid: u32) {
    let _sending = SENDING.lock();
    PENDING.lock().retain(|p| p.xid != xid);
    AGAIN.lock().retain(|a| a.1 != xid);
}

/// Bring the withdrawn (`withdraw`) window `xid` back over the virtual
/// screen: the union rect, mapped, raised, above, focused, and taking
/// input again (`cutout::pass_input`). A withdrawn window takes its
/// window manager states from its `_NET_WM_STATE` property, so the
/// property lists them before the map; the requests go out at once,
/// from the calling thread, and the span is asserted again once a window
/// manager manages the window (`span_after_map`).
pub fn unpark_span(xid: u32, x: i32, y: i32, w: u32, h: u32) {
    let Ok((conn, screen)) = iris_lib::sys::capture::x11::shared_conn() else {
        return;
    };
    let root = conn.setup().roots[screen].root;
    {
        // Under `SENDING`: once the window is mapped, no withdrawal of
        // the dispatcher's is still to be sent for it.
        let _sending = SENDING.lock();
        PARKED.lock().retain(|&w| w != xid);
    }
    super::cutout::take_input(conn, xid);
    let states = [
        atom_cached(conn, b"_NET_WM_STATE"),
        atom_cached(conn, b"_NET_WM_STATE_ABOVE"),
        atom_cached(conn, b"_NET_WM_STATE_SKIP_TASKBAR"),
        atom_cached(conn, b"_NET_WM_STATE_SKIP_PAGER"),
    ];
    if let [Some(state), Some(above), Some(taskbar), Some(pager)] = states {
        let _ = conn.change_property32(
            PropMode::REPLACE,
            xid,
            state,
            AtomEnum::ATOM,
            &[above, taskbar, pager],
        );
    }
    let _ = conn.configure_window(xid, &span(x, y, w, h));
    let _ = conn.map_window(xid);
    request_above(conn, root, xid);
    // Activate through the window manager (_NET_ACTIVE_WINDOW) as well
    // as directly: a reparenting window manager applies its own focus
    // when it processes the map, and a bare focus request that lands
    // first loses to it, sending the first keystrokes to the root.
    activate(conn, root, xid);
    let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, xid, x11rb::CURRENT_TIME);
    let _ = conn.flush();
    if managed_screen(conn, root) {
        enqueue(xid, Fixup::Unlisted);
        enqueue(xid, Fixup::Span { x, y, w, h });
    }
}

/// Whether a window manager runs on the screen of `root`: the one client
/// that selects SubstructureRedirect on the root.
fn managed_screen(conn: &impl Connection, root: u32) -> bool {
    conn.get_window_attributes(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_none_or(|a| a.all_event_masks.contains(EventMask::SUBSTRUCTURE_REDIRECT))
}

/// Take the override-redirect window `xid` off the screen: shrunk to
/// one pixel, so its renderer holds one-pixel buffers while it waits,
/// and unmapped. No window manager acts on such a window, so the
/// requests go out at once. `show_span` brings it back.
pub fn stow(xid: u32) {
    let Ok((conn, _)) = iris_lib::sys::capture::x11::shared_conn() else {
        return;
    };
    let _ = conn.configure_window(xid, &ConfigureWindowAux::new().width(1).height(1));
    let _ = conn.unmap_window(xid);
    let _ = conn.flush();
}

/// Map the override-redirect window `xid` over the union rect (root
/// pixels), above every other window, leaving the input focus and the
/// window's input shape as they are.
pub fn show_span(xid: u32, x: i32, y: i32, w: u32, h: u32) {
    let Ok((conn, _)) = iris_lib::sys::capture::x11::shared_conn() else {
        return;
    };
    let _ = conn.configure_window(xid, &span(x, y, w, h));
    let _ = conn.map_window(xid);
    let _ = conn.flush();
}

/// Queue a fixup and wake the dispatcher.
fn enqueue(xid: u32, fixup: Fixup) {
    dispatcher();
    PENDING.lock().push(Pending {
        xid,
        fixup,
        deadline: Instant::now() + PATIENCE,
    });
    let fd = WAKE_FD.load(Ordering::Acquire);
    if fd >= 0 {
        // SAFETY: `fd` is the pipe's write end, open for the process's
        // life; the byte is a wake, its value unread.
        unsafe { libc::write(fd, (&1u8 as *const u8).cast(), 1) };
    }
}

/// Start the dispatcher thread on first use. It has a connection of its
/// own, whose event queue no other reader drains, with PropertyChange
/// selected on the root: a _NET_CLIENT_LIST update wakes it.
fn dispatcher() {
    static START: Once = Once::new();
    START.call_once(|| {
        let mut fds = [0i32; 2];
        // SAFETY: `fds` has room for the two descriptors pipe writes.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return;
        }
        WAKE_FD.store(fds[1], Ordering::Release);
        std::thread::spawn(move || dispatcher_loop(fds[0]));
    });
}

/// Sleep on the X connection and the wake pipe; apply each fixup whose
/// window the client list holds, and the spans' second assertions.
fn dispatcher_loop(wake_fd: i32) {
    let Ok((conn, screen)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots[screen].root;
    let aux = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
    let _ = conn.change_window_attributes(root, &aux);
    let _ = conn.flush();
    let client_list = atom_cached(&conn, b"_NET_CLIENT_LIST");
    let x_fd = conn.stream().as_raw_fd();
    let mut scanned = Instant::now() - RESCAN;
    loop {
        let mut changed = false;
        loop {
            match conn.poll_for_event() {
                Ok(Some(x11rb::protocol::Event::PropertyNotify(ev))) => {
                    changed |= Some(ev.atom) == client_list;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => return,
            }
        }
        let now = Instant::now();
        {
            let _sending = SENDING.lock();
            let waiting = !PENDING.lock().is_empty();
            let parked = !PARKED.lock().is_empty();
            if (waiting && (changed || scanned.elapsed() >= RESCAN)) || (parked && changed) {
                scanned = now;
                let listed = client_windows(&conn, root, client_list);
                for (xid, fixup) in managed(&listed) {
                    apply(&conn, root, xid, fixup);
                    if let Fixup::Span { .. } = fixup {
                        AGAIN.lock().push((now + SPAN_AGAIN, xid, fixup));
                    }
                }
                for &xid in PARKED.lock().iter().filter(|x| listed.contains(x)) {
                    send_withdraw(&conn, root, xid);
                }
            }
            AGAIN.lock().retain(|&(at, xid, fixup)| {
                let due = at <= now;
                if due {
                    apply(&conn, root, xid, fixup);
                }
                !due
            });
            let _ = conn.flush();
        }
        PENDING.lock().retain(|p| p.deadline > now);

        // Sleep until the next second assertion, the rescan bound while
        // a fixup waits, a queued fixup, or an X event.
        let mut timeout = if PENDING.lock().is_empty() {
            None
        } else {
            Some(RESCAN)
        };
        if let Some(at) = AGAIN.lock().iter().map(|a| a.0).min() {
            let left = at.saturating_duration_since(Instant::now());
            timeout = Some(timeout.map_or(left, |t| t.min(left)));
        }
        let ms = timeout.map_or(-1, |t| t.as_millis().min(i32::MAX as u128) as i32);
        let mut fds = [
            libc::pollfd {
                fd: x_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: `fds` is two valid pollfds for the duration of the call.
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, ms) } <= 0 {
            continue;
        }
        if fds[1].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            // SAFETY: `buf` is writable for its length; the bytes are wakes.
            unsafe { libc::read(wake_fd, buf.as_mut_ptr().cast(), buf.len()) };
        }
    }
}

/// The windows the client list holds; none when it cannot be read.
fn client_windows(conn: &RustConnection, root: u32, client_list: Option<u32>) -> Vec<u32> {
    let Some(client_list) = client_list else {
        return Vec::new();
    };
    conn.get_property(false, root, client_list, AtomEnum::WINDOW, 0, 4096)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|list| list.value32().map(Iterator::collect))
        .unwrap_or_default()
}

/// Take the pending fixups whose windows `listed` holds.
fn managed(listed: &[u32]) -> Vec<(u32, Fixup)> {
    let mut ready = Vec::new();
    PENDING.lock().retain(|p| {
        let managed = listed.contains(&p.xid);
        if managed {
            ready.push((p.xid, p.fixup));
        }
        !managed
    });
    ready
}

/// Send `fixup`'s requests for window `xid`, unflushed.
fn apply(conn: &RustConnection, root: u32, xid: u32, fixup: Fixup) {
    match fixup {
        Fixup::AlwaysOnTop => request_above(conn, root, xid),
        Fixup::Span { x, y, w, h } => {
            let _ = conn.configure_window(xid, &span(x, y, w, h));
            request_above(conn, root, xid);
        }
        Fixup::Unlisted => {
            if let (Some(state), Some(taskbar), Some(pager)) = (
                atom_cached(conn, b"_NET_WM_STATE"),
                atom_cached(conn, b"_NET_WM_STATE_SKIP_TASKBAR"),
                atom_cached(conn, b"_NET_WM_STATE_SKIP_PAGER"),
            ) {
                // _NET_WM_STATE_ADD, two states, source: application.
                send_to_wm(conn, root, xid, state, [1, taskbar, pager, 1, 0]);
            }
        }
    }
}

/// The union rect, raised.
fn span(x: i32, y: i32, w: u32, h: u32) -> ConfigureWindowAux {
    ConfigureWindowAux::new()
        .x(x)
        .y(y)
        .width(w)
        .height(h)
        .stack_mode(StackMode::ABOVE)
}

/// Interned atoms shared by every fixup: intern_atom is a round trip,
/// and the same few EWMH names serve every window.
pub(super) fn atom_cached(conn: &impl Connection, name: &'static [u8]) -> Option<u32> {
    static CACHE: std::sync::LazyLock<
        parking_lot::Mutex<std::collections::HashMap<&'static [u8], u32>>,
    > = std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));
    if let Some(atom) = CACHE.lock().get(name) {
        return Some(*atom);
    }
    let atom = conn.intern_atom(false, name).ok()?.reply().ok()?.atom;
    CACHE.lock().insert(name, atom);
    Some(atom)
}

/// _NET_WM_STATE += _NET_WM_STATE_ABOVE on `win`.
fn request_above(conn: &impl Connection, root: u32, win: u32) {
    if let (Some(state), Some(above)) = (
        atom_cached(conn, b"_NET_WM_STATE"),
        atom_cached(conn, b"_NET_WM_STATE_ABOVE"),
    ) {
        // _NET_WM_STATE_ADD, one state, source: application.
        send_to_wm(conn, root, win, state, [1, above, 0, 1, 0]);
    }
}

/// _MOTIF_WM_HINTS decorations 0 on `win`: no window manager frame. GPUI
/// asks for a frame for a window that opens while no compositor runs,
/// and a frame would move a span's client area off the union rect, so a
/// point on the window would not be the same point on the screen. A
/// window manager reads the hints as it manages the window, and drops a
/// frame it drew when they change. Unflushed.
fn unframe(conn: &impl Connection, win: u32) {
    if let Some(hints) = atom_cached(conn, b"_MOTIF_WM_HINTS") {
        // Flags: decorations; functions; decorations: none; input mode;
        // status.
        let _ = conn.change_property32(PropMode::REPLACE, win, hints, hints, &[2, 0, 0, 0, 0]);
    }
}

/// _NET_ACTIVE_WINDOW: the activation request a reparenting window
/// manager honors.
fn activate(conn: &impl Connection, root: u32, win: u32) {
    if let Some(active) = atom_cached(conn, b"_NET_ACTIVE_WINDOW") {
        // Source: application, no timestamp, no current active window.
        send_to_wm(conn, root, win, active, [1, x11rb::CURRENT_TIME, 0, 0, 0]);
    }
}

/// Send the client message `type_` about `win` to the window manager,
/// on the root window, as EWMH asks. Unflushed.
fn send_to_wm(conn: &impl Connection, root: u32, win: u32, type_: u32, data: [u32; 5]) {
    let event = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window: win,
        type_,
        data: ClientMessageData::from(data),
    };
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
}
