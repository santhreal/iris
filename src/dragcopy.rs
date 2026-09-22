use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod clipboard;
#[cfg(target_os = "linux")]
mod icon;
#[cfg(target_os = "linux")]
mod xdnd;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, test))]
mod windows;
#[cfg(target_os = "macos")]
use macos::copy_abs_paths;
#[cfg(windows)]
use windows::copy_abs_paths;

#[cfg(all(test, target_os = "linux"))]
mod tests;

#[cfg(target_os = "linux")]
use clipboard::serve_x11_clipboard;
#[cfg(target_os = "linux")]
use xdnd::{run_xdnd_drag, XdndAtoms};

#[cfg(target_os = "linux")]
use x11rb::connection::Connection;
#[cfg(target_os = "linux")]
use x11rb::protocol::xfixes::{ConnectionExt as XfixesExt, SelectionEventMask};
#[cfg(target_os = "linux")]
use x11rb::protocol::xproto::{
    ConnectionExt as XprotoExt, CreateWindowAux, EventMask, WindowClass,
};
#[cfg(target_os = "linux")]
use x11rb::CURRENT_TIME;

/// Validate and normalize drag paths before any platform work.
#[cfg(target_os = "linux")]
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

/// Drag-out: speaks the XDnD protocol directly from a 1x1 window.
/// Grab-less by design: pointer motion is tracked by polling
/// XQueryPointer, so a window manager's passive-grab activation on the
/// press that started the gesture cannot block the drag (GTK's
/// drag_begin fails with AlreadyGrabbed in that state). Selection
/// requests and XdndStatus/XdndFinished arrive as events on our own
/// connection, which grabs do not affect.
/// Thumbnail carried under the pointer during a drag: an
/// override-redirect window with a rounded shape mask, moved at poll
/// rate by the drag thread. Plain data; defined on every platform so
/// the non-Linux drag stub keeps the same signature.
pub struct DragIcon {
    /// Shared so a surface that already holds the pixels (the toast's
    /// thumb_rgba) hands over a refcount, not a multi-MB clone.
    pub rgba: std::sync::Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

/// `text/uri-list` payload for a set of paths: one `file://` URI per
/// line, CRLF-terminated per RFC 2483. Pure so the wire format is
/// testable without an X connection.
#[cfg(target_os = "linux")]
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

/// Percent-encode a filesystem path for a file:// URI: unreserved
/// characters and '/' pass through, everything else (spaces, UTF-8,
/// '%' itself) is %XX. Without this a path with a space produces a
/// URI every file manager rejects.
pub fn uri_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(target_os = "linux")]
pub fn start_file_drag_at_cursor(
    paths: Vec<PathBuf>,
    icon: Option<DragIcon>,
) -> Result<(), String> {
    let paths = validate_drag_paths(paths)?;
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_none() {
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

#[cfg(not(target_os = "linux"))]
pub fn start_file_drag_at_cursor(
    _paths: Vec<PathBuf>,
    _icon: Option<DragIcon>,
) -> Result<(), String> {
    Err("file drag-out is implemented for Linux only".to_string())
}

/// The four atoms a clipboard serve answers on. Grouping them stops a
/// TARGETS/uri-list/UTF8 transposition at the call site from compiling
/// silently.
#[cfg(target_os = "linux")]
pub(super) struct CbAtoms {
    pub(super) clipboard: x11rb::protocol::xproto::Atom,
    pub(super) targets: x11rb::protocol::xproto::Atom,
    pub(super) uri_list: x11rb::protocol::xproto::Atom,
    pub(super) utf8: x11rb::protocol::xproto::Atom,
}

/// Interned atoms shared across drags and clipboard serves: atoms are
/// server-global constants, so the 13-name XDnD set and the 4-name
/// clipboard set resolve once per process instead of once per call.
#[cfg(target_os = "linux")]
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

/// Shared clipboard image set: used by the capture pipeline's
/// process-global clipboard owner.
pub fn clipboard_set_image(
    cb: &mut arboard::Clipboard,
    path: &std::path::Path,
) -> Result<(), String> {
    let img = image::open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?
        .to_rgba8();
    cb.set_image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: std::borrow::Cow::Borrowed(img.as_raw()),
    })
    .map_err(|e| format!("clipboard set_image: {e}"))
}

pub fn clipboard_set_text(cb: &mut arboard::Clipboard, text: &str) -> Result<(), String> {
    cb.set_text(text)
        .map_err(|e| format!("clipboard set_text: {e}"))
}

/// Copy files to the clipboard in the platform's file-list format:
/// `text/uri-list` on Linux, `CF_HDROP` on Windows, `NSURL`s on macOS.
/// File managers paste the files themselves.
pub fn copy_file_paths(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("no paths provided".to_string());
    }
    let abs = paths
        .iter()
        .map(|p| absolute_existing(p))
        .collect::<Result<Vec<_>, _>>()?;
    copy_abs_paths(&abs)
}

/// [`copy_file_paths`] for one file.
pub fn copy_file_path(path: &std::path::Path) -> Result<(), String> {
    copy_file_paths(&[path.to_path_buf()])
}

/// The absolute form of an existing path. Windows uses
/// `path::absolute`, because `canonicalize` yields a `\\?\` verbatim
/// path that some paste targets reject.
fn absolute_existing(p: &std::path::Path) -> Result<PathBuf, String> {
    if !p.exists() {
        return Err(format!("file does not exist: {}", p.display()));
    }
    #[cfg(windows)]
    let abs = std::path::absolute(p).map_err(|e| format!("resolve {}: {e}", p.display()))?;
    #[cfg(not(windows))]
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    Ok(abs)
}

#[cfg(target_os = "linux")]
fn copy_abs_paths(abs: &[PathBuf]) -> Result<(), String> {
    let first = abs[0].to_string_lossy().into_owned();
    serve_uri_list(build_uri_list(abs), first)
}

/// Acquire CLIPBOARD with a text/uri-list payload and serve it from a
/// background thread until another owner takes over or 5 minutes pass.
#[cfg(target_os = "linux")]
pub fn serve_uri_list(uri_list: String, fallback_text: String) -> Result<(), String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_none() {
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
