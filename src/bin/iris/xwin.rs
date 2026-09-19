//! X11 window helpers shared by the native surfaces: WM_CLASS-based
//! XID lookup (GPUI's X11 HasWindowHandle is unimplemented) and direct
//! window moves for positions a window manager would otherwise
//! override at map time.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

/// The primary monitor's rect in root pixels from randr (primary
/// first in `monitors()`), falling back to GPUI's primary display.
/// GPUI's X11 primary_display() can span the whole virtual screen
/// on multi-monitor setups, which centers windows on no monitor at
/// all; randr reports the real per-monitor geometry.
pub fn primary_monitor_rect(cx: &gpui::App) -> Option<(f32, f32, f32, f32)> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        if let Ok(monitors) = iris_lib::capture::x11::monitors() {
            if let Some(m) = monitors.first() {
                return Some((m.x as f32, m.y as f32, m.width as f32, m.height as f32));
            }
        }
    }
    cx.primary_display().map(|d| {
        let b = d.bounds();
        (
            b.origin.x.into(),
            b.origin.y.into(),
            b.size.width.into(),
            b.size.height.into(),
        )
    })
}

/// Window origin that centers a `w`x`h` window on the primary display,
/// or `fallback` when no display information is available.
pub fn centered_origin(
    cx: &gpui::App,
    w: f32,
    h: f32,
    fallback: (f32, f32),
) -> (f32, f32) {
    let Some((bx, by, sw, sh)) = primary_monitor_rect(cx) else {
        return fallback;
    };
    (bx + (sw - w) / 2.0, by + (sh - h) / 2.0)
}

/// A process-unique window class: surfaces that can have several
/// live instances (toasts replacing each other, editors) must be
/// distinguishable for the post-map fixup to find THE window it
/// belongs to, not whichever sibling happens to be newest.
pub fn unique_id(base: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{base}.{:x}.{}", nanos, SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
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
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        std::thread::spawn(move || move_to(&class, x as i32, y as i32, notification));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (class, x, y, notification);
}

/// Fullscreen on a specific monitor. GPUI's X11 backend has no
/// creation-time fullscreen: its render-time toggle races the WM's
/// own placement, so a second monitor's window can end up
/// fullscreened onto the primary. This owns the ordering instead:
/// place the window at the monitor's origin (defeating WM
/// placement), let the re-frame settle, then request
/// _NET_WM_STATE_FULLSCREEN, which the WM applies to the monitor
/// the window is on.
pub fn fullscreen_after_map(class: String, x: f32, y: f32) {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        std::thread::spawn(move || {
            let Ok((conn, _)) = x11rb::connect(None) else {
                return;
            };
            for _ in 0..120 {
                if let Some(xid) = find_xid_by_class_on(&conn, &class) {
                    suppress_decorations_on(&conn, xid);
                    let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
                        .x(x as i32)
                        .y(y as i32);
                    // Place and request fullscreen back to back: on
                    // compliant WMs the first placement sticks and
                    // the request lands within a frame. The re-assert
                    // below covers WMs whose re-framing re-places the
                    // window (openbox's anti-overlap cascade).
                    let _ = conn.configure_window(xid, &aux);
                    let _ = conn.flush();
                    request_fullscreen_on(&conn, xid);
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    let _ = conn.configure_window(xid, &aux);
                    let _ = conn.flush();
                    request_fullscreen_on(&conn, xid);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (class, x, y);
}

/// _NET_WM_STATE += the named state via a client message.
#[cfg(target_os = "linux")]
fn request_state_on(conn: &impl Connection, win: u32, atom_name: &[u8]) {
    let intern = |name: &[u8]| {
        conn.intern_atom(false, name)
            .ok()?
            .reply()
            .ok()
            .map(|r| r.atom)
    };
    let Some(state) = intern(b"_NET_WM_STATE") else {
        return;
    };
    let Some(property) = intern(atom_name) else {
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
            property,
            0,
            1, // source: application
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
#[cfg(target_os = "linux")]
fn activate_window_on(conn: &impl Connection, win: u32) {
    let intern = |name: &[u8]| {
        conn.intern_atom(false, name)
            .ok()?
            .reply()
            .ok()
            .map(|r| r.atom)
    };
    let Some(active) = intern(b"_NET_ACTIVE_WINDOW") else {
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

/// _NET_WM_STATE += _NET_WM_STATE_FULLSCREEN via a client message.
#[cfg(target_os = "linux")]
fn request_fullscreen_on(conn: &impl Connection, win: u32) {
    request_state_on(conn, win, b"_NET_WM_STATE_FULLSCREEN");
}

/// Pin a window above everything: _NET_WM_STATE_ABOVE via a client
/// message once the window's XID exists. Used by the pin-to-screen
/// reference window.
pub fn always_on_top_after_map(class: String) {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        std::thread::spawn(move || {
            let Ok((conn, _)) = x11rb::connect(None) else {
                return;
            };
            for _ in 0..120 {
                if let Some(xid) = find_xid_by_class_on(&conn, &class) {
                    suppress_decorations_on(&conn, xid);
                    request_state_on(&conn, xid, b"_NET_WM_STATE_ABOVE");
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = class;
}

/// Span the virtual screen with one window: explicit placement at the
/// union rect plus _NET_WM_STATE_ABOVE. _NET_WM_STATE_FULLSCREEN pins
/// a window to a single monitor, so the overlay does not use it.
pub fn span_after_map(class: String, x: i32, y: i32, w: u32, h: u32) {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        std::thread::spawn(move || {
            let Ok((conn, _)) = x11rb::connect(None) else {
                return;
            };
            for _ in 0..120 {
                if let Some(xid) = find_xid_by_class_on(&conn, &class) {
                    suppress_decorations_on(&conn, xid);
                    let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
                        .x(x)
                        .y(y)
                        .width(w)
                        .height(h)
                        .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE);
                    let _ = conn.configure_window(xid, &aux);
                    let _ = conn.flush();
                    request_state_on(&conn, xid, b"_NET_WM_STATE_ABOVE");
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    // The WM's own placement pass may have moved us;
                    // assert the span again.
                    let _ = conn.configure_window(xid, &aux);
                    let _ = conn.flush();
                    request_state_on(&conn, xid, b"_NET_WM_STATE_ABOVE");
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (class, x, y, w, h);
}

/// Find a mapped client's XID by a WM_CLASS substring. Uses
/// _NET_CLIENT_LIST: WMs reparent clients into frames, so the window
/// is not a direct child of the root. With several surfaces sharing a
/// class (a toast replacing its predecessor, two editors), the newest
/// is the one a post-map fixup is for: _NET_CLIENT_LIST appends in
/// map order, so take the LAST match.
pub fn find_xid_by_class_on(conn: &impl Connection, class_substr: &str) -> Option<u32> {
    let root = conn.setup().roots[0].root;
    let client_list = conn
        .intern_atom(false, b"_NET_CLIENT_LIST")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let windows = conn
        .get_property(false, root, client_list, AtomEnum::WINDOW, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    // Pipeline the WM_CLASS reads: one request per client window, all
    // sent before the first reply is awaited. Sequential reply() calls
    // cost a round trip per window; batched, the whole scan is one.
    let pending: Vec<(u32, _)> = windows
        .value
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .filter_map(|win| {
            conn.get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
                .ok()
                .map(|cookie| (win, cookie))
        })
        .collect();
    let mut newest = None;
    for (win, cookie) in pending {
        let Ok(class) = cookie.reply() else { continue };
        if class
            .value
            .windows(class_substr.len())
            .any(|w| w == class_substr.as_bytes())
        {
            newest = Some(win);
        }
    }
    newest
}

/// Move a window, retrying the XID lookup: _NET_CLIENT_LIST updates a
/// beat after the map, and through the single-instance forward the
/// hop adds real latency, so the budget is seconds, not frames. The
/// move itself repeats for a second: the decoration and window-type
/// changes make the WM re-frame the window, and re-framing re-applies
/// the WM's own placement (openbox's anti-overlap cascade) after a
/// single-shot move has already landed.
pub fn move_to(class_substr: &str, x: i32, y: i32, notification: bool) {
    let Ok((conn, _)) = x11rb::connect(None) else {
        return;
    };
    for _ in 0..120 {
        if let Some(xid) = find_xid_by_class_on(&conn, class_substr) {
            if notification {
                set_window_type_notification_on(&conn, xid);
            }
            suppress_decorations_on(&conn, xid);
            let aux = x11rb::protocol::xproto::ConfigureWindowAux::new().x(x).y(y);
            let _ = conn.configure_window(xid, &aux);
            let _ = conn.flush();
            for _ in 0..10 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let _ = conn.configure_window(xid, &aux);
                let _ = conn.flush();
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Drag the window under the press until the button is released.
/// Client-side decorations give the WM no title bar to grab, and
/// _NET_WM_MOVERESIZE loses the pointer grab against GPUI's
/// click-handling grab, so the title bar drags directly: poll the
/// root pointer and slide the window by the delta. WM-independent.
pub fn begin_wm_move(class: String, root_x: i32, root_y: i32) {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        std::thread::spawn(move || {
            let Ok((conn, screen_num)) = x11rb::connect(None) else {
                return;
            };
            let root = conn.setup().roots[screen_num].root;
            let mut xid = None;
            for _ in 0..20 {
                if let Some(found) = find_xid_by_class_on(&conn, &class) {
                    xid = Some(found);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            let Some(xid) = xid else {
                return;
            };
            // The window's current root position.
            let Ok(trans) = conn.translate_coordinates(xid, root, 0, 0) else {
                return;
            };
            let Ok(trans) = trans.reply() else {
                return;
            };
            let win0 = (i32::from(trans.dst_x), i32::from(trans.dst_y));
            let anchor = (root_x, root_y);
            loop {
                let Ok(pointer) = conn.query_pointer(root) else {
                    return;
                };
                let Ok(pointer) = pointer.reply() else {
                    return;
                };
                if pointer.mask
                    & x11rb::protocol::xproto::KeyButMask::BUTTON1
                    != x11rb::protocol::xproto::KeyButMask::BUTTON1
                {
                    return; // released: the drag ends
                }
                let dx = i32::from(pointer.root_x) - anchor.0;
                let dy = i32::from(pointer.root_y) - anchor.1;
                if dx != 0 || dy != 0 {
                    let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
                        .x(win0.0 + dx)
                        .y(win0.1 + dy);
                    let _ = conn.configure_window(xid, &aux);
                    let _ = conn.flush();
                }
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (class, root_x, root_y);
}

#[cfg(target_os = "linux")]
pub fn set_window_type_notification_on(conn: &impl Connection, win: u32) {
    let Ok(ty) = conn.intern_atom(false, b"_NET_WM_WINDOW_TYPE").map(|r| r.reply()) else {
        return;
    };
    let Ok(notif) = conn
        .intern_atom(false, b"_NET_WM_WINDOW_TYPE_NOTIFICATION")
        .map(|r| r.reply())
    else {
        return;
    };
    let (Ok(ty), Ok(notif)) = (ty, notif) else { return };
    let _ = conn.change_property32(
        x11rb::protocol::xproto::PropMode::REPLACE,
        win,
        ty.atom,
        AtomEnum::ATOM,
        &[notif.atom],
    );
    let _ = conn.flush();
}

/// Bring a parked window back over the virtual screen: configure the
/// union rect, raise, focus. The window was never unmapped, so this
/// skips renderer init entirely; one XCB round trip of latency.
pub fn unpark_span(class: String, x: i32, y: i32, w: u32, h: u32) {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        std::thread::spawn(move || {
            let Ok((conn, _)) = x11rb::connect(None) else {
                return;
            };
            if let Some(xid) = find_xid_by_class_on(&conn, &class) {
                let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .x(x)
                    .y(y)
                    .width(w)
                    .height(h)
                    .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE);
                let _ = conn.configure_window(xid, &aux);
                let _ = conn.map_window(xid);
                let _ = conn.flush();
                request_state_on(&conn, xid, b"_NET_WM_STATE_ABOVE");
                // Activate through the WM (_NET_ACTIVE_WINDOW), not a
                // bare set_input_focus: a reparenting WM applies its own
                // focus when it processes the map, and a raw grab that
                // lands first loses the race and the overlay's first
                // keystrokes go to the root window.
                activate_window_on(&conn, xid);
                let _ = conn.set_input_focus(
                    x11rb::protocol::xproto::InputFocus::POINTER_ROOT,
                    xid,
                    x11rb::CURRENT_TIME,
                );
                let _ = conn.flush();
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (class, x, y, w, h);
}

/// _MOTIF_WM_HINTS decorations=0: GPUI falls back to server-side
/// decorations when no compositor is present, and WMs like openbox
/// then titlebar even notification windows. Set the hint directly.
pub fn suppress_decorations_on(conn: &impl Connection, win: u32) {
    let Ok(motif) = conn.intern_atom(false, b"_MOTIF_WM_HINTS") else {
        return;
    };
    let Ok(motif) = motif.reply() else { return };
    // flags=2 (decorations), functions=0, decorations=0, input=0, status=0
    let hints: [u32; 5] = [2, 0, 0, 0, 0];
    let _ = conn.change_property32(
        x11rb::protocol::xproto::PropMode::REPLACE,
        win,
        motif.atom,
        motif.atom,
        &hints,
    );
    let _ = conn.flush();
}
