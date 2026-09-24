//! Installing a downloaded release over the running iris: each OS
//! ships one release asset and swaps it in its own way.

#[cfg(not(windows))]
use std::path::Path;

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

/// Ok when this install can replace itself. `update::apply` checks it
/// before the download and before it stops the daemon, so a refusal
/// leaves the running iris as it was. A deb or rpm install cannot: the
/// package manager owns its files.
pub fn ready() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux::ready()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(())
    }
}

/// Spawn the new binary at `exe` and exit this process. The daemon is
/// already stopped (see `update::apply`), so the fresh `iris` binds the
/// socket and becomes the daemon rather than forwarding to a stale
/// instance. Windows hands off to a detached installer helper instead.
#[cfg(not(windows))]
fn relaunch(exe: &Path) -> ! {
    let _ = std::process::Command::new(exe).spawn();
    std::process::exit(0);
}
