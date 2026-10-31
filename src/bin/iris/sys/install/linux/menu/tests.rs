use super::*;

/// An AppImage's mount, with its binary and its icon, and an account's
/// data directory, in one tempdir.
struct Rig {
    _root: tempfile::TempDir,
    places: Places,
    exe: PathBuf,
}

impl Rig {
    fn new(icon: Option<&[u8]>) -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let usr = root.path().join("mount/usr");
        let exe = usr.join("bin/iris");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"").unwrap();
        if let Some(icon) = icon {
            let at = usr.join(ICON);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, icon).unwrap();
        }
        let data = root.path().join("data");
        Self {
            places: Places {
                applications: data.join("applications"),
                icon: data.join("icons/hicolor/256x256/apps/dev.iris.app.png"),
            },
            exe,
            _root: root,
        }
    }

    fn entry(&self) -> String {
        std::fs::read_to_string(self.places.applications.join(NAME)).expect("entry")
    }
}

/// The `Exec` lines of `text`, in order.
fn execs(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("Exec=")).collect()
}

#[test]
fn every_exec_of_the_package_entry_runs_the_appimage() {
    let rig = Rig::new(Some(b"png"));
    let image = Path::new("/home/a/My Apps/iris.AppImage");
    assert_eq!(rig_first(&rig, image), Ok(true));
    let text = rig.entry();
    let package = execs(PACKAGE_ENTRY);
    let want: Vec<String> = package
        .iter()
        .map(|l| {
            let args = l.strip_prefix("Exec=iris").expect("the package runs iris");
            format!("Exec=\"/home/a/My Apps/iris.AppImage\"{args}")
        })
        .collect();
    assert_eq!(execs(&text), want);
    assert!(!package.is_empty());
    // The application launch is the bare app; its actions add an option.
    assert_eq!(package[0], "Exec=iris");
    let icon = rig.places.icon.to_str().unwrap();
    assert!(text.contains(&format!("\nIcon={icon}\n")), "{text}");
    assert!(!text.contains("\nIcon=iris\n"), "{text}");
    assert!(
        text.starts_with(
            "[Desktop Entry]\nType=Application\nTryExec=/home/a/My Apps/iris.AppImage\n"
        ),
        "{text}"
    );
    assert_eq!(std::fs::read(&rig.places.icon).unwrap(), b"png");
    // Every other line is the package's.
    let rest = |t: &str| -> Vec<String> {
        t.lines()
            .filter(|l| {
                !l.starts_with("Exec=") && !l.starts_with("Icon=") && !l.starts_with("TryExec=")
            })
            .map(str::to_string)
            .collect()
    };
    assert_eq!(rest(&text), rest(PACKAGE_ENTRY));
}

fn rig_first(rig: &Rig, image: &Path) -> Result<bool, String> {
    integrate_in(&rig.places, image, &rig.exe)
}

#[test]
fn a_later_start_is_not_a_first_and_a_moved_appimage_moves_the_entry() {
    let rig = Rig::new(Some(b"png"));
    assert_eq!(rig_first(&rig, Path::new("/a/iris.AppImage")), Ok(true));
    let first = rig.entry();
    assert_eq!(rig_first(&rig, Path::new("/a/iris.AppImage")), Ok(false));
    assert_eq!(rig.entry(), first);
    assert_eq!(rig_first(&rig, Path::new("/b/iris.AppImage")), Ok(false));
    let moved = rig.entry();
    assert!(moved.contains("\nTryExec=/b/iris.AppImage\n"), "{moved}");
    assert!(!moved.contains("/a/iris.AppImage"), "{moved}");
    // No staged copy is left beside the entry or the icon.
    for dir in [&rig.places.applications, rig.places.icon.parent().unwrap()] {
        let names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }
}

#[test]
fn an_appimage_with_no_icon_writes_no_entry_and_stays_a_first_start() {
    let rig = Rig::new(None);
    let e = rig_first(&rig, Path::new("/a/iris.AppImage")).expect_err("no icon");
    assert!(e.starts_with("read "), "{e}");
    assert!(!rig.places.applications.join(NAME).exists());
    let icon = rig.exe.parent().unwrap().parent().unwrap().join(ICON);
    std::fs::create_dir_all(icon.parent().unwrap()).unwrap();
    std::fs::write(icon, b"png").unwrap();
    assert_eq!(rig_first(&rig, Path::new("/a/iris.AppImage")), Ok(true));
}

#[test]
fn a_path_an_entry_cannot_hold_writes_nothing() {
    let rig = Rig::new(Some(b"png"));
    let e = rig_first(&rig, Path::new("/a\nb/iris.AppImage")).expect_err("newline");
    assert!(e.ends_with("which a desktop entry cannot"), "{e}");
    assert!(!rig.places.applications.exists());
    assert!(!rig.places.icon.exists());
}
