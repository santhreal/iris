//! Linux: the AppImage, renamed over `$APPIMAGE`, and the deb and the
//! rpm, each installed over the installed package by its package
//! manager (`package`).

mod package;

use std::path::{Path, PathBuf};

use package::Package;

/// Release asset suffix for an AppImage install.
pub const APPIMAGE_ASSET: &str = "linux-x86_64.AppImage";

/// Release asset suffix for a deb install.
pub const DEB_ASSET: &str = "linux-x86_64.deb";

/// Release asset suffix for an rpm install.
pub const RPM_ASSET: &str = "linux-x86_64.rpm";

/// How this iris was installed.
enum Install {
    /// The AppImage at this path.
    AppImage(PathBuf),
    Package(Package),
}

/// This iris's install: the AppImage the AppImage runtime sets
/// `$APPIMAGE` to, else the package that installed this binary.
fn install() -> Result<Install, String> {
    if let Some(path) = std::env::var_os("APPIMAGE") {
        return Ok(Install::AppImage(path.into()));
    }
    let exe = crate::sys::exe::this().map_err(|e| format!("update: find this iris: {e}"))?;
    Package::of(&exe).map(Install::Package).ok_or_else(|| {
        format!(
            "update: {} is not from an AppImage, deb or rpm; install the release by hand",
            exe.display()
        )
    })
}

/// The release asset that updates this iris. An install that cannot
/// update gets the AppImage's, and `ready` refuses it.
pub fn asset() -> &'static str {
    match install() {
        Ok(Install::Package(package)) => asset_of(package),
        Ok(Install::AppImage(_)) | Err(_) => APPIMAGE_ASSET,
    }
}

/// The release asset that holds `package`.
fn asset_of(package: Package) -> &'static str {
    match package {
        Package::Deb => DEB_ASSET,
        Package::Rpm => RPM_ASSET,
    }
}

/// Ok when this install can replace itself: an AppImage can, and so
/// can a package whose package manager this process can run as root.
pub fn ready() -> Result<(), String> {
    match install()? {
        Install::AppImage(_) => Ok(()),
        Install::Package(package) => package.ready(),
    }
}

/// Replace the installed iris with the downloaded asset `file` and
/// restart; returns only on failure.
pub fn apply_file(file: &Path) -> Result<(), String> {
    let target = match install()? {
        Install::AppImage(target) => {
            replace(file, &target)?;
            target
        }
        Install::Package(package) => {
            package.install(file)?;
            PathBuf::from(package::BINARY)
        }
    };
    // An AppImage restarts from the AppImage, not sys::exe::this(): that
    // path is inside the old AppImage's mount, which serves the old
    // binary.
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

// WHY: docs/install.md prints the refusal of an iris that is neither
// an AppImage nor a package's binary; the test process, run from the
// target directory with no $APPIMAGE, is one. The asset cases close
// "a package install downloads the AppImage, and its package manager
// fails on it". The class closed by the `replace` cases is "a failed
// swap leaves the AppImage changed, or a half-written copy beside it":
// the updater then restarts the old AppImage. Not covered: ETXTBSY on
// the running file, which a rename does not hit.
#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::package::Package;
    use super::{asset_of, replace, APPIMAGE_ASSET};

    #[test]
    fn an_install_from_no_appimage_and_no_package_refuses_to_update() {
        assert!(std::env::var_os("APPIMAGE").is_none());
        let exe = crate::sys::exe::this().expect("this iris");
        assert_eq!(
            super::ready(),
            Err(format!(
                "update: {} is not from an AppImage, deb or rpm; install the release by hand",
                exe.display()
            ))
        );
        assert_eq!(super::asset(), APPIMAGE_ASSET);
    }

    /// Each package downloads the release asset of its own kind: the
    /// deb's package manager installs no rpm, and neither an AppImage.
    #[test]
    fn a_package_downloads_the_asset_its_package_manager_installs() {
        assert_eq!(asset_of(Package::Deb), "linux-x86_64.deb");
        assert_eq!(asset_of(Package::Rpm), "linux-x86_64.rpm");
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
