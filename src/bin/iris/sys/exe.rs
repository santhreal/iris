//! The iris binary this process runs.
//!
//! A package upgrade (dpkg, rpm) renames the new binary over the old
//! one while the daemon keeps running the old file. Linux then reports
//! the running binary's path with " (deleted)" appended, a path that
//! names no file: an autostart entry written with it starts nothing,
//! and a spawn of it fails. The file at the path without the suffix is
//! the installed iris, the one an entry or a spawn has to run.
//!
//! On Linux, `appimage` is the AppImage file this binary runs from:
//! the file to replace on update and to start at login.

use std::io;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod appimage;
#[cfg(target_os = "linux")]
pub use appimage::appimage;

/// The suffix Linux appends to the path of a running binary whose file
/// was deleted or replaced (proc(5), /proc/pid/exe).
const DELETED: &str = " (deleted)";

/// The path of the iris binary this process runs: the file installed
/// there now when a package upgrade replaced it.
#[allow(clippy::disallowed_methods)]
pub fn this() -> io::Result<PathBuf> {
    std::env::current_exe().map(installed)
}

/// `exe` without the suffix of a replaced binary, when a file is at the
/// path without it. Otherwise `exe`: the binary was removed, and a
/// spawn of it fails with that path in its error.
fn installed(exe: PathBuf) -> PathBuf {
    let Some(path) = exe.to_str().and_then(|p| p.strip_suffix(DELETED)) else {
        return exe;
    };
    let path = PathBuf::from(path);
    if path.is_file() {
        path
    } else {
        exe
    }
}

// WHY: the class closed here is "a daemon whose binary a package upgrade
// replaced writes or spawns a path that names no file". Not covered:
// a binary deleted with no file put in its place, which has no iris to
// run.
#[cfg(test)]
mod tests;
