//! Linux: the AppImage, renamed over `$APPIMAGE`, and the deb and the
//! rpm, each installed over the installed package by its package
//! manager (`package`).

mod package;

use std::path::{Path, PathBuf};

use package::Package;

/// Release asset suffix for an AppImage install. `{arch}` is this
/// build's architecture (`super::for_this_arch`).
pub const APPIMAGE_ASSET: &str = "linux-{arch}.AppImage";

/// Release asset suffix for a deb install.
pub const DEB_ASSET: &str = "linux-{arch}.deb";

/// Release asset suffix for an rpm install.
pub const RPM_ASSET: &str = "linux-{arch}.rpm";

/// How this iris was installed.
enum Install {
    /// The AppImage at this path.
    AppImage(PathBuf),
    Package(Package),
}

/// This iris's install: the AppImage it runs from, else the package
/// that installed this binary.
fn install() -> Result<Install, String> {
    let exe = crate::sys::exe::this().map_err(|e| format!("update: find this iris: {e}"))?;
    if let Some(appimage) = crate::sys::exe::appimage(&exe) {
        return Ok(Install::AppImage(appimage));
    }
    Package::of(&exe).map(Install::Package).ok_or_else(|| {
        format!(
            "update: {} is not from an AppImage, deb or rpm; install the release by hand",
            exe.display()
        )
    })
}

/// The release asset that updates this iris. An install that cannot
/// update gets the AppImage's, and `ready` refuses it.
pub fn asset() -> String {
    match install() {
        Ok(Install::Package(package)) => asset_of(package),
        Ok(Install::AppImage(_)) | Err(_) => super::for_this_arch(APPIMAGE_ASSET),
    }
}

/// The release asset that holds `package`.
fn asset_of(package: Package) -> String {
    super::for_this_arch(match package {
        Package::Deb => DEB_ASSET,
        Package::Rpm => RPM_ASSET,
    })
}

/// Ok when this install can replace itself: an AppImage in a directory
/// this account can write, and a package whose package manager this
/// process can run as root.
pub fn ready() -> Result<(), String> {
    match install()? {
        Install::AppImage(target) => writable(&staged(&target)),
        Install::Package(package) => package.ready(),
    }
}

/// Where `replace` copies the new AppImage before it renames it over
/// the AppImage `target`.
fn staged(target: &Path) -> PathBuf {
    target.with_extension("new")
}

/// Ok when `staged` can be created: create it and delete it again. An
/// AppImage in a directory this account cannot write, such as /opt,
/// fails here, before the download and before the daemon stops.
fn writable(staged: &Path) -> Result<(), String> {
    std::fs::File::create(staged).map_err(|e| {
        format!(
            "update: cannot write {}: {e}; move the AppImage to a folder this account can \
             write, or install the deb or rpm",
            staged.display()
        )
    })?;
    let _ = std::fs::remove_file(staged);
    Ok(())
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
    let tmp = staged(target);
    let swapped = std::fs::copy(file, &tmp)
        .map_err(|e| format!("update: stage {}: {e}", tmp.display()))
        .and_then(|_| {
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("update: chmod {}: {e}", tmp.display()))
        })
        .and_then(|()| {
            std::fs::rename(&tmp, target)
                .map_err(|e| format!("update: replace {}: {e}", target.display()))
        });
    if swapped.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    swapped
}

// WHY: docs/install.md prints the refusal of an iris that is neither
// an AppImage nor a package's binary; the test process, run from the
// target directory, is one. The classes closed: "an AppImage in a
// directory this account cannot write stops the daemon before it says
// so", "a package install downloads the AppImage, and its package
// manager fails on it", and "a failed swap leaves the AppImage changed,
// or a half-written copy beside it": the updater then restarts the old
// AppImage. sys::exe::appimage closes "an iris started from another
// AppImage's program overwrites that AppImage". Not covered: ETXTBSY on
// the running file, which a rename does not hit.
#[cfg(test)]
mod tests;
