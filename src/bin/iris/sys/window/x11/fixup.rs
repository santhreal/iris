//! Window manager state for mapped windows: stacking above other
//! windows, and a span over the virtual screen. A window manager applies
//! a _NET_WM_STATE request only to a window it manages, so a fixup
//! waits on one thread until its window is in _NET_CLIENT_LIST.

use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageData, ClientMessageEvent, ConfigureWindowAux,
    ConnectionExt, EventMask, InputFocus, StackMode, CLIENT_MESSAGE_EVENT,
};
use x11rb::rust_connection::RustConnection;

/// What a fixup asserts on its window.
#[derive(Clone, Copy)]
enum Fixup {
    /// _NET_WM_STATE_ABOVE.
    AlwaysOnTop,
    /// The union rect of the monitors in root pixels, raised, above.
    Span { x: i32, y: i32, w: u32, h: u32 },
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
/// The write end of the dispatcher's wake pipe, -1 until it runs.
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

/// Keep window `xid` above other windows once it is managed.
pub fn always_on_top_after_map(xid: u32) {
    enqueue(xid, Fixup::AlwaysOnTop);
}

/// Span the virtual screen with window `xid` once it is managed: the
/// union rect (`x`, `y`, `w`, `h`, root pixels), raised and above.
pub fn span_after_map(xid: u32, x: i32, y: i32, w: u32, h: u32) {
    enqueue(xid, Fixup::Span { x, y, w, h });
}

/// Bring the parked window `xid` back over the virtual screen: the
/// union rect, mapped, raised, above, focused. The window stays
/// managed while parked, so the requests go out at once, from the
/// calling thread.
pub fn unpark_span(xid: u32, x: i32, y: i32, w: u32, h: u32) {
    let Ok((conn, screen)) = iris_lib::capture::x11::shared_conn() else {
        return;
    };
    let root = conn.setup().roots[screen].root;
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
    let mut again: Vec<(Instant, u32, Fixup)> = Vec::new();
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
        let waiting = !PENDING.lock().is_empty();
        if waiting && (changed || scanned.elapsed() >= RESCAN) {
            scanned = now;
            for (xid, fixup) in managed(&conn, root, client_list) {
                apply(&conn, root, xid, fixup);
                if let Fixup::Span { .. } = fixup {
                    again.push((now + SPAN_AGAIN, xid, fixup));
                }
            }
        }
        again.retain(|&(at, xid, fixup)| {
            let due = at <= now;
            if due {
                apply(&conn, root, xid, fixup);
            }
            !due
        });
        let _ = conn.flush();
        PENDING.lock().retain(|p| p.deadline > now);

        // Sleep until the next second assertion, the rescan bound while
        // a fixup waits, a queued fixup, or an X event.
        let mut timeout = if PENDING.lock().is_empty() {
            None
        } else {
            Some(RESCAN)
        };
        if let Some(at) = again.iter().map(|a| a.0).min() {
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

/// Take the pending fixups whose windows the client list holds.
fn managed(conn: &RustConnection, root: u32, client_list: Option<u32>) -> Vec<(u32, Fixup)> {
    let Some(client_list) = client_list else {
        return Vec::new();
    };
    let Some(list) = conn
        .get_property(false, root, client_list, AtomEnum::WINDOW, 0, 4096)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return Vec::new();
    };
    let Some(windows) = list.value32() else {
        return Vec::new();
    };
    let windows: Vec<u32> = windows.collect();
    let mut ready = Vec::new();
    PENDING.lock().retain(|p| {
        let managed = windows.contains(&p.xid);
        if managed {
            ready.push((p.xid, p.fixup));
        }
        !managed
    });
    ready
}

/// Send `fixup`'s requests for window `xid`, unflushed.
fn apply(conn: &RustConnection, root: u32, xid: u32, fixup: Fixup) {
    if let Fixup::Span { x, y, w, h } = fixup {
        let _ = conn.configure_window(xid, &span(x, y, w, h));
    }
    request_above(conn, root, xid);
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
