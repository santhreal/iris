//! Auto-update: check GitHub for a newer release, download the
//! platform asset, apply it, restart the daemon.
//!
//! The release contract lives in `packaging/CONTRACT.md`: tags are
//! `v{semver}` and each platform has one asset name. `check` reads the
//! latest-release JSON and picks the asset for this OS; `apply` hands
//! the download to `sys::install`, which swaps it in.
//!
//! Update strategy per OS:
//!   Windows — start the NSIS installer with `/S /RUN`: it waits for
//!             iris.exe to be free, replaces it, and starts iris.
//!   macOS   — mount the DMG, replace `/Applications/iris.app`, relaunch.
//!   Linux   — AppImage: overwrite the file `$APPIMAGE` points at and
//!             relaunch. A deb/rpm install is owned by the package
//!             manager, so `--update` reports that path instead of
//!             fighting it.
//!
//! `main` handles `--check-update`/`--update` client-side before the
//! forward probe; the Settings window calls `check` on a background
//! thread. Neither path blocks the UI loop.

use std::path::PathBuf;

/// The GitHub repo that publishes releases.
const REPO: &str = "santhreal/iris";

/// A newer release and the asset that applies to this platform.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// The release's semver (tag minus the leading `v`).
    pub version: semver::Version,
    /// Direct download URL for this platform's asset.
    pub asset_url: String,
    /// The asset filename, used as the download's file name.
    pub asset_name: String,
}

/// The currently running version, from Cargo.toml at build time.
pub fn current_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| semver::Version::new(0, 0, 0))
}

/// GET `url`, returning the `ureq::Error` boxed so the `Err` variant
/// stays small. Callers match the status to tell "no releases" (404)
/// from a real failure.
fn http_get(url: &str) -> Result<ureq::Response, Box<ureq::Error>> {
    ureq::get(url)
        .set("User-Agent", concat!("iris/", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(Box::new)
}

/// Query the latest release. Returns `Ok(None)` when already current,
/// `Ok(Some(info))` when a newer version with a matching asset exists.
pub fn check() -> Result<Option<UpdateInfo>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = match http_get(&url) {
        Ok(r) => r,
        // A repo with no releases answers 404 on /latest: that is "up
        // to date", not a failure. Any other error propagates.
        Err(e) if matches!(*e, ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(format!("update: GET {url}: {e}")),
    };
    let body = resp
        .into_string()
        .map_err(|e| format!("update: read release body: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("update: parse release JSON: {e}"))?;

    let tag = json
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "update: release has no tag_name".to_string())?;
    let version = semver::Version::parse(tag.trim_start_matches('v'))
        .map_err(|e| format!("update: bad tag {tag}: {e}"))?;

    if version <= current_version() {
        return Ok(None);
    }

    let selector = crate::sys::install::ASSET;
    let assets = json
        .get("assets")
        .and_then(|a| a.as_array())
        .ok_or_else(|| "update: release has no assets".to_string())?;
    let asset = assets
        .iter()
        .find(|a| {
            a.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.ends_with(selector))
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("update: no asset ending in {selector}"))?;

    let asset_url = asset
        .get("browser_download_url")
        .and_then(|u| u.as_str())
        .ok_or_else(|| "update: asset has no download url".to_string())?
        .to_string();
    let asset_name = asset
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("iris-update")
        .to_string();

    Ok(Some(UpdateInfo {
        version,
        asset_url,
        asset_name,
    }))
}

/// Download `info.asset_url` into this user's cache and return its
/// path. The cache is private to the user: a download in a shared temp
/// directory could be swapped by another local user between the write
/// and the install.
fn download(info: &UpdateInfo) -> Result<PathBuf, String> {
    // The name comes from the release JSON; a separator in it would
    // move the write out of the update directory.
    let name = std::path::Path::new(&info.asset_name);
    if name.file_name() != Some(name.as_os_str()) {
        return Err(format!("update: bad asset name {:?}", info.asset_name));
    }
    let dir = iris_lib::dirs::cache_dir()
        .ok_or_else(|| "update: no cache directory for this user".to_string())?
        .join("update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("update: create {}: {e}", dir.display()))?;
    let path = dir.join(name);
    // create_new never writes through a file or link already there.
    let _ = std::fs::remove_file(&path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("update: create {}: {e}", path.display()))?;
    let resp =
        http_get(&info.asset_url).map_err(|e| format!("update: GET {}: {e}", info.asset_url))?;
    std::io::copy(&mut resp.into_reader(), &mut file)
        .map_err(|e| format!("update: download {}: {e}", info.asset_name))?;
    Ok(path)
}

/// Apply a downloaded update and restart the daemon.
///
/// This never returns on success: the platform swap ends by relaunching
/// the binary and exiting this process. It returns `Err` when the swap
/// could not be staged, leaving the running install untouched.
///
/// Order matters: check that this install can replace itself, then
/// download (a refusal or a network failure must not stop a working
/// daemon), then stop the daemon, then swap. The daemon holds the IPC
/// socket and, on Windows, locks its own exe; on Linux a running
/// AppImage cannot be overwritten (ETXTBSY). Stopping it before the swap
/// clears all three.
pub fn apply(info: &UpdateInfo) -> Result<(), String> {
    crate::sys::install::ready()?;
    let file = download(info)?;
    stop_daemon();
    crate::sys::install::apply_file(&file)
}

/// Ask the running daemon to quit and wait for its socket to free. A
/// missing daemon is a no-op. This runs before the file swap so the
/// binary is not locked or busy when it is replaced.
fn stop_daemon() {
    crate::sys::ipc::send_to_daemon(&["--quit".to_string()]);
    crate::sys::ipc::wait_for_daemon_exit(5000);
}

// WHY: the classes closed here are "a release asset name steers the
// download's write outside the per-user update directory" and "an
// install that cannot replace itself stops the daemon before it says
// so": `iris --update` on a deb or rpm install downloaded the AppImage,
// quit the daemon, and only then failed. Not covered: the network
// fetch and the per-OS install that follow.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names_that_leave_the_update_dir_are_rejected() {
        for name in [
            "../iris.AppImage",
            "a/b.dmg",
            "/abs/setup.exe",
            "..",
            ".",
            "",
        ] {
            let info = UpdateInfo {
                version: semver::Version::new(9, 9, 9),
                asset_url: "https://invalid.example/asset".to_string(),
                asset_name: name.to_string(),
            };
            let err = download(&info).expect_err(name);
            assert!(err.starts_with("update: bad asset name"), "{name:?}: {err}");
        }
    }

    /// A deb or rpm install refuses before anything runs: no download,
    /// and the daemon keeps running. The asset name here would fail the
    /// download with a different error. An install that replaces itself
    /// has nothing to refuse.
    #[test]
    fn a_refused_install_fails_before_the_download() {
        let Err(refusal) = crate::sys::install::ready() else {
            return;
        };
        assert_eq!(
            refusal,
            "update: not an AppImage install; update via apt/dnf"
        );
        let info = UpdateInfo {
            version: semver::Version::new(9, 9, 9),
            asset_url: "https://invalid.example/asset".to_string(),
            asset_name: "../escape".to_string(),
        };
        assert_eq!(apply(&info).expect_err("refused"), refusal);
    }
}
