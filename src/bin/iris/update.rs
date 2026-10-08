//! Auto-update: check GitHub for a newer release, download the
//! platform asset, apply it, restart the daemon.
//!
//! The release contract lives in `packaging/CONTRACT.md`: tags are
//! `v{semver}`, each kind of install on each architecture updates from
//! one asset, and every asset is published with a `.sha256` sidecar
//! and a minisign `.minisig` signature. `check` asks GitHub for the
//! release `update_channel` selects (the latest release on stable, the
//! newest by version on beta) and picks the asset for this install and
//! its two sidecars. `apply` downloads the asset, deletes it unless its
//! SHA-256 is the sidecar's and its signature verifies against
//! packaging/minisign.pub (`signature`), stops the daemon, and hands
//! the asset to `sys::install`, which swaps it in. An asset a failed
//! install left behind is used again when it passes both checks again.
//! A swap that fails starts the stopped daemon again.
//!
//! Update strategy per OS:
//!   Windows — an installed iris starts the NSIS installer with
//!             `/S /RUN`: it waits for iris.exe to be free, replaces
//!             it, and starts iris. A portable iris swaps its iris.exe
//!             for the one in the portable zip and relaunches.
//!   macOS   — mount the DMG, swap its iris.app for the bundle iris
//!             runs from, relaunch.
//!   Linux   — AppImage: overwrite the file `$APPIMAGE` points at and
//!             relaunch. A deb or rpm install runs apt-get or dnf as
//!             root, through pkexec unless it is root, on the release's
//!             deb or rpm, and relaunches /usr/bin/iris.
//!
//! `main` handles `--check-update`/`--update` client-side before the
//! forward probe, the Settings window calls `check` on a background
//! thread, and the daemon checks on a thread of its own (`schedule`).
//! None of them blocks the UI loop. Every check that gets an answer
//! records it in `update.json` (`state`), which `offered` and
//! `last_checked` read.

mod checksum;
mod schedule;
mod signature;
mod state;

pub(crate) use schedule::spawn as spawn_background_check;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use iris_lib::config::{Config, UpdateChannel};
use minisign_verify::PublicKey;
use serde::{Deserialize, Serialize};

use crate::sys::ipc::Quit;

/// The GitHub repo that publishes releases.
const REPO: &str = "santhreal/iris";

/// How many releases a beta check reads: GitHub lists them newest
/// first.
const BETA_PAGE: usize = 30;

/// How long `--update` waits for the running daemon to exit after
/// `--quit`. A quitting daemon saves its recording first: the encoder
/// gives a wedged ffmpeg 10 s, and the segment join follows.
const QUIT_WAIT: Duration = Duration::from_secs(60);

/// A newer release and the asset that applies to this platform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// The release's semver (tag minus the leading `v`).
    #[serde(with = "version_text")]
    pub version: semver::Version,
    /// Direct download URL for this platform's asset.
    pub asset_url: String,
    /// The asset filename, used as the download's file name.
    pub asset_name: String,
    /// Direct download URL for the asset's `.sha256` sidecar.
    pub checksum_url: String,
    /// Direct download URL for the asset's `.minisig` signature.
    pub signature_url: String,
}

/// A `semver::Version` as its text, in `update.json`.
mod version_text {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(version: &semver::Version, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(version)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<semver::Version, D::Error> {
        let text = String::deserialize(d)?;
        semver::Version::parse(&text).map_err(serde::de::Error::custom)
    }
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

/// Ask GitHub for the release the configured `update_channel` selects.
/// Returns `Ok(None)` when already current, `Ok(Some(info))` when a
/// newer version with a matching asset exists. The answer is recorded
/// in `update.json`.
pub fn check() -> Result<Option<UpdateInfo>, String> {
    let channel = Config::load().update_channel;
    let found = fetch(channel)?;
    let recorded = state::record_check(&state::path(), channel, found.as_ref(), SystemTime::now());
    if let Err(e) = recorded {
        iris_lib::ilog!("iris: {e}");
    }
    Ok(found)
}

/// The update GitHub offers this install on `channel`, unrecorded.
fn fetch(channel: UpdateChannel) -> Result<Option<UpdateInfo>, String> {
    let selector = crate::sys::install::asset();
    let current = current_version();
    let api = format!("https://api.github.com/repos/{REPO}/releases");
    match channel {
        UpdateChannel::Stable => get_json(&format!("{api}/latest"))?.map_or(Ok(None), |latest| {
            select_stable(&latest, &current, &selector)
        }),
        UpdateChannel::Beta => get_json(&format!("{api}?per_page={BETA_PAGE}"))?
            .map_or(Ok(None), |list| select_beta(&list, &current, &selector)),
    }
}

/// The JSON at `url`. `None` for a 404: a repo with no releases answers
/// it on /latest, and that is "up to date", not a failure.
fn get_json(url: &str) -> Result<Option<serde_json::Value>, String> {
    let resp = match http_get(url) {
        Ok(r) => r,
        Err(e) if matches!(*e, ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(format!("update: GET {url}: {e}")),
    };
    let body = resp
        .into_string()
        .map_err(|e| format!("update: read {url}: {e}"))?;
    serde_json::from_str(&body)
        .map(Some)
        .map_err(|e| format!("update: parse {url}: {e}"))
}

/// The update `latest`, GitHub's latest release, offers over `current`
/// on the stable channel. A draft or a prerelease offers nothing:
/// GitHub's latest release is neither, and a stable install never
/// installs a prerelease.
fn select_stable(
    latest: &serde_json::Value,
    current: &semver::Version,
    selector: &str,
) -> Result<Option<UpdateInfo>, String> {
    let flag = |key| latest.get(key).and_then(serde_json::Value::as_bool) == Some(true);
    let prerelease_tag = tag_version(latest).is_some_and(|v| !v.pre.is_empty());
    if flag("draft") || flag("prerelease") || prerelease_tag {
        return Ok(None);
    }
    select(latest, current, selector)
}

/// The update the beta channel offers over `current` from `list`,
/// GitHub's release list: of the releases that are not drafts and whose
/// tag is `v{semver}`, the newest by version that holds an asset ending
/// in `selector`, prerelease or not.
fn select_beta(
    list: &serde_json::Value,
    current: &semver::Version,
    selector: &str,
) -> Result<Option<UpdateInfo>, String> {
    let releases = list
        .as_array()
        .ok_or_else(|| "update: the release list is not a JSON array".to_string())?;
    let newest = releases
        .iter()
        .filter(|r| r.get("draft").and_then(serde_json::Value::as_bool) != Some(true))
        .filter_map(|r| Some((tag_version(r)?, r)))
        .filter(|(version, r)| version > current && find_asset(r, selector).is_some())
        .max_by(|a, b| a.0.cmp(&b.0));
    match newest {
        Some((_, release)) => select(release, current, selector),
        None => Ok(None),
    }
}

/// The version a release's `v{semver}` tag states; `None` for any other
/// tag.
fn tag_version(release: &serde_json::Value) -> Option<semver::Version> {
    semver::Version::parse(str_field(release, "tag_name")?.strip_prefix('v')?).ok()
}

/// The asset of `release` whose name ends in `selector`, and its name.
fn find_asset<'a>(
    release: &'a serde_json::Value,
    selector: &str,
) -> Option<(&'a str, &'a serde_json::Value)> {
    release.get("assets")?.as_array()?.iter().find_map(|a| {
        str_field(a, "name")
            .filter(|n| n.ends_with(selector))
            .map(|n| (n, a))
    })
}

/// The update `release`, a release JSON object, offers over `current`:
/// `None` when its tag is not newer, the asset whose name ends in
/// `selector` with that asset's `.sha256` sidecar and `.minisig`
/// signature when it is. A newer release that lacks any of the three is
/// an error: an asset that cannot be checked is never downloaded.
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
    let (asset_name, asset) = find_asset(release, selector)
        .ok_or_else(|| format!("update: no asset ending in {selector}"))?;
    let asset_url = str_field(asset, "browser_download_url")
        .ok_or_else(|| "update: asset has no download url".to_string())?;
    let sidecar_url = |suffix: &str| {
        let sidecar = format!("{asset_name}{suffix}");
        assets
            .iter()
            .find(|a| str_field(a, "name") == Some(sidecar.as_str()))
            .and_then(|a| str_field(a, "browser_download_url"))
            .map(str::to_string)
            .ok_or_else(|| {
                format!("update: release {tag} has no {sidecar} to check {asset_name} against")
            })
    };

    Ok(Some(UpdateInfo {
        version,
        asset_url: asset_url.to_string(),
        asset_name: asset_name.to_string(),
        checksum_url: sidecar_url(".sha256")?,
        signature_url: sidecar_url(".minisig")?,
    }))
}

/// The string at `key` of a JSON object.
fn str_field<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}

/// The update the last check found for the configured channel, when it
/// is newer than this iris (`state::State::offer_for`). Reads
/// `update.json`.
pub fn offered() -> Option<UpdateInfo> {
    state::State::load(&state::path()).offer_for(Config::load().update_channel, &current_version())
}

/// Download `info.asset_url` into this user's cache and return its
/// path. The cache is private to the user: a download in a shared temp
/// directory could be swapped by another local user between the write
/// and the install.
fn download(info: &UpdateInfo) -> Result<PathBuf, String> {
    let dir = iris_lib::dirs::cache_dir()
        .ok_or_else(|| "update: no cache directory for this user".to_string())?
        .join("update");
    download_into(info, &dir, &signature::release_key()?)
}

/// Download `info.asset_url` into `dir` and return its path. The
/// download's SHA-256 is the one its sidecar lists and its `.minisig`
/// is a signature by `key` for this asset and version, or it is deleted
/// and this fails: a truncated, corrupt, or substituted file never
/// reaches the installer. A file already there with that SHA-256 is
/// kept instead of downloaded again, and its signature is checked the
/// same way.
fn download_into(info: &UpdateInfo, dir: &Path, key: &PublicKey) -> Result<PathBuf, String> {
    // The name comes from the release JSON; a separator in it would
    // move the write out of the update directory.
    let name = Path::new(&info.asset_name);
    if name.file_name() != Some(name.as_os_str()) {
        return Err(format!("update: bad asset name {:?}", info.asset_name));
    }
    let want = fetch_sidecar(info)?;
    let minisig = fetch_signature(info)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("update: create {}: {e}", dir.display()))?;
    let path = dir.join(name);
    let fetched = if checksum::file_is(&path, &want) {
        Ok(())
    } else {
        // create_new never writes through a file or link already there.
        let _ = std::fs::remove_file(&path);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("update: create {}: {e}", path.display()))?;
        // The file closes before a failure deletes it: Windows deletes
        // no open file.
        http_get(&info.asset_url)
            .map_err(|e| format!("update: GET {}: {e}", info.asset_url))
            .and_then(|resp| {
                let mut out = checksum::Sha256Writer::new(file);
                std::io::copy(&mut resp.into_reader(), &mut out)
                    .map_err(|e| format!("update: download {}: {e}", info.asset_name))?;
                Ok(out.finish())
            })
            .and_then(|got| checksum::verify(&info.asset_name, &got, &want))
    };
    let checked = fetched
        .and_then(|()| signature::verify(&path, &minisig, key, &info.asset_name, &info.version));
    if let Err(e) = checked {
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

/// The text of `info`'s `.minisig` signature.
fn fetch_signature(info: &UpdateInfo) -> Result<String, String> {
    let resp = http_get(&info.signature_url)
        .map_err(|e| format!("update: GET {}: {e}", info.signature_url))?;
    let mut text = String::new();
    resp.into_reader()
        .take(signature::SIGNATURE_MAX)
        .read_to_string(&mut text)
        .map_err(|e| format!("update: read {}.minisig: {e}", info.asset_name))?;
    Ok(text)
}

/// Apply a downloaded update and restart the daemon.
///
/// This never returns on success: the platform swap ends by relaunching
/// the binary and exiting this process. It returns `Err` when the swap
/// could not be staged, leaving the running install untouched.
///
/// Order matters: check that this install can replace itself, then
/// download and verify (a refusal, a network failure, or a download
/// that fails its checksum or signature must not stop a working
/// daemon), then stop the daemon, then swap. The daemon holds the IPC
/// socket and, on Windows, locks its own exe; on Linux a running
/// AppImage cannot be overwritten (ETXTBSY). Stopping it before the swap
/// clears all three. A swap that fails leaves the installed iris as it
/// was, and the daemon this stopped starts again from it.
pub fn apply(info: &UpdateInfo) -> Result<(), String> {
    crate::sys::install::ready()?;
    install(
        info,
        download,
        stop_daemon,
        crate::sys::install::apply_file,
        crate::sys::ipc::start_daemon,
    )
}

/// `apply` after the readiness check: `fetch` the verified asset, `stop`
/// the daemon, `swap` the asset in, and after a failed swap `restart`
/// the daemon `stop` stopped. A download that fails a check ends the
/// update before `stop` runs.
fn install(
    info: &UpdateInfo,
    fetch: impl FnOnce(&UpdateInfo) -> Result<PathBuf, String>,
    stop: impl FnOnce() -> Result<Quit, String>,
    swap: impl FnOnce(&Path) -> Result<(), String>,
    restart: impl FnOnce() -> bool,
) -> Result<(), String> {
    let file = fetch(info)?;
    let stopped = stop()?;
    let Err(e) = swap(&file) else {
        return Ok(());
    };
    Err(after_failed_swap(e, stopped, restart))
}

/// Download and verify the update `info` states, then start a detached
/// `iris --update` to install it. The daemon, where the Settings window
/// runs, cannot install an update itself: the install stops the daemon
/// first. The `iris --update` finds the verified download in the update
/// directory, checks it again, and does not fetch it again.
pub fn hand_off(info: &UpdateInfo) -> Result<(), String> {
    crate::sys::install::ready()?;
    download(info)?;
    let exe = crate::sys::exe::this().map_err(|e| format!("update: find this iris: {e}"))?;
    crate::sys::detach::spawn(&exe, &["--update"])
        .map_err(|e| format!("update: start {} --update: {e}", exe.display()))
}

/// Whether a hand-off the tray or an update notice started is running.
static HANDING_OFF: AtomicBool = AtomicBool::new(false);

/// Hand off the update `offered` returns on a thread of its own: the
/// Install of the tray and of the update notice. A failure shows in a
/// notice. An Install while a hand-off runs does nothing.
pub(crate) fn install_offered() -> Result<(), String> {
    let info = offered().ok_or_else(|| {
        "update: no newer release is known; check for updates in Settings".to_string()
    })?;
    if HANDING_OFF.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    std::thread::Builder::new()
        .name("iris-update-install".into())
        .spawn(move || {
            let result = hand_off(&info);
            HANDING_OFF.store(false, Ordering::Release);
            if let Err(e) = result {
                crate::daemon::report_failure("Update failed", e);
            }
        })
        .map(drop)
        .map_err(|e| {
            HANDING_OFF.store(false, Ordering::Release);
            format!("update: start the install thread: {e}")
        })
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
