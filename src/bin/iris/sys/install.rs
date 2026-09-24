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

/// Start the new binary at `exe` and exit this process. The daemon is
/// already stopped (see `update::apply`), so the fresh `iris` binds the
/// socket and becomes the daemon rather than forwarding to a stale
/// instance. It starts detached, as a client starts the daemon: closing
/// the terminal `iris --update` ran in does not end it. On Windows the
/// installer starts iris instead (`/RUN`).
#[cfg(not(windows))]
fn relaunch(exe: &Path) -> ! {
    if let Err(e) = crate::sys::detach::spawn(exe, &[]) {
        iris_lib::ilog!("iris: start the updated daemon {}: {e}", exe.display());
    }
    std::process::exit(0);
}
