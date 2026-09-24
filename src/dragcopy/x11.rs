//! X11 file transfer: XDnD drag-out and CLIPBOARD serving of
//! `text/uri-list`, each from a private 1x1 window on its own
//! connection. Wayland sessions have no route here and fail with an
//! explicit error.

use std::path::PathBuf;

use super::{uri_encode_path, DragIcon};

mod clipboard;
mod icon;
#[cfg(test)]
mod tests;
mod xdnd;

use clipboard::serve_x11_clipboard;
use xdnd::{run_xdnd_drag, XdndAtoms};

use x11rb::connection::Connection;
use x11rb::protocol::xfixes::{ConnectionExt as XfixesExt, SelectionEventMask};
use x11rb::protocol::xproto::{
    ConnectionExt as XprotoExt, CreateWindowAux, EventMask, WindowClass,
};
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

/// The four atoms a clipboard serve answers on. Grouping them stops a
/// TARGETS/uri-list/UTF8 transposition at the call site from compiling
/// silently.
pub(super) struct CbAtoms {
    pub(super) clipboard: x11rb::protocol::xproto::Atom,
    pub(super) targets: x11rb::protocol::xproto::Atom,
    pub(super) uri_list: x11rb::protocol::xproto::Atom,
    pub(super) utf8: x11rb::protocol::xproto::Atom,
}

/// Interned atoms shared across drags and clipboard serves: atoms are
/// server-global constants, so the 13-name XDnD set and the 4-name
/// clipboard set resolve once per process instead of once per call.
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

pub(super) fn copy_abs_paths(abs: &[PathBuf]) -> Result<(), String> {
    let first = abs[0].to_string_lossy().into_owned();
    serve_uri_list(build_uri_list(abs), first)
}

/// Acquire CLIPBOARD with a text/uri-list payload and serve it from a
/// background thread until another owner takes over or 5 minutes pass.
fn serve_uri_list(uri_list: String, fallback_text: String) -> Result<(), String> {
    if !crate::session::x11() {
        return Err("file copy needs an X11 session".to_string());
    }
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    let clipboard_atom = atom_cached(&conn, b"CLIPBOARD").ok_or("intern CLIPBOARD failed")?;
    let targets_atom = atom_cached(&conn, b"TARGETS").ok_or("intern TARGETS failed")?;
    let uri_list_atom =
        atom_cached(&conn, b"text/uri-list").ok_or("intern text/uri-list failed")?;
    let utf8_atom = atom_cached(&conn, b"UTF8_STRING").ok_or("intern UTF8_STRING failed")?;

    let win = conn
        .generate_id()
        .map_err(|e| format!("generate window id: {e}"))?;
    let aux = CreateWindowAux::new()
        .override_redirect(1)
        .event_mask(EventMask::PROPERTY_CHANGE);
    conn.create_window(
        x11rb::COPY_FROM_PARENT as u8,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &aux,
    )
    .map_err(|e| format!("create clipboard window: {e}"))?;

    conn.set_selection_owner(win, clipboard_atom, CURRENT_TIME)
        .map_err(|e| format!("set selection owner: {e}"))?;
    let owner = conn
        .get_selection_owner(clipboard_atom)
        .map_err(|e| format!("get selection owner: {e}"))?
        .reply()
        .map_err(|e| format!("get selection owner reply: {e}"))?
        .owner;
    if owner != win {
        return Err("failed to acquire CLIPBOARD selection".to_string());
    }

    if let Ok(cookie) = conn.xfixes_query_version(4, 0) {
        if cookie.reply().is_ok() {
            let _ = conn.xfixes_select_selection_input(
                root,
                clipboard_atom,
                SelectionEventMask::SET_SELECTION_OWNER,
            );
        }
    }
    let _ = conn.flush();

    std::thread::Builder::new()
        .name("iris-x11-clipboard".into())
        .spawn(move || {
            serve_x11_clipboard(
                conn,
                win,
                CbAtoms {
                    clipboard: clipboard_atom,
                    targets: targets_atom,
                    uri_list: uri_list_atom,
                    utf8: utf8_atom,
                },
                uri_list,
                fallback_text,
            );
        })
        .map_err(|e| format!("spawn clipboard thread: {e}"))?;
    Ok(())
}
