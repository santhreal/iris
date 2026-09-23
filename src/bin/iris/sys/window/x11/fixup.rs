use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

/// A post-map fixup the dispatcher applies once the window's XID
/// exists. Each variant carries the geometry/state it asserts; the
/// reassert schedule is per-variant (a Move re-places for a second
/// against WM re-framing, a Span re-asserts once).
enum Fixup {
    Move { x: i32, y: i32, notification: bool },
    AlwaysOnTop,
    Span { x: i32, y: i32, w: u32, h: u32 },
    Unpark { x: i32, y: i32, w: u32, h: u32 },
}

/// One queued fixup: the WM_CLASS substring to match and the work to
/// apply once its window appears in _NET_CLIENT_LIST.
struct Pending {
    class: String,
    fixup: Fixup,
    deadline: std::time::Instant,
}

/// The pending queue and the dispatcher's self-pipe write end. The
/// dispatcher resolves every queued fixup against a single
/// _NET_CLIENT_LIST scan per change, replacing the old design where
/// each helper spawned a thread that re-scanned the client list every
/// 15-50ms (a round trip per scan per window).
static PENDING: parking_lot::Mutex<Vec<Pending>> = parking_lot::Mutex::new(Vec::new());
static WAKE_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Queue a fixup and wake the dispatcher. No-op under Wayland.
fn enqueue_fixup(class: String, fixup: Fixup) {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return;
    }
    dispatcher();
    PENDING.lock().push(Pending {
        class,
        fixup,
        deadline: std::time::Instant::now() + std::time::Duration::from_secs(6),
    });
    let fd = WAKE_FD.load(std::sync::atomic::Ordering::Acquire);
    if fd >= 0 {
        unsafe { libc::write(fd, &1u8 as *const u8 as *const libc::c_void, 1) };
    }
}

/// Spawn the single dispatcher thread on first use. It owns a
/// dedicated connection (separate from `shared_conn`, which
/// `begin_wm_move` reads events on) and selects PropertyChange on the
/// root so _NET_CLIENT_LIST updates arrive as events.
fn dispatcher() {
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(|| {
        let mut fds = [0i32; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return;
        }
        WAKE_FD.store(fds[1], std::sync::atomic::Ordering::Release);
        std::thread::spawn(move || dispatcher_loop(fds[0]));
    });
}

/// The dispatcher loop: block on the X fd and the wake pipe, resolve
/// pending fixups when the client list changes (or the 50ms scan cap
/// elapses, covering a missed event), and fire scheduled reasserts.
fn dispatcher_loop(wake_fd: i32) {
    use std::os::unix::io::AsRawFd;
    use std::time::{Duration, Instant};
    use x11rb::protocol::xproto::{ChangeWindowAttributesAux, EventMask};
    let Some((conn, _)) = x11rb::connect(None).ok() else {
        return;
    };
    let root = conn.setup().roots[0].root;
    let aux = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
    let _ = conn.change_window_attributes(root, &aux);
    let _ = conn.flush();
    let client_list = atom_cached(&conn, b"_NET_CLIENT_LIST");
    let x_fd = conn.stream().as_raw_fd();
    // (fire_at, xid, fixup, remaining reasserts)
    let mut reasserts: Vec<(Instant, u32, Fixup, u8)> = Vec::new();
    let mut last_scan = Instant::now() - Duration::from_secs(1);
    loop {
        // Drain queued X events; note a client-list change.
        let mut changed = false;
        loop {
            match conn.poll_for_event() {
                Ok(Some(x11rb::protocol::Event::PropertyNotify(ev))) => {
                    if Some(ev.atom) == client_list {
                        changed = true;
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => return,
            }
        }
        let now = Instant::now();
        let has_pending = !PENDING.lock().is_empty();
        if has_pending && (changed || last_scan.elapsed() >= Duration::from_millis(50)) {
            last_scan = now;
            scan_and_resolve(&conn, &mut reasserts);
        }
        // Fire due reasserts; reschedule the ones with repeats left.
        let mut i = 0;
        while i < reasserts.len() {
            if reasserts[i].0 <= now {
                let (_, xid, fixup, remaining) = reasserts.remove(i);
                apply_fixup(&conn, xid, &fixup, false);
                if remaining > 1 {
                    reasserts.push((now + Duration::from_millis(100), xid, fixup, remaining - 1));
                }
            } else {
                i += 1;
            }
        }
        // Drop fixups whose window never mapped.
        PENDING.lock().retain(|p| p.deadline > now);
        // Next wake: earliest reassert, else the 50ms scan cap while a
        // fixup is pending, else block until a new fixup or X event.
        let mut timeout = -1i32;
        if !PENDING.lock().is_empty() {
            timeout = 50;
        }
        if let Some(t) = reasserts.iter().map(|r| r.0).min() {
            let ms = t.saturating_duration_since(Instant::now()).as_millis() as i32;
            timeout = if timeout < 0 { ms } else { timeout.min(ms) };
        }
        let mut pfds = [
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
        if unsafe { libc::poll(pfds.as_mut_ptr(), 2, timeout) } <= 0 {
            continue;
        }
        if pfds[1].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            unsafe { libc::read(wake_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        }
    }
}

/// One _NET_CLIENT_LIST scan resolving every pending fixup: read the
/// list once, batch the WM_CLASS reads, and match each pending class
/// against the newest (last in map order) window carrying it.
fn scan_and_resolve(
    conn: &x11rb::rust_connection::RustConnection,
    reasserts: &mut Vec<(std::time::Instant, u32, Fixup, u8)>,
) {
    let root = conn.setup().roots[0].root;
    let Some(client_list) = atom_cached(conn, b"_NET_CLIENT_LIST") else {
        return;
    };
    let Ok(windows) = conn
        .get_property(false, root, client_list, AtomEnum::WINDOW, 0, 1024)
        .map(|c| c.reply())
    else {
        return;
    };
    let Ok(windows) = windows else { return };
    let pending_cookies: Vec<(u32, _)> = windows
        .value
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .filter_map(|win| {
            conn.get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
                .ok()
                .map(|cookie| (win, cookie))
        })
        .collect();
    let classes: Vec<(u32, Vec<u8>)> = pending_cookies
        .into_iter()
        .filter_map(|(win, c)| c.reply().ok().map(|r| (win, r.value)))
        .collect();
    let mut pend = PENDING.lock();
    let mut i = 0;
    while i < pend.len() {
        let class = pend[i].class.as_bytes();
        let newest = classes
            .iter()
            .filter(|(_, c)| c.windows(class.len()).any(|w| w == class))
            .map(|(w, _)| *w)
            .next_back();
        if let Some(xid) = newest {
            let p = pend.remove(i);
            apply_fixup(conn, xid, &p.fixup, true);
            schedule_reassert(reasserts, xid, p.fixup);
        } else {
            i += 1;
        }
    }
}

/// Apply a resolved fixup. `initial` runs the one-time setup
/// (decorations, window type, map, focus); a reassert only re-runs
/// the geometry/state the WM may have overridden.
fn apply_fixup(
    conn: &x11rb::rust_connection::RustConnection,
    xid: u32,
    fixup: &Fixup,
    initial: bool,
) {
    use x11rb::protocol::xproto::{ConfigureWindowAux, StackMode};
    match fixup {
        Fixup::Move { x, y, notification } => {
            if initial {
                if *notification {
                    set_window_type_notification_on(conn, xid);
                }
                suppress_decorations_on(conn, xid);
            }
            let aux = ConfigureWindowAux::new().x(*x).y(*y);
            let _ = conn.configure_window(xid, &aux);
            let _ = conn.flush();
        }
        Fixup::AlwaysOnTop => {
            if initial {
                suppress_decorations_on(conn, xid);
                request_state_on(conn, xid, b"_NET_WM_STATE_ABOVE");
            }
        }
        Fixup::Span { x, y, w, h } => {
            if initial {
                suppress_decorations_on(conn, xid);
            }
            let aux = ConfigureWindowAux::new()
                .x(*x)
                .y(*y)
                .width(*w)
                .height(*h)
                .stack_mode(StackMode::ABOVE);
            let _ = conn.configure_window(xid, &aux);
            let _ = conn.flush();
            request_state_on(conn, xid, b"_NET_WM_STATE_ABOVE");
        }
        Fixup::Unpark { x, y, w, h } => {
            if initial {
                let aux = ConfigureWindowAux::new()
                    .x(*x)
                    .y(*y)
                    .width(*w)
                    .height(*h)
                    .stack_mode(StackMode::ABOVE);
                let _ = conn.configure_window(xid, &aux);
                let _ = conn.map_window(xid);
                let _ = conn.flush();
                request_state_on(conn, xid, b"_NET_WM_STATE_ABOVE");
                // Activate through the WM (_NET_ACTIVE_WINDOW), not a
                // bare set_input_focus: a reparenting WM applies its own
                // focus when it processes the map, and a raw grab that
                // lands first loses the race and the overlay's first
                // keystrokes go to the root window.
                activate_window_on(conn, xid);
                let _ = conn.set_input_focus(
                    x11rb::protocol::xproto::InputFocus::POINTER_ROOT,
                    xid,
                    x11rb::CURRENT_TIME,
                );
                let _ = conn.flush();
            }
        }
    }
}

/// Queue a fixup's reassert schedule: a Move re-places ten times over
/// a second (the WM's re-framing re-applies its own placement), a
/// Span re-asserts once after the re-frame settles.
fn schedule_reassert(
    reasserts: &mut Vec<(std::time::Instant, u32, Fixup, u8)>,
    xid: u32,
    fixup: Fixup,
) {
    let now = std::time::Instant::now();
    match fixup {
        Fixup::Move { .. } => {
            reasserts.push((now + std::time::Duration::from_millis(100), xid, fixup, 10))
        }
        Fixup::Span { .. } => {
            reasserts.push((now + std::time::Duration::from_millis(200), xid, fixup, 1))
        }
        _ => {}
    }
}

/// Post-map placement fixup: WMs like openbox apply their own
/// placement at map time and draw decorations on borderless GPUI
/// windows. Finds the window by WM_CLASS substring, suppresses
/// decorations, and puts it at (`x`, `y`). No-op under Wayland and
/// once the WM honors the requested origin (mutter, KWin).
pub fn place_after_map(class: String, x: f32, y: f32) {
    place_after_map_kind(class, x, y, false);
}

/// `notification` marks the window _NET_WM_WINDOW_TYPE_NOTIFICATION:
/// openbox then neither decorates it nor applies its anti-overlap
/// cascade, which is what throws a replacement toast out of the
/// corner while its predecessor is still fading there.
pub fn place_after_map_kind(class: String, x: f32, y: f32, notification: bool) {
    enqueue_fixup(
        class,
        Fixup::Move {
            x: x as i32,
            y: y as i32,
            notification,
        },
    );
}

/// Interned atoms shared across every post-map fixup: intern_atom is
/// a round trip, and the same handful of EWMH names is resolved on
/// every window that maps.
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

/// _NET_WM_STATE += the named state via a client message.
fn request_state_on(conn: &impl Connection, win: u32, atom_name: &'static [u8]) {
    let Some(state) = atom_cached(conn, b"_NET_WM_STATE") else {
        return;
    };
    let Some(property) = atom_cached(conn, atom_name) else {
        return;
    };
    let root = conn.setup().roots[0].root;
    let event = x11rb::protocol::xproto::ClientMessageEvent {
        response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window: win,
        type_: state,
        data: x11rb::protocol::xproto::ClientMessageData::from([
            1, // _NET_WM_STATE_ADD
            property, 0, 1, // source: application
            0,
        ]),
    };
    let mask = x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY
        | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
    let _ = conn.flush();
}

/// _NET_ACTIVE_WINDOW client message: the EWMH activation request a
/// reparenting WM honors. Falls back to nothing under a WM that does
/// not read it; the caller still sets input focus directly.
fn activate_window_on(conn: &impl Connection, win: u32) {
    let Some(active) = atom_cached(conn, b"_NET_ACTIVE_WINDOW") else {
        return;
    };
    let root = conn.setup().roots[0].root;
    let event = x11rb::protocol::xproto::ClientMessageEvent {
        response_type: x11rb::protocol::xproto::CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window: win,
        type_: active,
        data: x11rb::protocol::xproto::ClientMessageData::from([
            1, // source: application
            x11rb::CURRENT_TIME,
            0,
            0,
            0,
        ]),
    };
    let mask = x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_NOTIFY
        | x11rb::protocol::xproto::EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
}

/// Pin a window above everything: _NET_WM_STATE_ABOVE via a client
/// message once the window's XID exists. Used by the pin-to-screen
/// reference window.
pub fn always_on_top_after_map(class: String) {
    enqueue_fixup(class, Fixup::AlwaysOnTop);
}

/// Span the virtual screen with one window: explicit placement at the
/// union rect plus _NET_WM_STATE_ABOVE. _NET_WM_STATE_FULLSCREEN pins
/// a window to a single monitor, so the overlay does not use it.
pub fn span_after_map(class: String, x: i32, y: i32, w: u32, h: u32) {
    enqueue_fixup(class, Fixup::Span { x, y, w, h });
}

pub fn set_window_type_notification_on(conn: &impl Connection, win: u32) {
    let (Some(ty), Some(notif)) = (
        atom_cached(conn, b"_NET_WM_WINDOW_TYPE"),
        atom_cached(conn, b"_NET_WM_WINDOW_TYPE_NOTIFICATION"),
    ) else {
        return;
    };
    let _ = conn.change_property32(
        x11rb::protocol::xproto::PropMode::REPLACE,
        win,
        ty,
        AtomEnum::ATOM,
        &[notif],
    );
    let _ = conn.flush();
}

/// Bring a parked window back over the virtual screen: configure the
/// union rect, raise, focus. The window was never unmapped, so this
/// skips renderer init entirely; one XCB round trip of latency.
pub fn unpark_span(class: String, x: i32, y: i32, w: u32, h: u32) {
    enqueue_fixup(class, Fixup::Unpark { x, y, w, h });
}

/// _MOTIF_WM_HINTS decorations=0: GPUI falls back to server-side
/// decorations when no compositor is present, and WMs like openbox
/// then titlebar even notification windows. Set the hint directly.
pub fn suppress_decorations_on(conn: &impl Connection, win: u32) {
    let Some(motif) = atom_cached(conn, b"_MOTIF_WM_HINTS") else {
        return;
    };
    // flags=2 (decorations), functions=0, decorations=0, input=0, status=0
    let hints: [u32; 5] = [2, 0, 0, 0, 0];
    let _ = conn.change_property32(
        x11rb::protocol::xproto::PropMode::REPLACE,
        win,
        motif,
        motif,
        &hints,
    );
    let _ = conn.flush();
}
