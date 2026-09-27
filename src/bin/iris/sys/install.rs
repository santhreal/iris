//! Installing a downloaded release over the running iris: each kind of
//! install updates from one release asset and swaps it in its own way.

use std::path::Path;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{apply_file, ready, ASSET};
#[cfg(target_os = "macos")]
pub use macos::{apply_file, ready, ASSET};
#[cfg(windows)]
pub use windows::{apply_file, asset, ready};

/// The release asset suffix that updates this iris.
#[cfg(not(windows))]
pub fn asset() -> &'static str {
    ASSET
}

/// Delete what the last update left beside this iris: the iris.exe a
/// portable Windows update renamed aside. The daemon runs this once it
/// has started. The other installs leave nothing behind.
pub fn tidy() {
    #[cfg(windows)]
    windows::tidy();
}

/// Start the new binary at `exe` and exit this process. The daemon is
/// already stopped (see `update::apply`), so the fresh `iris` binds the
/// socket and becomes the daemon rather than forwarding to a stale
/// instance. It starts detached, as a client starts the daemon: closing
/// the terminal `iris --update` ran in does not end it. An installed
/// Windows iris does not come here: its installer starts iris (`/RUN`).
fn relaunch(exe: &Path) -> ! {
    if let Err(e) = crate::sys::detach::spawn(exe, &[]) {
        iris_lib::ilog!("iris: start the updated daemon {}: {e}", exe.display());
    }
    std::process::exit(0);
}
