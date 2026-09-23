//! Installing a downloaded release over the running iris: each OS
//! ships one release asset and swaps it in its own way.

#[cfg(not(windows))]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{apply_file, ASSET};
#[cfg(target_os = "macos")]
pub use macos::{apply_file, ASSET};
#[cfg(windows)]
pub use windows::{apply_file, ASSET};

/// Spawn the new binary and exit this process. The daemon is already
/// stopped (see `update::apply`), so the fresh `iris` binds the socket and
/// becomes the daemon rather than forwarding to a stale instance.
/// Windows hands off to a detached installer helper instead.
#[cfg(not(windows))]
fn relaunch() -> ! {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("iris"));
    let _ = std::process::Command::new(exe).spawn();
    std::process::exit(0);
}
