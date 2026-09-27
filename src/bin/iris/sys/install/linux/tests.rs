use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::*;

/// The test process runs from the target directory, from no AppImage
/// and no package, whatever `$APPIMAGE` and `$APPDIR` it inherited.
#[test]
fn an_install_from_no_appimage_and_no_package_refuses_to_update() {
    let exe = crate::sys::exe::this().expect("this iris");
    assert_eq!(
        ready(),
        Err(format!(
            "update: {} is not from an AppImage, deb or rpm; install the release by hand",
            exe.display()
        ))
    );
    assert_eq!(asset(), APPIMAGE_ASSET);
}

/// Each package downloads the release asset of its own kind: the deb's
/// package manager installs no rpm, and neither an AppImage.
#[test]
fn a_package_downloads_the_asset_its_package_manager_installs() {
    assert_eq!(asset_of(Package::Deb), "linux-x86_64.deb");
    assert_eq!(asset_of(Package::Rpm), "linux-x86_64.rpm");
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

#[test]
fn a_writable_appimage_directory_is_ready_and_left_as_it_was() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("iris.AppImage");
    std::fs::write(&target, b"old").expect("write");
    writable(&staged(&target)).expect("writable");
    assert_eq!(names(dir.path()), ["iris.AppImage"]);
    assert_eq!(std::fs::read(&target).expect("read"), b"old");
}

#[test]
fn an_appimage_directory_that_takes_no_file_is_not_ready() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("gone/iris.AppImage");
    let staged = staged(&target);
    let e = writable(&staged).expect_err("no directory");
    let want = format!(
        "update: cannot write {}: No such file or directory (os error 2); move the AppImage \
         to a folder this account can write, or install the deb or rpm",
        staged.display()
    );
    assert_eq!(e, want);
}

/// `ready` checks the file `replace` writes: a check of another name
/// passes where the swap then fails.
#[test]
fn the_ready_check_writes_where_the_swap_stages() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("iris.AppImage");
    std::fs::write(&target, b"old").expect("write");
    // A directory at the staged path: create() of a file there fails,
    // and so does the swap's copy.
    std::fs::create_dir(staged(&target)).expect("mkdir");
    std::fs::write(staged(&target).join("kept"), b"x").expect("write");
    writable(&staged(&target)).expect_err("the staged path is taken");
    let download = dir.path().join("download");
    std::fs::write(&download, b"new").expect("write");
    replace(&download, &target).expect_err("the staged path is taken");
    assert_eq!(std::fs::read(&target).expect("read"), b"old");
}

#[test]
fn a_replace_puts_an_executable_copy_at_the_target() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (new, target) = (
        dir.path().join("download"),
        dir.path().join("iris.AppImage"),
    );
    std::fs::write(&new, b"new").expect("write");
    std::fs::write(&target, b"old").expect("write");
    replace(&new, &target).expect("replace");
    assert_eq!(std::fs::read(&target).expect("read"), b"new");
    let mode = std::fs::metadata(&target)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    assert_eq!(names(dir.path()), ["download", "iris.AppImage"]);
}

#[test]
fn a_download_that_cannot_be_read_leaves_the_target_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("iris.AppImage");
    std::fs::write(&target, b"old").expect("write");
    let e = replace(&dir.path().join("missing"), &target).expect_err("no download");
    assert!(e.starts_with("update: stage "), "{e}");
    assert_eq!(std::fs::read(&target).expect("read"), b"old");
    assert_eq!(names(dir.path()), ["iris.AppImage"]);
}

#[test]
fn a_rename_that_fails_deletes_the_staged_copy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let new = dir.path().join("download");
    std::fs::write(&new, b"new").expect("write");
    // A non-empty directory: rename() of a file over it fails.
    let target = dir.path().join("iris.AppImage");
    std::fs::create_dir(&target).expect("mkdir");
    std::fs::write(target.join("kept"), b"old").expect("write");
    let e = replace(&new, &target).expect_err("rename over a directory");
    assert!(e.starts_with("update: replace "), "{e}");
    assert_eq!(std::fs::read(target.join("kept")).expect("read"), b"old");
    assert_eq!(names(dir.path()), ["download", "iris.AppImage"]);
}
