//! macOS: the DMG's iris.app, swapped in for the bundle this iris runs
//! from.

use std::ffi::CString;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[cfg(test)]
mod tests;

/// Release asset suffix for this platform.
pub const ASSET: &str = "macos-universal.dmg";

/// Where `install` copies the new bundle before the swap: a sibling of
/// the installed one, so the two share a volume and one rename swaps
/// them. The dot keeps it out of Finder and Launchpad.
const STAGED: &str = ".iris.app.update";

/// `hdiutil attach` fails with this while another disk image operation
/// holds the DiskImages framework, such as a scan of an image just
/// written or another image being attached.
const BUSY: &str = "Resource temporarily unavailable";
/// Tries of an attach that fails with `BUSY`, `ATTACH_WAIT` apart.
const ATTACH_TRIES: u32 = 5;
const ATTACH_WAIT: Duration = Duration::from_secs(1);

/// The `.app` bundle this iris runs from. A binary outside a bundle,
/// as a `cargo build` leaves it, has no bundle to replace.
fn bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| format!("update: locate the running iris: {e}"))?;
    bundle_of(&exe).ok_or_else(|| {
        format!(
            "update: {} is not inside an iris.app bundle; install the release DMG by hand",
            exe.display()
        )
    })
}

/// The bundle of an executable at `<bundle>.app/Contents/MacOS/<exe>`.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let inside = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    inside.then(|| bundle.to_path_buf())
}

/// Ok when this iris runs from an `.app` bundle, the one an update
/// replaces.
pub fn ready() -> Result<(), String> {
    bundle().map(drop)
}

/// Replace the installed iris with the downloaded asset `file` and
/// restart; returns only on failure.
pub fn apply_file(file: &Path) -> Result<(), String> {
    let target = bundle()?;
    install(file, &target)?;
    super::relaunch(&target.join("Contents/MacOS/iris"))
}

/// Put the `iris.app` of the DMG `file` in place of the bundle
/// `target`. The new bundle is copied beside `target`, then the two
/// trade places in one rename (`RENAME_SWAP`): a failure at any step
/// leaves `target` as it was, and the running iris is never left
/// without a bundle.
fn install(file: &Path, target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("update: {} has no parent directory", target.display()))?;
    let staged = parent.join(STAGED);
    let _ = std::fs::remove_dir_all(&staged);
    // A mount point of its own: the DMG's volume name, iris, may
    // already be mounted at /Volumes/iris.
    let mount = file.with_extension("mount");
    std::fs::create_dir_all(&mount)
        .map_err(|e| format!("update: create {}: {e}", mount.display()))?;
    let attached = retry_busy(ATTACH_TRIES, ATTACH_WAIT, || {
        run(Command::new("hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&mount)
            .arg(file))
    });
    let installed = attached.and_then(|()| {
        // ditto copies a bundle whole: symlinks, extended attributes,
        // and the code signature.
        let copied = run(Command::new("ditto")
            .arg(mount.join("iris.app"))
            .arg(&staged));
        // -force: Spotlight may still be reading the volume, and
        // nothing more is read from it.
        let _ = Command::new("hdiutil")
            .args(["detach", "-force", "-quiet"])
            .arg(&mount)
            .status();
        copied?;
        let exe = staged.join("Contents/MacOS/iris");
        if !exe.is_file() {
            return Err(format!(
                "update: the iris.app in {} has no Contents/MacOS/iris",
                file.display()
            ));
        }
        swap(&staged, target).map_err(|e| {
            format!(
                "update: swap {} for {}: {e}",
                staged.display(),
                target.display()
            )
        })
    });
    let _ = std::fs::remove_dir(&mount);
    // After the swap the staged path holds the old bundle; after a
    // failure, the partial copy.
    let _ = std::fs::remove_dir_all(&staged);
    installed
}

/// Run `op` until it succeeds, fails with an error other than `BUSY`,
/// or has run `tries` times, `wait` apart. Returns its last result.
fn retry_busy(
    tries: u32,
    wait: Duration,
    mut op: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    let mut left = tries.max(1);
    loop {
        left -= 1;
        match op() {
            Err(e) if left > 0 && e.contains(BUSY) => std::thread::sleep(wait),
            done => return done,
        }
    }
}

/// Run `cmd`. An exit status other than 0 is an error with its stderr.
fn run(cmd: &mut Command) -> Result<(), String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let out = cmd
        .output()
        .map_err(|e| format!("update: run {program}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "update: {program} {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr).trim()
    ))
}

/// Exchange the directory entries `a` and `b` in one rename.
fn swap(a: &Path, b: &Path) -> io::Result<()> {
    let a = CString::new(a.as_os_str().as_bytes())?;
    let b = CString::new(b.as_os_str().as_bytes())?;
    // SAFETY: both are NUL-terminated paths that outlive the call.
    if unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
