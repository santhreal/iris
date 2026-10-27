//! Windows: an installed iris updates through the NSIS installer,
//! started silent and detached. A portable iris, one unpacked from the
//! release zip, swaps the complete application payload for the one in
//! the new zip.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use super::recovery;

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

/// Non-iris files every portable package contains. Keeping the complete
/// list here makes an incomplete update fail before any file moves.
const RUNTIME_PAYLOAD: [&str; 13] = [
    "ffmpeg.exe",
    "licenses/ffmpeg/FFmpeg.txt",
    "licenses/ffmpeg/FFmpeg-LICENSE.txt",
    "licenses/ffmpeg/FFMPEG-SOURCE.txt",
    "licenses/ffmpeg/x264.txt",
    "licenses/ffmpeg/libvpx.txt",
    "licenses/ffmpeg/Opus.txt",
    "licenses/ffmpeg/zlib.txt",
    "licenses/ffmpeg/nv-codec-headers.txt",
    "licenses/ffmpeg/LLVM.txt",
    "licenses/ffmpeg/compiler-rt.txt",
    "licenses/ffmpeg/compiler-rt-CREDITS.txt",
    "licenses/ffmpeg/mingw-w64-runtime.txt",
];

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
    writable(&staging(&exe)?, &exe)
}

/// The directory beside `exe` a portable update unpacks into.
fn staging(exe: &Path) -> Result<PathBuf, String> {
    exe.parent()
        .map(|dir| dir.join(".iris-update"))
        .ok_or_else(|| format!("update: {} has no directory", exe.display()))
}

/// Ok when `staging` can be created: create it and delete it again.
fn writable(staging: &Path, exe: &Path) -> Result<(), String> {
    recovery::refuse_recovery(staging, &aside(exe), exe)?;
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

/// Replace `exe` and its bundled files with the application in the
/// portable zip `zip`: unpack the zip into a staging directory beside
/// `exe`, replace the payload with rollback on failure, and rename the
/// running iris.exe aside last. A failure restores the old payload.
/// Staging and its recovery files remain if any restore fails.
fn swap_portable(zip: &Path, exe: &Path) -> Result<(), String> {
    let staging = staging(exe)?;
    recovery::refuse_recovery(&staging, &aside(exe), exe)?;
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)
        .map_err(|e| format!("update: create {}: {e}", staging.display()))?;
    let swapped = unpack(zip, &staging).and_then(|new| {
        let backup = staging.join("old");
        replace_payload(&new, exe, &backup)
    });
    recovery::finish_swap(&staging, &aside(exe), exe, swapped)
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
    let root = into.join(ZIP_DIR);
    let new = root.join("iris.exe");
    if !new.is_file() {
        return Err(format!(
            "update: {} holds no {ZIP_DIR}\\iris.exe",
            zip.display()
        ));
    }
    for required in RUNTIME_PAYLOAD {
        if !root.join(required).is_file() {
            return Err(format!(
                "update: {} holds no {ZIP_DIR}\\{required}",
                zip.display(),
            ));
        }
    }
    Ok(new)
}

/// Replace every file in the payload containing `new`, then replace
/// the running executable last. Older versions of each replaced file
/// go under `backup`; any failure restores all of them.
fn replace_payload(new: &Path, exe: &Path, backup: &Path) -> Result<(), String> {
    let root = new.parent().expect("unpack returns the payload iris.exe");
    let mut files = Vec::new();
    recovery::files_below(root, root, &mut files)?;
    files.sort();
    files.retain(|path| path != Path::new("iris.exe"));
    let app = exe.parent().expect("exe has a directory");
    let mut replaced = Vec::new();
    for relative in files {
        let target = app.join(&relative);
        match recovery::replace_with_backup(&root.join(&relative), &target, &backup.join(&relative))
        {
            Ok(old) => replaced.push((target, old)),
            Err(error) => {
                return Err(match recovery::rollback(&replaced) {
                    Ok(()) => error,
                    Err(recovery_error) => format!("{error}; {recovery_error}"),
                });
            }
        }
    }
    if let Err(error) = replace(new, exe) {
        return Err(match recovery::rollback(&replaced) {
            Ok(()) => error,
            Err(recovery_error) => format!("{error}; {recovery_error}"),
        });
    }
    Ok(())
}

/// Rename `exe` to iris.exe.old and move `new` to `exe`. When the move
/// fails, `exe` is renamed back.
fn replace(new: &Path, exe: &Path) -> Result<(), String> {
    recovery::replace_with_backup(new, exe, &aside(exe)).map(|_| ())
}

/// Where a portable update moves the running `exe`: iris.exe.old.
fn aside(exe: &Path) -> PathBuf {
    exe.with_extension("exe.old")
}

/// Delete the iris.exe.old and staging files a portable update left.
/// The updated iris runs this at start; while the `iris --update` that
/// renamed it still runs, the executable delete fails and the file
/// stays until the next start.
pub fn tidy() {
    if let Ok(exe) = exe() {
        tidy_at(&exe);
    }
}

fn tidy_at(exe: &Path) {
    if let Ok(staging) = staging(&exe) {
        recovery::tidy(&staging, &aside(exe), exe);
    }
}

// WHY: the classes closed here are "a portable iris updates through the
// installer, which installs a second copy elsewhere and leaves the
// portable one old", "a portable update leaves an old or missing
// bundled runtime", "a failed portable swap leaves the old payload
// replaced or a staging directory behind", and "cleanup or a retry
// deletes recovery files left by a failed rollback". Not covered: the
// relaunch, and an actual rename Windows refuses because another
// program holds iris.exe open.
#[cfg(test)]
mod tests;
