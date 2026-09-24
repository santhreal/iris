//! X11 window operations for `sys::window`, keyed by the window's XID:
//! window manager state for managed windows, moves and resizes that
//! follow the pointer where the window manager does not handle
//! _NET_WM_MOVERESIZE, and the present report.

mod fixup;
mod present;
mod wm_drag;

pub use fixup::{always_on_top_after_map, span_after_map, unpark_span};
pub use present::PresentWatch;
pub use wm_drag::{begin_wm_move, begin_wm_resize};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

/// `window`'s X11 id; None for a window that is not an X11 window, such
/// as a Wayland surface.
pub fn xid(window: &gpui::Window) -> Option<u32> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

/// Whether a running EWMH window manager moves and resizes a window on
/// _NET_WM_MOVERESIZE. _NET_SUPPORTED outlives a window manager that
/// exits, so its _NET_SUPPORTING_WM_CHECK window must still name itself.
pub fn wm_moveresize() -> bool {
    let Ok((conn, screen_num)) = iris_lib::capture::x11::shared_conn() else {
        return false;
    };
    let root = conn.setup().roots[screen_num].root;
    let (Some(check), Some(supported), Some(moveresize)) = (
        fixup::atom_cached(conn, b"_NET_SUPPORTING_WM_CHECK"),
        fixup::atom_cached(conn, b"_NET_SUPPORTED"),
        fixup::atom_cached(conn, b"_NET_WM_MOVERESIZE"),
    ) else {
        return false;
    };
    let wm = conn.get_property(false, root, check, AtomEnum::WINDOW, 0, 1);
    let atoms = conn.get_property(false, root, supported, AtomEnum::ATOM, 0, 4096);
    let (Ok(wm), Ok(atoms)) = (wm, atoms) else {
        return false;
    };
    let first = |reply: x11rb::protocol::xproto::GetPropertyReply| reply.value32()?.next();
    let Some(wm) = wm.reply().ok().and_then(first) else {
        return false;
    };
    let handled = atoms
        .reply()
        .ok()
        .and_then(|reply| Some(reply.value32()?.any(|atom| atom == moveresize)))
        .unwrap_or(false);
    handled
        && conn
            .get_property(false, wm, check, AtomEnum::WINDOW, 0, 1)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .and_then(first)
            == Some(wm)
}
