//! Windows: the NSIS installer, started silent and detached.

use std::path::Path;

/// Release asset suffix for this platform.
pub const ASSET: &str = "windows-x86_64-setup.exe";

/// The installer's options (packaging/windows/iris.nsi): `/S` runs it
/// silent, and `/RUN` starts iris when the installation ends, whether
/// it succeeded or failed.
const OPTIONS: [&str; 2] = ["/S", "/RUN"];

/// Replace the installed iris with the downloaded installer `file` and
/// restart; returns only on failure.
///
/// Windows denies write access to the file of a running program, and
/// this process may run the installed iris.exe. The installer waits
/// until no process runs that file before it writes it, so this process
/// starts the installer detached and exits.
pub fn apply_file(file: &Path) -> Result<(), String> {
    crate::sys::detach::spawn(file, &OPTIONS)
        .map_err(|e| format!("update: start the installer {}: {e}", file.display()))?;
    std::process::exit(0);
}
