//! X11 window helpers shared by the native surfaces: WM_CLASS-based
//! XID lookup (GPUI's X11 HasWindowHandle is unimplemented) and direct
//! window moves for positions a window manager would otherwise
//! override at map time.

mod fixup;
mod move_drag;

pub use fixup::{
    always_on_top_after_map, place_after_map, place_after_map_kind, span_after_map,
    suppress_decorations_on, unpark_span,
};
pub use move_drag::begin_wm_move;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

/// A process-shared X connection for `begin_wm_move`'s drag tracking.
/// The post-map fixups run on the dispatcher's own connection; this
/// one stays separate so a drag's event reads never race the
/// dispatcher's. x11rb serializes requests internally.
pub(super) fn shared_conn() -> Option<(&'static x11rb::rust_connection::RustConnection, usize)> {
    static CONN: std::sync::LazyLock<Option<(x11rb::rust_connection::RustConnection, usize)>> =
        std::sync::LazyLock::new(|| x11rb::connect(None).ok());
    CONN.as_ref().map(|(c, s)| (c, *s))
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
