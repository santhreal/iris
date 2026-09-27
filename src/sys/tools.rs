//! Where each platform installs the external programs iris runs, and
//! how a spawn from the daemon hides its console.

#[cfg(not(windows))]
use std::{path::PathBuf, process::Command};

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub(crate) use windows::{extra_dirs, hide_console};

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
