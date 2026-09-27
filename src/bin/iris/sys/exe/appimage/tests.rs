use super::*;

fn some(path: &str) -> Option<PathBuf> {
    Some(PathBuf::from(path))
}

/// iris runs from the AppImage `$APPIMAGE` names only from inside the
/// mount `$APPDIR` names. A pair another AppImage's program passed on
/// names that AppImage, which an update would overwrite with iris and
/// an autostart entry would start.
#[test]
fn only_an_iris_inside_the_appimage_mount_runs_from_the_appimage() {
    let exe = Path::new("/tmp/.mount_irisAb12/usr/bin/iris");
    let appimage = "/home/u/Apps/iris.AppImage";
    let cases = [
        (
            "run from the mount",
            some("/tmp/.mount_irisAb12"),
            some(appimage),
            some(appimage),
        ),
        (
            "the mount with a trailing slash",
            some("/tmp/.mount_irisAb12/"),
            some(appimage),
            some(appimage),
        ),
        (
            "another AppImage's mount",
            some("/tmp/.mount_kittyCd34"),
            some("/home/u/Apps/kitty.AppImage"),
            None,
        ),
        (
            "a mount whose name the path extends",
            some("/tmp/.mount_iris"),
            some(appimage),
            None,
        ),
        ("no $APPDIR", None, some(appimage), None),
        ("no $APPIMAGE", some("/tmp/.mount_irisAb12"), None, None),
        ("an empty $APPDIR", some(""), some(appimage), None),
        (
            "a relative $APPDIR",
            some("tmp/.mount_irisAb12"),
            some(appimage),
            None,
        ),
        ("the root as $APPDIR", some("/"), some(appimage), None),
    ];
    for (case, appdir, appimage, want) in cases {
        assert_eq!(appimage_of(exe, appimage, appdir), want, "{case}");
    }
}

/// `$APPDIR` through a symlink names the mount iris runs from, whose
/// path the kernel reports resolved.
#[test]
fn a_symlinked_appdir_names_the_mount_it_resolves_to() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mount = dir.path().join("mount");
    std::fs::create_dir(&mount).expect("mkdir");
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&mount, &link).expect("symlink");
    let mount = std::fs::canonicalize(&mount).expect("canonicalize");
    let exe = mount.join("usr/bin/iris");
    assert_eq!(
        appimage_of(&exe, some("/a/iris.AppImage"), Some(link)),
        some("/a/iris.AppImage")
    );
}
