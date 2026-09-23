//! Windows: the NSIS installer, run silent by a detached helper.

use std::path::{Path, PathBuf};

/// Release asset suffix for this platform.
pub const ASSET: &str = "windows-x86_64-setup.exe";

/// Replace the installed iris with the downloaded asset `file` and
/// restart; returns only on failure.
pub fn apply_file(file: &Path) -> Result<(), String> {
    // The updater IS the installed iris.exe, and Windows locks a running
    // executable against overwrite. Hand off to a detached cmd helper
    // that waits for this process to exit, runs the NSIS installer
    // silent, then relaunches iris. Exit immediately so the exe is free
    // when the installer writes.
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("iris.exe"));
    let helper = format!(
        "timeout /t 2 /nobreak >nul & \"\"{}\" /S & start \"\" \"{}\"\"",
        file.display(),
        exe.display()
    );
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("cmd")
        .args(["/C", &helper])
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("update: spawn installer helper: {e}"))?;
    std::process::exit(0);
}
