//! X11 drag-out: XDnD from a private 1x1 window on its own connection.
//! Wayland sessions have no route here and fail with an explicit error.

use std::path::PathBuf;

use super::{uri_encode_path, DragIcon};

mod icon;
#[cfg(test)]
mod tests;
mod xdnd;

use xdnd::{run_xdnd_drag, XdndAtoms};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as XprotoExt, CreateWindowAux, WindowClass};
use x11rb::CURRENT_TIME;

/// Validate and normalize drag paths before any platform work.
fn validate_drag_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() {
        return Err("no paths provided for file drag".to_string());
    }
    let mut path_bufs = Vec::with_capacity(paths.len());
    for pb in paths {
        if !pb.exists() {
            return Err(format!("drag path does not exist: {}", pb.display()));
        }
        path_bufs.push(pb);
    }
    Ok(path_bufs)
}

/// `text/uri-list` payload for a set of paths: one `file://` URI per
/// line, CRLF-terminated per RFC 2483. Pure so the wire format is
/// testable without an X connection.
fn build_uri_list(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for pb in paths {
        let abs = std::fs::canonicalize(pb).unwrap_or_else(|_| pb.clone());
        out.push_str(&format!(
            "file://{}\r\n",
            uri_encode_path(&abs.to_string_lossy())
        ));
    }
    out
}

/// Drag-out: speaks the XDnD protocol directly from a 1x1 window.
/// Grab-less by design: pointer motion is tracked by polling
/// XQueryPointer, so a window manager's passive-grab activation on the
/// press that started the gesture cannot block the drag (GTK's
/// drag_begin fails with AlreadyGrabbed in that state). Selection
/// requests and XdndStatus/XdndFinished arrive as events on our own
/// connection, which grabs do not affect.
pub(super) fn start_file_drag_at_cursor(
    paths: Vec<PathBuf>,
    icon: Option<DragIcon>,
) -> Result<(), String> {
    let paths = validate_drag_paths(paths)?;
    if !crate::session::x11() {
        return Err("file drag needs an X11 session".to_string());
    }
    let uri_list = build_uri_list(&paths);
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))?;
    let root = conn.setup().roots[screen_num].root;
    let atoms = XdndAtoms::intern(&conn)?;

    let win = conn
        .generate_id()
        .map_err(|e| format!("generate window id: {e}"))?;
    let aux = CreateWindowAux::new().override_redirect(1);
    conn.create_window(
        x11rb::COPY_FROM_PARENT as u8,
        win,
        root,
        -1,
        -1,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &aux,
    )
    .map_err(|e| format!("create drag window: {e}"))?;
    conn.set_selection_owner(win, atoms.selection, CURRENT_TIME)
        .map_err(|e| format!("set XdndSelection owner: {e}"))?;
    let owner = conn
        .get_selection_owner(atoms.selection)
        .map_err(|e| format!("get XdndSelection owner: {e}"))?
        .reply()
        .map_err(|e| format!("get XdndSelection owner reply: {e}"))?
        .owner;
    if owner != win {
        return Err("failed to acquire XdndSelection".to_string());
    }
    let _ = conn.flush();

    std::thread::Builder::new()
        .name("iris-xdnd-drag".into())
        .spawn(move || run_xdnd_drag(conn, win, root, atoms, uri_list, icon))
        .map_err(|e| format!("spawn drag thread: {e}"))?;
    Ok(())
}

/// Interned atoms shared across drags: atoms are server-global
/// constants, so the 13-name XDnD set resolves once per process instead
/// of once per drag.
pub(super) fn atom_cached<C: Connection>(conn: &C, name: &'static [u8]) -> Option<u32> {
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
