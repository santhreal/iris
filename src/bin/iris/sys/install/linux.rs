//! Linux: the AppImage, renamed over `$APPIMAGE`. A deb/rpm install
//! defers to the package manager.

use std::path::{Path, PathBuf};

/// Release asset suffix for this platform: the AppImage is the only
/// self-updatable Linux artifact.
pub const ASSET: &str = "linux-x86_64.AppImage";

/// Replace the installed iris with the downloaded asset `file` and
/// restart; returns only on failure.
pub fn apply_file(file: &Path) -> Result<(), String> {
    // Only an AppImage install can self-update. A running binary cannot
    // be overwritten in place (ETXTBSY), but a rename() over it is
    // atomic and allowed: copy the download to a sibling of $APPIMAGE,
    // then rename it onto the target.
    let appimage = std::env::var("APPIMAGE")
        .map_err(|_| "update: not an AppImage install; update via apt/dnf".to_string())?;
    let target = PathBuf::from(appimage);
    let tmp = target.with_extension("new");
    std::fs::copy(file, &tmp).map_err(|e| format!("update: stage {}: {e}", tmp.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("update: chmod {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &target)
        .map_err(|e| format!("update: replace {}: {e}", target.display()))?;
    super::relaunch()
}
