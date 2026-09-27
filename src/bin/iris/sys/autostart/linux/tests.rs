use std::path::Path;

use super::*;

/// The entry the deb and the rpm install.
const PACKAGE_ENTRY: &str =
    include_str!("../../../../../../packaging/linux/iris-autostart.desktop");

/// An account with no autostart entry, and a system directory that
/// holds the package's entry when `package` is set.
fn account(package: bool) -> (tempfile::TempDir, Places) {
    let root = tempfile::tempdir().expect("tempdir");
    let system = root.path().join("etc/xdg/autostart");
    if package {
        std::fs::create_dir_all(&system).expect("mkdir");
        std::fs::write(system.join(ENTRY), PACKAGE_ENTRY).expect("write");
    }
    let places = Places {
        user: root.path().join("home/.config/autostart"),
        system: vec![root.path().join("missing/autostart"), system],
    };
    (root, places)
}

fn iris(program: &str, packaged: bool) -> Iris {
    Iris {
        exec: format!(
            "{} --daemon",
            exec_arg(Path::new(program)).expect("exec_arg")
        ),
        packaged,
    }
}

/// Every name in `dir`, sorted; none when it does not exist.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|e| e.expect("entry").file_name().into_string().expect("utf-8"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn user_entry(places: &Places) -> String {
    std::fs::read_to_string(places.user.join(ENTRY)).expect("user entry")
}

#[test]
fn an_appimage_starts_at_login_only_once_turned_on() {
    let (_root, places) = account(false);
    let image = iris("/home/a/Apps/iris.AppImage", false);
    assert!(!enabled_in(&places, &image));
    set_in(&places, &image, true).expect("on");
    assert!(enabled_in(&places, &image));
    assert_eq!(
        user_entry(&places),
        entry(r#""/home/a/Apps/iris.AppImage" --daemon"#)
    );
    assert_eq!(names(&places.user), [ENTRY]);
    // Another AppImage, or the same one moved, is not the one it starts.
    assert!(!enabled_in(&places, &iris("/opt/iris.AppImage", false)));
    set_in(&places, &image, false).expect("off");
    assert!(!enabled_in(&places, &image));
    assert_eq!(names(&places.user), Vec::<String>::new());
    set_in(&places, &image, false).expect("off again");
}

#[test]
fn a_package_starts_at_login_until_turned_off() {
    let (_root, places) = account(true);
    let packaged = iris("/usr/bin/iris", true);
    assert!(enabled_in(&places, &packaged));
    // The package's entry starts the packaged iris, not an AppImage.
    assert!(!enabled_in(&places, &iris("/home/a/iris.AppImage", false)));
    set_in(&places, &packaged, false).expect("off");
    assert!(!enabled_in(&places, &packaged));
    assert_eq!(user_entry(&places), HIDDEN);
    set_in(&places, &packaged, true).expect("on");
    assert!(enabled_in(&places, &packaged));
    assert_eq!(user_entry(&places), entry(r#""/usr/bin/iris" --daemon"#));
    assert_eq!(names(&places.user), [ENTRY]);
}

#[test]
fn an_entry_that_starts_nothing_reads_as_off() {
    let (_root, places) = account(false);
    let image = iris("/a/iris.AppImage", false);
    let exec = format!("Exec={}", image.exec);
    for (text, on) in [
        (format!("[Desktop Entry]\n{exec}\n"), true),
        (format!("[Desktop Entry]\n{exec}\nHidden=true\n"), false),
        (format!("[Desktop Entry]\n{exec}\nHidden = true\n"), false),
        (format!("[Desktop Entry]\n{exec}\nHidden=false\n"), true),
        (
            format!("[Desktop Entry]\n{exec}\nX-GNOME-Autostart-enabled=false\n"),
            false,
        ),
        (
            format!("[Desktop Entry]\n{exec}\n[Desktop Action x]\nHidden=true\n"),
            true,
        ),
        (format!("[Desktop Action x]\n{exec}\n"), false),
        ("[Desktop Entry]\nExec=iris --daemon\n".to_string(), false),
    ] {
        std::fs::create_dir_all(&places.user).expect("mkdir");
        std::fs::write(places.user.join(ENTRY), &text).expect("write");
        assert_eq!(enabled_in(&places, &image), on, "{text}");
    }
}

#[test]
fn exec_quotes_the_program_as_one_argument() {
    for (program, arg) in [
        ("/usr/bin/iris", r#""/usr/bin/iris""#),
        (
            "/home/a/My Apps/iris.AppImage",
            r#""/home/a/My Apps/iris.AppImage""#,
        ),
        (r#"/a"b$c`d"#, r#""/a\\"b\\$c\\`d""#),
        (r"/a\b", r#""/a\\\\b""#),
        ("/100%/iris", r#""/100%%/iris""#),
    ] {
        assert_eq!(
            exec_arg(Path::new(program)).as_deref(),
            Ok(arg),
            "{program}"
        );
    }
    let e = exec_arg(Path::new("/a\nb")).expect_err("newline");
    assert!(
        e.ends_with("holds a control character, which an Exec line cannot"),
        "{e}"
    );
    use std::os::unix::ffi::OsStrExt;
    let e = exec_arg(Path::new(std::ffi::OsStr::from_bytes(b"/a\xffb"))).expect_err("not UTF-8");
    assert!(
        e.ends_with("is not UTF-8, which a desktop entry holds"),
        "{e}"
    );
}

#[test]
fn a_write_that_fails_leaves_nothing_behind() {
    let (root, mut places) = account(false);
    let blocker = root.path().join("file");
    std::fs::write(&blocker, b"").expect("write");
    places.user = blocker.join("autostart");
    let e = set_in(&places, &iris("/usr/bin/iris", true), true).expect_err("not a directory");
    assert!(e.starts_with("start at login: write "), "{e}");
    assert_eq!(names(root.path()), ["file"]);
}

/// The rename fails after the staged copy is written: the entry's name
/// is a directory that holds a file.
#[test]
fn a_rename_that_fails_leaves_no_staged_copy() {
    let (_root, places) = account(false);
    std::fs::create_dir_all(places.user.join(ENTRY).join("x")).expect("mkdir");
    let e = set_in(&places, &iris("/usr/bin/iris", true), true).expect_err("a directory");
    assert!(e.starts_with("start at login: write "), "{e}");
    assert_eq!(names(&places.user), [ENTRY]);
    let e = set_in(&places, &iris("/usr/bin/iris", true), false).expect_err("a directory");
    assert!(e.starts_with("start at login: delete "), "{e}");
}

/// A package entry turned off in /etc/xdg starts nothing, and the
/// first system directory that holds the entry is the one that counts.
#[test]
fn a_package_entry_that_starts_nothing_reads_as_off() {
    let packaged = iris("/usr/bin/iris", true);
    for (text, on) in [
        (PACKAGE_ENTRY.to_string(), true),
        (format!("{PACKAGE_ENTRY}Hidden=true\n"), false),
        (
            PACKAGE_ENTRY.replace(
                "X-GNOME-Autostart-enabled=true",
                "X-GNOME-Autostart-enabled=false",
            ),
            false,
        ),
    ] {
        let (_root, mut places) = account(true);
        let first = places.user.with_file_name("first");
        std::fs::create_dir_all(&first).expect("mkdir");
        std::fs::write(first.join(ENTRY), &text).expect("write");
        places.system.insert(0, first);
        assert_eq!(enabled_in(&places, &packaged), on, "{text}");
    }
}
