//! Where each platform installs the external programs iris runs, and
//! how a spawn from the daemon hides its console.

use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::Command;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub(crate) use windows::{extra_dirs, hide_console};

/// Runtime directories searched before PATH. A runtime beside the
/// executable covers the AppImage, macOS, Windows, and portable Linux
/// layouts. Native Linux packages install iris in /usr/bin and FFmpeg
/// in a private library directory, so a distro /usr/bin/ffmpeg does
/// not take precedence over the runtime shipped by iris.
pub(crate) fn bundled_dirs() -> Vec<PathBuf> {
    crate::sys::exe::this()
        .map(|exe| bundled_dirs_from(&exe))
        .unwrap_or_default()
}

pub(crate) fn bundled_dirs_from(exe: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let exe_dir = exe.parent().map(Path::to_path_buf);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let native = exe_dir.as_deref() == Some(Path::new("/usr/bin"));
        if !native {
            dirs.extend(exe_dir.clone());
        }
        dirs.extend(["/usr/lib/iris", "/usr/lib64/iris"].map(PathBuf::from));
        if native {
            dirs.extend(exe_dir);
        }
    }
    #[cfg(any(windows, target_os = "macos"))]
    dirs.extend(exe_dir);
    dirs
}

/// Directories beyond PATH, searched after it: Homebrew (Apple silicon,
/// then Intel) and MacPorts.
#[cfg(target_os = "macos")]
pub(crate) fn extra_dirs() -> Vec<PathBuf> {
    ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
        .map(PathBuf::from)
        .into()
}

/// A Linux session's PATH already holds the package manager's prefix.
#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn extra_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// A Unix child inherits the daemon's terminal, or none: no console
/// window opens for it.
#[cfg(not(windows))]
pub(crate) fn hide_console(_cmd: &mut Command) {}
