use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

/// A zip at `dir\name` holding `entry` (`top\iris.exe`) with `body`,
/// made with the tar.exe the updater unpacks it with.
fn zip_of(dir: &Path, name: &str, top: &str, body: &[u8]) -> PathBuf {
    let src = dir.join("src");
    std::fs::create_dir_all(src.join(top)).expect("mkdir");
    std::fs::write(src.join(top).join("iris.exe"), body).expect("write");
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
    // The test binary has no uninstall.exe beside it: a portable iris.
    assert_eq!(asset(), PORTABLE_ASSET);
}

#[test]
fn a_portable_swap_puts_the_new_iris_in_place() {
    let (dir, exe) = install(b"old");
    let zip = zip_of(dir.path(), "new.zip", ZIP_DIR, b"new");
    swap_portable(&zip, &exe).expect("swap");
    assert_eq!(std::fs::read(&exe).expect("read"), b"new");
    assert_eq!(std::fs::read(aside(&exe)).expect("read"), b"old");
    assert_eq!(names(exe.parent().unwrap()), ["iris.exe", "iris.exe.old"]);
    // A second update replaces the iris.exe.old the first one left.
    let zip = zip_of(dir.path(), "newer.zip", ZIP_DIR, b"newer");
    swap_portable(&zip, &exe).expect("second swap");
    assert_eq!(std::fs::read(&exe).expect("read"), b"newer");
    assert_eq!(std::fs::read(aside(&exe)).expect("read"), b"new");
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
    assert_eq!(names(exe.parent().unwrap()), ["iris.exe", "iris.exe.old"]);
}

/// Each zip fails the swap, and the directory holds the old iris.exe
/// and nothing else.
#[test]
fn a_swap_that_fails_leaves_the_iris_alone() {
    let (dir, exe) = install(b"old");
    let wrong_dir = zip_of(dir.path(), "wrong.zip", "other", b"new");
    let corrupt = dir.path().join("corrupt.zip");
    std::fs::write(&corrupt, b"PK\x03\x04 not a zip").expect("write");
    for (zip, want) in [
        (wrong_dir, "holds no iris\\iris.exe".to_string()),
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
    writable(&staging(&exe).expect("staging")).expect("a writable folder");
    assert_eq!(names(exe.parent().unwrap()), ["iris.exe"]);
    let gone = dir.path().join("gone").join("iris.exe");
    let e = writable(&staging(&gone).expect("staging")).expect_err("no folder");
    assert!(e.starts_with("update: cannot write "), "{e}");
    assert!(e.ends_with("or install it with the setup"), "{e}");
}

#[test]
fn tidy_deletes_the_iris_a_portable_update_moved_aside() {
    let old = aside(&std::env::current_exe().expect("current_exe"));
    std::fs::write(&old, b"old").expect("write");
    tidy();
    assert!(!old.exists(), "{} stayed", old.display());
}
