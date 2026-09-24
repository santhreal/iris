//! macOS: the DMG, copied over `/Applications/iris.app`.

use std::path::Path;

/// Release asset suffix for this platform.
pub const ASSET: &str = "macos-universal.dmg";

/// Replace the installed iris with the downloaded asset `file` and
/// restart; returns only on failure.
pub fn apply_file(file: &Path) -> Result<(), String> {
    // Mount the DMG, replace /Applications/iris.app, unmount, relaunch.
    // macOS lets a running .app be unlinked, so rm the old bundle before
    // copying: a bare `cp -R` would merge and leave stale files behind.
    let out = std::process::Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-readonly"])
        .arg(file)
        .output()
        .map_err(|e| format!("update: hdiutil attach: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "update: hdiutil attach failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mount = stdout
        .lines()
        .rev()
        .find_map(|l| {
            l.split('\t')
                .next_back()
                .map(str::trim)
                .filter(|s| s.starts_with('/'))
        })
        .ok_or_else(|| "update: no mount point in hdiutil output".to_string())?;
    let src = Path::new(mount).join("iris.app");
    let dst = Path::new("/Applications/iris.app");
    let _ = std::fs::remove_dir_all(dst);
    let copy = std::process::Command::new("cp")
        .args(["-R"])
        .arg(&src)
        .arg(dst)
        .status();
    let _ = std::process::Command::new("hdiutil")
        .args(["detach", mount])
        .status();
    match copy {
        Ok(s) if s.success() => super::relaunch(&dst.join("Contents/MacOS/iris")),
        Ok(s) => Err(format!("update: copy app exited {s}")),
        Err(e) => Err(format!("update: copy app: {e}")),
    }
}
