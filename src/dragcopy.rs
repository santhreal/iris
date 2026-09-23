//! Files out of iris: drag-out and file-list copy, one implementation
//! per platform behind these functions. X11 speaks XDnD and serves
//! `text/uri-list`; Windows uses OLE drag and `CF_HDROP`; macOS uses an
//! `NSDraggingSession` and `NSURL`s on the pasteboard.

use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, test))]
mod windows;
#[cfg(target_os = "macos")]
use macos::copy_abs_paths;
#[cfg(windows)]
use windows::copy_abs_paths;
#[cfg(target_os = "linux")]
use x11::copy_abs_paths;

/// Thumbnail carried under the pointer during a drag. On X11 it is an
/// override-redirect window with a rounded shape mask, moved at poll
/// rate by the drag thread; Windows and macOS draw the shell's own drag
/// image instead. Plain data, defined on every platform.
pub struct DragIcon {
    /// Shared so a surface that already holds the pixels (the toast's
    /// thumb_rgba) hands over a refcount, not a multi-MB clone.
    pub rgba: std::sync::Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
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

/// Linux: an XDnD drag on X11, imaged with `icon`.
#[cfg(target_os = "linux")]
pub fn start_file_drag_at_cursor(
    paths: Vec<PathBuf>,
    icon: Option<DragIcon>,
) -> Result<(), String> {
    x11::start_file_drag_at_cursor(paths, icon)
}

/// Windows: an OLE drag of the files; the shell draws the drag image,
/// so `icon` is unused.
#[cfg(windows)]
pub fn start_file_drag_at_cursor(
    paths: Vec<PathBuf>,
    _icon: Option<DragIcon>,
) -> Result<(), String> {
    let abs = paths
        .iter()
        .map(|p| absolute_existing(p))
        .collect::<Result<Vec<_>, _>>()?;
    windows::drag_abs_paths(abs)
}

/// macOS: an `NSDraggingSession` begun from `ns_view` (the window's
/// content view) and the mouse event being handled. AppKit images each
/// file with its Finder icon. Main thread only.
#[cfg(target_os = "macos")]
pub fn start_file_drag_from_view(
    ns_view: *mut core::ffi::c_void,
    paths: Vec<PathBuf>,
) -> Result<(), String> {
    let abs = paths
        .iter()
        .map(|p| absolute_existing(p))
        .collect::<Result<Vec<_>, _>>()?;
    macos::drag_abs_paths(ns_view, &abs)
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
