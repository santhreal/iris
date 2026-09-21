//! X11 window helpers shared by the native surfaces: WM_CLASS-based
//! XID lookup (GPUI's X11 HasWindowHandle is unimplemented) and direct
//! window moves for positions a window manager would otherwise
//! override at map time.

mod fixup;
mod move_drag;

pub use fixup::{
    always_on_top_after_map, place_after_map, place_after_map_kind,
    span_after_map, suppress_decorations_on, unpark_span,
};
pub use move_drag::begin_wm_move;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

/// A process-shared X connection for `begin_wm_move`'s drag tracking.
/// The post-map fixups run on the dispatcher's own connection; this
/// one stays separate so a drag's event reads never race the
/// dispatcher's. x11rb serializes requests internally.
#[cfg(target_os = "linux")]
pub(super) fn shared_conn() -> Option<(&'static x11rb::rust_connection::RustConnection, usize)> {
    static CONN: std::sync::LazyLock<Option<(x11rb::rust_connection::RustConnection, usize)>> =
        std::sync::LazyLock::new(|| x11rb::connect(None).ok());
    CONN.as_ref().map(|(c, s)| (c, *s))
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub(super) fn shared_conn() -> Option<(&'static x11rb::rust_connection::RustConnection, usize)> {
    None
}

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
pub fn centered_origin(cx: &gpui::App, w: f32, h: f32, fallback: (f32, f32)) -> (f32, f32) {
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
    format!(
        "{base}.{:x}.{}",
        nanos,
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Find a mapped client's XID by a WM_CLASS substring. Uses
/// _NET_CLIENT_LIST: WMs reparent clients into frames, so the window
/// is not a direct child of the root. With several surfaces sharing a
/// class (a toast replacing its predecessor, two editors), the newest
/// is the one a post-map fixup is for: _NET_CLIENT_LIST appends in
/// map order, so take the LAST match.
pub fn find_xid_by_class_on(conn: &impl Connection, class_substr: &str) -> Option<u32> {
    let root = conn.setup().roots[0].root;
    let client_list = fixup::atom_cached(conn, b"_NET_CLIENT_LIST")?;
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
