//! Auto-update: check GitHub for a newer release, download the
//! platform asset, apply it, restart the daemon.
//!
//! The release contract lives in `packaging/CONTRACT.md`: tags are
//! `v{semver}` and each kind of install updates from one asset,
//! published with a `.sha256` sidecar. `check` reads the latest-release
//! JSON and picks the asset for this install and its sidecar; `apply`
//! downloads the asset, deletes it unless its SHA-256 is the sidecar's,
//! stops the daemon, and hands the asset to `sys::install`, which swaps
//! it in. An asset a failed install left behind is used again when its
//! SHA-256 is the sidecar's. A swap that fails starts the stopped daemon
//! again.
//!
//! Update strategy per OS:
//!   Windows — an installed iris starts the NSIS installer with
//!             `/S /RUN`: it waits for iris.exe to be free, replaces
//!             it, and starts iris. A portable iris swaps its iris.exe
//!             for the one in the portable zip and relaunches.
//!   macOS   — mount the DMG, swap its iris.app for the bundle iris
//!             runs from, relaunch.
//!   Linux   — AppImage: overwrite the file `$APPIMAGE` points at and
//!             relaunch. A deb/rpm install is owned by the package
//!             manager, so `--update` reports that path instead of
//!             fighting it.
//!
//! `main` handles `--check-update`/`--update` client-side before the
//! forward probe; the Settings window calls `check` on a background
//! thread. Neither path blocks the UI loop.

mod checksum;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::sys::ipc::Quit;

/// The GitHub repo that publishes releases.
const REPO: &str = "santhreal/iris";

/// How long `--update` waits for the running daemon to exit after
/// `--quit`. A quitting daemon saves its recording first: the encoder
/// gives a wedged ffmpeg 10 s, and the segment join follows.
const QUIT_WAIT: Duration = Duration::from_secs(60);

/// A newer release and the asset that applies to this platform.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// The release's semver (tag minus the leading `v`).
    pub version: semver::Version,
    /// Direct download URL for this platform's asset.
    pub asset_url: String,
    /// The asset filename, used as the download's file name.
    pub asset_name: String,
    /// Direct download URL for the asset's `.sha256` sidecar.
    pub checksum_url: String,
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
    select(&json, &current_version(), crate::sys::install::asset())
}

/// The update `release`, a latest-release JSON object, offers over
/// `current`: `None` when its tag is not newer, the asset whose name
/// ends in `selector` and that asset's `.sha256` sidecar when it is. A
/// newer release that lacks either is an error: an asset with no
/// sidecar cannot be checked, so it is never downloaded.
fn select(
    release: &serde_json::Value,
    current: &semver::Version,
    selector: &str,
) -> Result<Option<UpdateInfo>, String> {
    let tag = str_field(release, "tag_name")
        .ok_or_else(|| "update: release has no tag_name".to_string())?;
    let version = semver::Version::parse(tag.trim_start_matches('v'))
        .map_err(|e| format!("update: bad tag {tag}: {e}"))?;
    if version <= *current {
        return Ok(None);
    }

    let assets = release
        .get("assets")
        .and_then(|a| a.as_array())
        .ok_or_else(|| "update: release has no assets".to_string())?;
    let (asset_name, asset) = assets
        .iter()
        .find_map(|a| {
            str_field(a, "name")
                .filter(|n| n.ends_with(selector))
                .map(|n| (n, a))
        })
        .ok_or_else(|| format!("update: no asset ending in {selector}"))?;
    let asset_url = str_field(asset, "browser_download_url")
        .ok_or_else(|| "update: asset has no download url".to_string())?;
    let sidecar = format!("{asset_name}.sha256");
    let checksum_url = assets
        .iter()
        .find(|a| str_field(a, "name") == Some(sidecar.as_str()))
        .and_then(|a| str_field(a, "browser_download_url"))
        .ok_or_else(|| {
            format!("update: release {tag} has no {sidecar} to check {asset_name} against")
        })?;

    Ok(Some(UpdateInfo {
        version,
        asset_url: asset_url.to_string(),
        asset_name: asset_name.to_string(),
        checksum_url: checksum_url.to_string(),
    }))
}

/// The string at `key` of a JSON object.
fn str_field<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

/// Download `info.asset_url` into this user's cache and return its
/// path. The cache is private to the user: a download in a shared temp
/// directory could be swapped by another local user between the write
/// and the install.
fn download(info: &UpdateInfo) -> Result<PathBuf, String> {
    let dir = iris_lib::dirs::cache_dir()
        .ok_or_else(|| "update: no cache directory for this user".to_string())?
        .join("update");
    download_into(info, &dir)
}

/// Download `info.asset_url` into `dir` and return its path. The
/// download's SHA-256 is the one its sidecar lists, or it is deleted
/// and this fails: a truncated, corrupt, or substituted file never
/// reaches the installer. A file already there with that SHA-256 is
/// the release asset, and is kept instead of downloaded again.
fn download_into(info: &UpdateInfo, dir: &Path) -> Result<PathBuf, String> {
    // The name comes from the release JSON; a separator in it would
    // move the write out of the update directory.
    let name = Path::new(&info.asset_name);
    if name.file_name() != Some(name.as_os_str()) {
        return Err(format!("update: bad asset name {:?}", info.asset_name));
    }
    let want = fetch_sidecar(info)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("update: create {}: {e}", dir.display()))?;
    let path = dir.join(name);
    if checksum::file_is(&path, &want) {
        return Ok(path);
    }
    // create_new never writes through a file or link already there.
    let _ = std::fs::remove_file(&path);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("update: create {}: {e}", path.display()))?;
    // The file closes before a failure deletes it: Windows deletes no
    // open file.
    let fetched = http_get(&info.asset_url)
        .map_err(|e| format!("update: GET {}: {e}", info.asset_url))
        .and_then(|resp| {
            let mut out = checksum::Sha256Writer::new(file);
            std::io::copy(&mut resp.into_reader(), &mut out)
                .map_err(|e| format!("update: download {}: {e}", info.asset_name))?;
            Ok(out.finish())
        })
        .and_then(|got| checksum::verify(&info.asset_name, &got, &want));
    if let Err(e) = fetched {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

/// The digest `info`'s sidecar lists for its asset.
fn fetch_sidecar(info: &UpdateInfo) -> Result<checksum::Digest, String> {
    let resp = http_get(&info.checksum_url)
        .map_err(|e| format!("update: GET {}: {e}", info.checksum_url))?;
    let mut text = String::new();
    resp.into_reader()
        .take(checksum::SIDECAR_MAX)
        .read_to_string(&mut text)
        .map_err(|e| format!("update: read {}.sha256: {e}", info.asset_name))?;
    checksum::parse_sidecar(&text, &info.asset_name)
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
/// clears all three. A swap that fails leaves the installed iris as it
/// was, and the daemon this stopped starts again from it.
pub fn apply(info: &UpdateInfo) -> Result<(), String> {
    crate::sys::install::ready()?;
    let file = download(info)?;
    let stopped = stop_daemon()?;
    let Err(e) = crate::sys::install::apply_file(&file) else {
        return Ok(());
    };
    Err(after_failed_swap(e, stopped, crate::sys::ipc::start_daemon))
}

/// Download and verify the update `info` names, then start a detached
/// `iris --update` to install it. The daemon, where the Settings window
/// runs, cannot install an update itself: the install stops the daemon
/// first. The `iris --update` finds the verified download in the update
/// directory and does not fetch it again.
pub fn hand_off(info: &UpdateInfo) -> Result<(), String> {
    crate::sys::install::ready()?;
    download(info)?;
    let exe = crate::sys::exe::this().map_err(|e| format!("update: find this iris: {e}"))?;
    crate::sys::detach::spawn(&exe, &["--update"])
        .map_err(|e| format!("update: start {} --update: {e}", exe.display()))
}

/// Quit the running daemon and wait for it to exit. A missing daemon is
/// a no-op. A daemon still running `QUIT_WAIT` after `--quit` fails the
/// update before the swap: replacing the binary under it would fail, or
/// leave no daemon running once it exits.
fn stop_daemon() -> Result<Quit, String> {
    match crate::sys::ipc::quit_daemon(QUIT_WAIT) {
        Quit::StillRunning => Err(format!(
            "update: the running iris still answers {} s after --quit; nothing was installed, \
             run iris --update again once it exits",
            QUIT_WAIT.as_secs()
        )),
        found => Ok(found),
    }
}

/// The error of a swap that failed with `e`, after `restart` starts the
/// daemon `stopped` found again. A daemon that did not run before the
/// update is not started.
fn after_failed_swap(e: String, stopped: Quit, restart: impl FnOnce() -> bool) -> String {
    if stopped != Quit::Exited {
        return e;
    }
    if restart() {
        format!("{e}; the iris already installed runs again")
    } else {
        format!(
            "{e}; the iris already installed did not start again, see {}",
            iris_lib::dirs::log_file().display()
        )
    }
}

// WHY: the classes closed here are "a release asset name steers the
// download's write outside the per-user update directory", "an install
// that cannot replace itself stops the daemon before it says so", "a
// download that is not the released file reaches the installer" (a
// dropped connection or a corrupt body replaced a working install),
// "a stale file in the update directory is installed or kept", and "a
// failed swap leaves no daemon running, or starts one that was not
// running". Not covered: the per-OS install that follows, and the
// `Quit::StillRunning` refusal, which needs a daemon that ignores
// `--quit`.
#[cfg(test)]
mod tests;
