//! Windows: an installed iris updates through the NSIS installer,
//! started silent and detached. A portable iris, one unpacked from the
//! release zip, swaps its iris.exe for the one in the new zip.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

/// Release asset suffix for an installed iris. `{arch}` is this
/// build's architecture (`super::for_this_arch`).
pub const SETUP_ASSET: &str = "windows-{arch}-setup.exe";

/// Release asset suffix for a portable iris.
pub const PORTABLE_ASSET: &str = "windows-{arch}-portable.zip";

/// The installer's options (packaging/windows/iris.nsi): `/S` runs it
/// silent, and `/RUN` starts iris when the installation ends, whether
/// it succeeded or failed.
const OPTIONS: [&str; 2] = ["/S", "/RUN"];

/// The directory in the portable zip that holds iris.exe
/// (packaging/windows/build_portable.sh).
const ZIP_DIR: &str = "iris";

/// The iris.exe this process runs.
fn exe() -> Result<PathBuf, String> {
    crate::sys::exe::this().map_err(|e| format!("update: find this iris: {e}"))
}

/// Whether `exe` was installed by the installer, which writes
/// uninstall.exe beside iris.exe. The portable zip holds none.
fn installed(exe: &Path) -> bool {
    exe.with_file_name("uninstall.exe").is_file()
}

/// The release asset that updates this iris.
pub fn asset() -> String {
    super::for_this_arch(match exe() {
        Ok(exe) if !installed(&exe) => PORTABLE_ASSET,
        _ => SETUP_ASSET,
    })
}

/// Ok for an installed iris: the installer replaces any install, and
/// on an upgrade it installs into the directory the last installation
/// used. A portable iris is ready when its directory takes the staging
/// directory the swap unpacks into, so a folder this account cannot
/// write fails before the download and before the daemon stops.
pub fn ready() -> Result<(), String> {
    let exe = exe()?;
    if installed(&exe) {
        return Ok(());
    }
    writable(&staging(&exe)?)
}

/// The directory beside `exe` a portable update unpacks into.
fn staging(exe: &Path) -> Result<PathBuf, String> {
    exe.parent()
        .map(|dir| dir.join(".iris-update"))
        .ok_or_else(|| format!("update: {} has no directory", exe.display()))
}

/// Ok when `staging` can be created: create it and delete it again.
fn writable(staging: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir(staging)
        .and_then(|()| std::fs::remove_dir(staging))
        .map_err(|e| {
            format!(
                "update: cannot write {}: {e}; move the portable iris to a folder this account \
                 can write, or install it with the setup",
                staging.display()
            )
        })
}

/// Replace this iris with the downloaded asset `file` and restart;
/// returns only on failure.
///
/// Windows denies write access to the file of a running program, and
/// this process may run the installed iris.exe. The installer waits
/// until no process runs that file before it writes it, so this process
/// starts the installer detached and exits. A portable iris renames its
/// running iris.exe aside, which Windows allows, and moves the new one
/// in (`swap_portable`).
pub fn apply_file(file: &Path) -> Result<(), String> {
    let exe = exe()?;
    if installed(&exe) {
        crate::sys::detach::spawn(file, &OPTIONS)
            .map_err(|e| format!("update: start the installer {}: {e}", file.display()))?;
        std::process::exit(0);
    }
    swap_portable(file, &exe)?;
    super::relaunch(&exe)
}

/// Replace `exe` with the iris.exe in the portable zip `zip`: unpack
/// the zip into a staging directory beside `exe`, rename `exe` to
/// iris.exe.old, and move the new file to `exe`. A failure leaves `exe`
/// as it was and deletes the staging directory.
fn swap_portable(zip: &Path, exe: &Path) -> Result<(), String> {
    let staging = staging(exe)?;
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)
        .map_err(|e| format!("update: create {}: {e}", staging.display()))?;
    let swapped = unpack(zip, &staging).and_then(|new| replace(&new, exe));
    let _ = std::fs::remove_dir_all(&staging);
    swapped
}

/// Unpack `zip` into `into` with the tar.exe Windows ships (bsdtar,
/// which reads zip) and return the iris.exe it holds.
fn unpack(zip: &Path, into: &Path) -> Result<PathBuf, String> {
    let tar = std::env::var_os("SystemRoot")
        .map(|root| Path::new(&root).join(r"System32\tar.exe"))
        .ok_or_else(|| "update: SystemRoot is not set; tar.exe cannot be found".to_string())?;
    let out = Command::new(&tar)
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(into)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("update: run {}: {e}", tar.display()))?;
    if !out.status.success() {
        return Err(format!(
            "update: unpack {}: {}",
            zip.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let new = into.join(ZIP_DIR).join("iris.exe");
    if !new.is_file() {
        return Err(format!(
            "update: {} holds no {ZIP_DIR}\\iris.exe",
            zip.display()
        ));
    }
    Ok(new)
}

/// Rename `exe` to iris.exe.old and move `new` to `exe`. When the move
/// fails, `exe` is renamed back.
fn replace(new: &Path, exe: &Path) -> Result<(), String> {
    let old = aside(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("update: move {} aside: {e}", exe.display()))?;
    if let Err(e) = std::fs::rename(new, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("update: replace {}: {e}", exe.display()));
    }
    Ok(())
}

/// Where a portable update moves the running `exe`: iris.exe.old.
fn aside(exe: &Path) -> PathBuf {
    exe.with_extension("exe.old")
}

/// Delete the iris.exe.old a portable update left beside this iris. The
/// updated iris runs this at start; while the `iris --update` that
/// renamed it still runs, the delete fails and the file stays until the
/// next start.
pub fn tidy() {
    if let Ok(exe) = exe() {
        let _ = std::fs::remove_file(aside(&exe));
    }
}

// WHY: the classes closed here are "a portable iris updates through the
// installer, which installs a second copy elsewhere and leaves the
// portable one old", and "a failed portable swap leaves no iris.exe, or
// a staging directory, behind". Not covered: the relaunch, and a rename
// Windows refuses because another program holds iris.exe open.
#[cfg(test)]
mod tests;
