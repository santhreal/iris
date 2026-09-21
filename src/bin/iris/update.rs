//! Auto-update: check GitHub for a newer release, download the
//! platform asset, apply it, restart the daemon.
//!
//! The release contract lives in `packaging/CONTRACT.md`: tags are
//! `v{semver}` and each platform has one asset name. `check` reads the
//! latest-release JSON and picks the asset for this OS; `apply` does
//! the platform-specific swap.
//!
//! Update strategy per OS:
//!   Windows — run the NSIS installer silent (`/S`), which overwrites
//!             the exe and re-registers autostart; then relaunch.
//!   macOS   — mount the DMG, replace `/Applications/iris.app`, relaunch.
//!   Linux   — AppImage: overwrite the file `$APPIMAGE` points at and
//!             relaunch. A deb/rpm install is owned by the package
//!             manager, so `--update` reports that path instead of
//!             fighting it.
//!
//! `main` handles `--check-update`/`--update` client-side before the
//! forward probe; the Settings window calls `check` on a background
//! thread. Neither path blocks the UI loop.

use std::path::{Path, PathBuf};

/// The GitHub repo that publishes releases.
const REPO: &str = "santhreal/iris";

/// A newer release and the asset that applies to this platform.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// The release's semver (tag minus the leading `v`).
    pub version: semver::Version,
    /// Direct download URL for this platform's asset.
    pub asset_url: String,
    /// The asset filename, used for the temp download.
    pub asset_name: String,
}

/// The currently running version, from Cargo.toml at build time.
pub fn current_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| semver::Version::new(0, 0, 0))
}

/// The asset filename suffix that identifies this platform's installer.
/// Matched against `assets[].name` in the release JSON.
fn asset_selector() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "windows-x86_64-setup.exe"
    }
    #[cfg(target_os = "macos")]
    {
        "macos-universal.dmg"
    }
    #[cfg(target_os = "linux")]
    {
        // AppImage is the only self-updatable Linux artifact; a deb/rpm
        // install defers to the package manager in `apply`.
        "linux-x86_64.AppImage"
    }
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

    let selector = asset_selector();
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

/// Download `info.asset_url` to a temp file and return its path.
fn download(info: &UpdateInfo) -> Result<PathBuf, String> {
    let resp = http_get(&info.asset_url)
        .map_err(|e| format!("update: GET {}: {e}", info.asset_url))?;
    let mut reader = resp.into_reader();
    let dir = std::env::temp_dir().join("iris-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("update: temp dir: {e}"))?;
    let path = dir.join(&info.asset_name);
    let mut file =
        std::fs::File::create(&path).map_err(|e| format!("update: create {}: {e}", path.display()))?;
    std::io::copy(&mut reader, &mut file)
        .map_err(|e| format!("update: download {}: {e}", info.asset_name))?;
    Ok(path)
}

/// Apply a downloaded update and restart the daemon.
///
/// This never returns on success: the platform swap ends by relaunching
/// the binary and exiting this process. It returns `Err` when the swap
/// could not be staged, leaving the running install untouched.
///
/// Order matters: download first (a network failure must not kill a
/// working install), then stop the daemon, then swap. The daemon holds
/// the IPC socket and, on Windows, locks its own exe; on Linux a running
/// AppImage cannot be overwritten (ETXTBSY). Stopping it before the swap
/// clears all three.
pub fn apply(info: &UpdateInfo) -> Result<(), String> {
    let file = download(info)?;
    stop_daemon();
    apply_file(&file, info)
}

/// Ask the running daemon to quit and wait for its socket to free. A
/// missing daemon is a no-op. This runs before the file swap so the
/// binary is not locked or busy when it is replaced.
fn stop_daemon() {
    crate::sys::ipc::send_to_daemon(&["--quit".to_string()]);
    crate::sys::ipc::wait_for_daemon_exit(5000);
}

#[cfg(target_os = "windows")]
fn apply_file(file: &Path, _info: &UpdateInfo) -> Result<(), String> {
    // NSIS silent install over the running exe, then relaunch. The
    // installer re-registers autostart and the Start Menu shortcut.
    let status = std::process::Command::new(file)
        .arg("/S")
        .status()
        .map_err(|e| format!("update: run installer: {e}"))?;
    if !status.success() {
        return Err(format!("update: installer exited {status}"));
    }
    relaunch()
}

#[cfg(target_os = "macos")]
fn apply_file(file: &Path, _info: &UpdateInfo) -> Result<(), String> {
    // Mount the DMG, copy iris.app over /Applications, unmount, relaunch.
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
        .find_map(|l| l.split('\t').last().map(str::trim).filter(|s| s.starts_with('/')))
        .ok_or_else(|| "update: no mount point in hdiutil output".to_string())?;
    let src = Path::new(mount).join("iris.app");
    let dst = Path::new("/Applications/iris.app");
    let copy = std::process::Command::new("cp")
        .args(["-R"])
        .arg(&src)
        .arg(dst)
        .status();
    let _ = std::process::Command::new("hdiutil")
        .args(["detach", mount])
        .status();
    match copy {
        Ok(s) if s.success() => relaunch(),
        Ok(s) => Err(format!("update: copy app exited {s}")),
        Err(e) => Err(format!("update: copy app: {e}")),
    }
}

#[cfg(target_os = "linux")]
fn apply_file(file: &Path, _info: &UpdateInfo) -> Result<(), String> {
    // Only an AppImage install can self-update: replace the file the
    // APPIMAGE env var points at. A deb/rpm install is owned by the
    // package manager — report that instead of overwriting /usr/bin.
    let appimage = std::env::var("APPIMAGE").map_err(|_| {
        "update: not an AppImage install; update via apt/dnf".to_string()
    })?;
    let target = PathBuf::from(appimage);
    std::fs::copy(file, &target)
        .map_err(|e| format!("update: replace {}: {e}", target.display()))?;
    // Preserve the executable bit on the new image.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&target)
            .map_err(|e| format!("update: stat {}: {e}", target.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms)
            .map_err(|e| format!("update: chmod {}: {e}", target.display()))?;
    }
    relaunch()
}

/// Spawn the new binary and exit this process. The daemon is already
/// stopped (see `apply`), so the fresh `iris` binds the socket and
/// becomes the daemon rather than forwarding to a stale instance.
fn relaunch() -> ! {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("iris"));
    let _ = std::process::Command::new(exe).spawn();
    std::process::exit(0);
}
