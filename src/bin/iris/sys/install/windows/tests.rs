use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

/// A zip at `dir\name` holding `entry` (`top\iris.exe`) with `body`,
/// made with the tar.exe the updater unpacks it with. `with_runtime`
/// adds the FFmpeg payload a current package requires.
fn zip_of(dir: &Path, name: &str, top: &str, body: &[u8]) -> PathBuf {
    zip_of_parts(dir, name, top, body, true)
}

fn zip_of_parts(dir: &Path, name: &str, top: &str, body: &[u8], with_runtime: bool) -> PathBuf {
    zip_of_filtered(dir, name, top, body, with_runtime, None)
}

fn zip_without(dir: &Path, name: &str, omitted: &str) -> PathBuf {
    zip_of_filtered(dir, name, ZIP_DIR, b"new", true, Some(omitted))
}

fn zip_of_filtered(
    dir: &Path,
    name: &str,
    top: &str,
    body: &[u8],
    with_runtime: bool,
    omitted: Option<&str>,
) -> PathBuf {
    let src = dir.join("src");
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(src.join(top)).expect("mkdir");
    std::fs::write(src.join(top).join("iris.exe"), body).expect("write");
    if with_runtime {
        for required in RUNTIME_PAYLOAD {
            if Some(required) == omitted {
                continue;
            }
            let path = src.join(top).join(required);
            std::fs::create_dir_all(path.parent().expect("payload parent")).expect("mkdir payload");
            if required == "ffmpeg.exe" {
                let mut ffmpeg = body.to_vec();
                ffmpeg.extend_from_slice(b"-ffmpeg");
                std::fs::write(path, ffmpeg).expect("write FFmpeg");
            } else {
                std::fs::write(path, body).expect("write notice");
            }
        }
    }
    let zip = dir.join(name);
    let tar =
        Path::new(&std::env::var_os("SystemRoot").expect("SystemRoot")).join(r"System32\tar.exe");
    let status = Command::new(tar)
        .arg("-a")
        .arg("-cf")
        .arg(&zip)
        .arg("-C")
        .arg(&src)
        .arg(top)
        .status()
        .expect("run tar.exe");
    assert!(status.success(), "tar.exe -a -cf {}", zip.display());
    zip
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

/// An install directory holding `iris.exe` with `old`.
fn install(old: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = dir.path().join("app");
    std::fs::create_dir(&app).expect("mkdir");
    let exe = app.join("iris.exe");
    std::fs::write(&exe, old).expect("write");
    (dir, exe)
}

#[test]
fn an_iris_with_an_uninstaller_beside_it_is_installed() {
    let (_dir, exe) = install(b"old");
    assert!(!installed(&exe));
    std::fs::write(exe.with_file_name("uninstall.exe"), b"").expect("write");
    assert!(installed(&exe));
    // The test binary has no uninstall.exe beside it: a portable iris,
    // which downloads the portable zip built for its architecture.
    let arch = std::env::consts::ARCH;
    assert_eq!(asset(), format!("windows-{arch}-portable.zip"));
    for kind in [SETUP_ASSET, PORTABLE_ASSET] {
        let name = crate::sys::install::for_this_arch(kind);
        assert!(
            crate::sys::install::in_contract(&name),
            "packaging/CONTRACT.md lists no iris-{{ver}}-{name}"
        );
    }
}

#[test]
fn a_portable_swap_puts_the_new_iris_in_place() {
    let (dir, exe) = install(b"old");
    let zip = zip_of(dir.path(), "new.zip", ZIP_DIR, b"new");
    swap_portable(&zip, &exe).expect("swap");
    assert_eq!(std::fs::read(&exe).expect("read"), b"new");
    assert_eq!(
        std::fs::read(exe.with_file_name("ffmpeg.exe")).expect("read FFmpeg"),
        b"new-ffmpeg"
    );
    assert_eq!(
        std::fs::read(
            exe.parent()
                .unwrap()
                .join("licenses/ffmpeg/FFMPEG-SOURCE.txt")
        )
        .expect("read source notice"),
        b"new"
    );
    assert_eq!(std::fs::read(aside(&exe)).expect("read"), b"old");
    assert_eq!(
        names(exe.parent().unwrap()),
        ["ffmpeg.exe", "iris.exe", "iris.exe.old", "licenses"]
    );
    // A second update replaces the iris.exe.old the first one left.
    let zip = zip_of(dir.path(), "newer.zip", ZIP_DIR, b"newer");
    swap_portable(&zip, &exe).expect("second swap");
    assert_eq!(std::fs::read(&exe).expect("read"), b"newer");
    assert_eq!(
        std::fs::read(exe.with_file_name("ffmpeg.exe")).expect("read FFmpeg"),
        b"newer-ffmpeg"
    );
    assert_eq!(
        std::fs::read(
            exe.parent()
                .unwrap()
                .join("licenses/ffmpeg/FFMPEG-SOURCE.txt")
        )
        .expect("read source notice"),
        b"newer"
    );
    assert_eq!(std::fs::read(aside(&exe)).expect("read"), b"new");
}

#[test]
fn a_payload_failure_restores_the_previous_runtime() {
    let (dir, exe) = install(b"old");
    std::fs::write(exe.with_file_name("ffmpeg.exe"), b"old-ffmpeg").expect("write old FFmpeg");
    // Directory creation for the nested notices fails after ffmpeg.exe
    // has been replaced. The swap must put the old executable back.
    std::fs::write(exe.parent().unwrap().join("licenses"), b"not a directory").expect("block");
    let zip = zip_of(dir.path(), "new.zip", ZIP_DIR, b"new");
    let error = swap_portable(&zip, &exe).expect_err("blocked license directory");
    assert!(error.starts_with("update: create "), "{error}");
    assert_eq!(std::fs::read(&exe).expect("read iris"), b"old");
    assert_eq!(
        std::fs::read(exe.with_file_name("ffmpeg.exe")).expect("read FFmpeg"),
        b"old-ffmpeg"
    );
    assert_eq!(
        std::fs::read(exe.parent().unwrap().join("licenses")).expect("read blocker"),
        b"not a directory"
    );
    assert_eq!(
        names(exe.parent().unwrap()),
        ["ffmpeg.exe", "iris.exe", "licenses"]
    );
}

#[test]
fn a_failed_rollback_preserves_recovery_against_cleanup_and_retries() {
    let (dir, exe) = install(b"old");
    let app = exe.parent().expect("app directory");
    let ffmpeg = exe.with_file_name("ffmpeg.exe");
    let notice = app.join("licenses/ffmpeg/FFMPEG-SOURCE.txt");
    std::fs::write(&ffmpeg, b"old-ffmpeg").expect("write old FFmpeg");
    std::fs::create_dir_all(notice.parent().expect("notice directory")).expect("mkdir notices");
    std::fs::write(&notice, b"old-notice").expect("write old notice");
    let zip = zip_of(dir.path(), "new.zip", ZIP_DIR, b"new");

    // ffmpeg.exe moves successfully. The next payload file fails to
    // move in but restores its own backup. Restoring ffmpeg.exe then
    // fails, leaving its only old copy under .iris-update\old.
    recovery::inject_renames(&[false, false, false, true, false, true]);
    let error = swap_portable(&zip, &exe).expect_err("injected rollback failure");
    assert!(error.contains("recovery file remains at"), "{error}");
    assert!(error.contains("recovery files remain"), "{error}");
    assert!(
        error.contains("restore them before running iris --update again"),
        "{error}"
    );
    let staging = staging(&exe).expect("staging");
    let backup = staging.join("old/ffmpeg.exe");
    assert_eq!(
        std::fs::read(&backup).expect("preserved backup"),
        b"old-ffmpeg"
    );
    assert_eq!(
        std::fs::read(&ffmpeg).expect("partially replaced FFmpeg"),
        b"new-ffmpeg"
    );
    assert_eq!(
        std::fs::read(&notice).expect("restored notice"),
        b"old-notice"
    );
    assert_eq!(std::fs::read(&exe).expect("old iris"), b"old");

    let error = writable(&staging, &exe).expect_err("readiness must preserve recovery");
    assert!(
        error.contains("recovery files from a previous update remain"),
        "{error}"
    );
    assert_eq!(
        std::fs::read(&backup).expect("backup after readiness"),
        b"old-ffmpeg"
    );
    let error = swap_portable(&zip, &exe).expect_err("retry must preserve recovery");
    assert!(
        error.contains("recovery files from a previous update remain"),
        "{error}"
    );
    tidy_at(&exe);
    assert_eq!(
        std::fs::read(&backup).expect("backup after tidy"),
        b"old-ffmpeg"
    );

    // Once the caller restores the backup, ordinary startup cleanup
    // removes the empty staging tree again.
    std::fs::remove_file(&ffmpeg).expect("remove partially replaced FFmpeg");
    std::fs::rename(&backup, &ffmpeg).expect("manual recovery");
    tidy_at(&exe);
    assert!(
        !staging.exists(),
        "completed recovery must not block cleanup"
    );
    assert_eq!(
        std::fs::read(&ffmpeg).expect("manually restored FFmpeg"),
        b"old-ffmpeg"
    );
}

#[test]
fn a_failed_executable_restore_preserves_the_aside_until_recovery() {
    let (dir, exe) = install(b"old");
    let zip = zip_of(dir.path(), "new.zip", ZIP_DIR, b"new");
    let staging = staging(&exe).expect("staging");
    let aside = aside(&exe);

    // Every packaged file moves in. Moving the running executable
    // aside succeeds, but both moving the new one in and restoring the
    // old one fail. No payload backup remains, so the aside itself must
    // keep staging, readiness, and startup cleanup in recovery mode.
    let mut faults = vec![false; RUNTIME_PAYLOAD.len() + 1];
    faults.extend([true, true]);
    recovery::inject_renames(&faults);
    let error = swap_portable(&zip, &exe).expect_err("injected executable restore failure");
    assert!(error.contains("recovery file remains at"), "{error}");
    assert!(error.contains("recovery files remain"), "{error}");
    assert!(
        !exe.exists(),
        "the failed new move must not create iris.exe"
    );
    assert_eq!(std::fs::read(&aside).expect("old executable aside"), b"old");

    let error = writable(&staging, &exe).expect_err("readiness must preserve the aside");
    assert!(
        error.contains("recovery files from a previous update remain"),
        "{error}"
    );
    tidy_at(&exe);
    assert_eq!(std::fs::read(&aside).expect("aside after tidy"), b"old");
    assert!(staging.exists());

    std::fs::rename(&aside, &exe).expect("manual executable recovery");
    tidy_at(&exe);
    assert_eq!(std::fs::read(&exe).expect("restored executable"), b"old");
    assert!(!staging.exists());
}

#[test]
fn a_swap_rejects_every_missing_runtime_file() {
    for required in RUNTIME_PAYLOAD {
        let (dir, exe) = install(b"old");
        let zip = zip_without(dir.path(), "new.zip", required);
        let error = swap_portable(&zip, &exe).expect_err(required);
        assert!(
            error.contains(&format!("holds no iris\\{required}")),
            "{required}: {error}"
        );
        assert_eq!(std::fs::read(&exe).expect("read"), b"old");
        assert_eq!(names(exe.parent().unwrap()), ["iris.exe"]);
    }
}

/// The `iris --update` that swaps a portable iris runs the iris.exe it
/// replaces. Windows writes no running program's file but renames it:
/// the swap succeeds while PING.EXE, copied to iris.exe, runs.
#[test]
fn a_portable_swap_replaces_a_running_iris() {
    let (dir, exe) = install(b"");
    let ping =
        Path::new(&std::env::var_os("SystemRoot").expect("SystemRoot")).join(r"System32\PING.EXE");
    std::fs::copy(&ping, &exe).expect("copy PING.EXE");
    let mut running = Command::new(&exe)
        .args(["-n", "30", "127.0.0.1"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("start iris.exe");
    let zip = zip_of(dir.path(), "new.zip", ZIP_DIR, b"new");
    let swapped = swap_portable(&zip, &exe);
    running.kill().expect("kill");
    running.wait().expect("wait");
    swapped.expect("swap over a running iris.exe");
    assert_eq!(std::fs::read(&exe).expect("read"), b"new");
    assert_eq!(
        names(exe.parent().unwrap()),
        ["ffmpeg.exe", "iris.exe", "iris.exe.old", "licenses"]
    );
}

/// Each zip fails the swap, and the directory holds the old iris.exe
/// and nothing else.
#[test]
fn a_swap_that_fails_leaves_the_iris_alone() {
    let (dir, exe) = install(b"old");
    let wrong_dir = zip_of(dir.path(), "wrong.zip", "other", b"new");
    let incomplete = zip_of_parts(dir.path(), "incomplete.zip", ZIP_DIR, b"new", false);
    let corrupt = dir.path().join("corrupt.zip");
    std::fs::write(&corrupt, b"PK\x03\x04 not a zip").expect("write");
    for (zip, want) in [
        (wrong_dir, "holds no iris\\iris.exe".to_string()),
        (incomplete, "holds no iris\\ffmpeg.exe".to_string()),
        (
            corrupt.clone(),
            format!("update: unpack {}", corrupt.display()),
        ),
        (dir.path().join("missing.zip"), "update: unpack".to_string()),
    ] {
        let e = swap_portable(&zip, &exe).expect_err(&zip.display().to_string());
        assert!(e.contains(&want), "{}: {e}", zip.display());
        assert_eq!(std::fs::read(&exe).expect("read"), b"old");
        assert_eq!(names(exe.parent().unwrap()), ["iris.exe"]);
    }
}

#[test]
fn a_move_that_fails_puts_the_iris_back() {
    let (dir, exe) = install(b"old");
    let e = replace(&dir.path().join("gone.exe"), &exe).expect_err("no new file");
    assert!(e.starts_with("update: replace "), "{e}");
    assert_eq!(std::fs::read(&exe).expect("read"), b"old");
    assert_eq!(names(exe.parent().unwrap()), ["iris.exe"]);
}

#[test]
fn a_portable_iris_is_ready_only_where_it_can_write() {
    let (dir, exe) = install(b"old");
    writable(&staging(&exe).expect("staging"), &exe).expect("a writable folder");
    assert_eq!(names(exe.parent().unwrap()), ["iris.exe"]);
    let gone = dir.path().join("gone").join("iris.exe");
    let e = writable(&staging(&gone).expect("staging"), &gone).expect_err("no folder");
    assert!(e.starts_with("update: cannot write "), "{e}");
    assert!(e.ends_with("or install it with the setup"), "{e}");
}

#[test]
fn tidy_deletes_the_iris_a_portable_update_moved_aside() {
    let old = aside(&crate::sys::exe::this().expect("current_exe"));
    std::fs::write(&old, b"old").expect("write");
    tidy();
    assert!(!old.exists(), "{} stayed", old.display());
}
