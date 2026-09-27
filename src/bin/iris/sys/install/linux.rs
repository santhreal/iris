//! Linux: the AppImage, renamed over `$APPIMAGE`. A deb/rpm install
//! defers to the package manager.

use std::path::{Path, PathBuf};

/// Release asset suffix for this platform: the AppImage is the only
/// self-updatable Linux artifact.
pub const ASSET: &str = "linux-x86_64.AppImage";

/// The AppImage this iris runs from; the AppImage runtime sets
/// `$APPIMAGE`. A deb or rpm install has none.
fn appimage() -> Result<PathBuf, String> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .ok_or_else(|| "update: not an AppImage install; update via apt/dnf".to_string())
}

/// Ok when this install can replace itself: an AppImage can. A deb or
/// rpm install cannot: the package manager owns its files.
pub fn ready() -> Result<(), String> {
    appimage().map(drop)
}

/// Replace the installed iris with the downloaded asset `file` and
/// restart; returns only on failure.
pub fn apply_file(file: &Path) -> Result<(), String> {
    let target = appimage()?;
    replace(file, &target)?;
    // The AppImage, not sys::exe::this(): that path is inside the old
    // AppImage's mount, which still serves the old binary.
    super::relaunch(&target)
}

/// Put a copy of `file` at `target`. A running binary cannot be
/// overwritten in place (ETXTBSY), but a rename() over it is atomic and
/// allowed: copy `file` to a sibling of `target`, then rename it onto
/// `target`. A failure leaves `target` as it was and deletes the copy.
fn replace(file: &Path, target: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = target.with_extension("new");
    let staged = std::fs::copy(file, &tmp)
        .map_err(|e| format!("update: stage {}: {e}", tmp.display()))
        .and_then(|_| {
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("update: chmod {}: {e}", tmp.display()))
        })
        .and_then(|()| {
            std::fs::rename(&tmp, target)
                .map_err(|e| format!("update: replace {}: {e}", target.display()))
        });
    if staged.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    staged
}

// WHY: docs/install.md prints this refusal for a deb or rpm install;
// the test process, like a package install, has no $APPIMAGE. The
// class closed by the `replace` cases is "a failed swap leaves the
// AppImage changed, or a half-written copy beside it": the updater
// then restarts the old AppImage. Not covered: ETXTBSY on the running
// file, which a rename does not hit.
#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::replace;

    #[test]
    fn an_install_outside_an_appimage_refuses_to_update() {
        assert!(std::env::var_os("APPIMAGE").is_none());
        assert_eq!(
            super::ready(),
            Err("update: not an AppImage install; update via apt/dnf".to_string())
        );
    }

    /// Every name in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name().into_string().expect("utf-8"))
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_replace_puts_an_executable_copy_at_the_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (new, target) = (
            dir.path().join("download"),
            dir.path().join("iris.AppImage"),
        );
        std::fs::write(&new, b"new").expect("write");
        std::fs::write(&target, b"old").expect("write");
        replace(&new, &target).expect("replace");
        assert_eq!(std::fs::read(&target).expect("read"), b"new");
        let mode = std::fs::metadata(&target)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        assert_eq!(names(dir.path()), ["download", "iris.AppImage"]);
    }

    #[test]
    fn a_download_that_cannot_be_read_leaves_the_target_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("iris.AppImage");
        std::fs::write(&target, b"old").expect("write");
        let e = replace(&dir.path().join("missing"), &target).expect_err("no download");
        assert!(e.starts_with("update: stage "), "{e}");
        assert_eq!(std::fs::read(&target).expect("read"), b"old");
        assert_eq!(names(dir.path()), ["iris.AppImage"]);
    }

    #[test]
    fn a_rename_that_fails_deletes_the_staged_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let new = dir.path().join("download");
        std::fs::write(&new, b"new").expect("write");
        // A non-empty directory: rename() of a file over it fails.
        let target = dir.path().join("iris.AppImage");
        std::fs::create_dir(&target).expect("mkdir");
        std::fs::write(target.join("kept"), b"old").expect("write");
        let e = replace(&new, &target).expect_err("rename over a directory");
        assert!(e.starts_with("update: replace "), "{e}");
        assert_eq!(std::fs::read(target.join("kept")).expect("read"), b"old");
        assert_eq!(names(dir.path()), ["download", "iris.AppImage"]);
    }
}
