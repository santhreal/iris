//! A failure's reason in words, for a notice, a status line, or the
//! library preview: a file by its name, and the cause a reader acts on.
//! The full path and the underlying error go to iris.log.

use std::io::ErrorKind;
use std::path::Path;

#[cfg(test)]
mod tests;

/// `path`'s file name, or the whole path where it has none.
pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// `path` did not open or read: `e` is the failed call's error.
pub fn unreadable(path: &Path, e: &std::io::Error) -> String {
    crate::ilog!("iris: read {}: {e}", path.display());
    let name = file_name(path);
    match e.kind() {
        ErrorKind::NotFound => format!("{name} no longer exists."),
        ErrorKind::PermissionDenied => format!("No permission to read {name}."),
        _ => format!("{name} did not read: {e}."),
    }
}

/// `path` did not decode as an image: `e` is the decoder's error, or
/// the open's, which states the open's cause instead. A stream that
/// ends early is a damaged image, not a failed read.
pub fn not_an_image(path: &Path, e: &image::ImageError) -> String {
    match e {
        image::ImageError::IoError(e) if e.kind() != ErrorKind::UnexpectedEof => {
            unreadable(path, e)
        }
        _ => {
            crate::ilog!("iris: decode {}: {e}", path.display());
            undecodable(path)
        }
    }
}

/// `path` holds no image the decoder reads.
pub fn undecodable(path: &Path) -> String {
    format!("{} is not a readable image.", file_name(path))
}

/// `path` does not start with the PNG signature: the editor opens PNG
/// files only.
pub fn not_png(path: &Path) -> String {
    format!("{} is not a PNG image.", file_name(path))
}
