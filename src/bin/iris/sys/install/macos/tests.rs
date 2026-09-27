// WHY: the class closed here is "a macOS update that fails part way
// leaves no iris.app": the updater deleted /Applications/iris.app and
// then copied the new bundle over, so a failed copy left no app, and it
// replaced /Applications/iris.app whichever bundle ran. Not covered:
// the relaunch that follows a swap, and a target on a volume without
// RENAME_SWAP.

use super::*;

#[test]
fn an_executable_inside_a_bundle_names_the_bundle() {
    for (exe, bundle) in [
        (
            "/Applications/iris.app/Contents/MacOS/iris",
            Some("/Applications/iris.app"),
        ),
        (
            "/Users/a/Applications/iris.app/Contents/MacOS/iris",
            Some("/Users/a/Applications/iris.app"),
        ),
        ("/usr/local/bin/iris", None),
        ("/Applications/iris.app/Contents/Resources/iris", None),
        ("/Applications/iris/Contents/MacOS/iris", None),
        ("/Applications/iris.app/MacOS/iris", None),
        ("iris", None),
    ] {
        assert_eq!(
            bundle_of(Path::new(exe)).as_deref(),
            bundle.map(Path::new),
            "{exe}"
        );
    }
}

#[test]
fn a_binary_outside_a_bundle_refuses_to_update() {
    let err = ready().expect_err("cargo runs a test binary outside any bundle");
    assert!(
        err.starts_with("update: /")
            && err.ends_with(" is not inside an iris.app bundle; install the release DMG by hand"),
        "{err}"
    );
}

/// A bundle at `dir/iris.app` whose executable holds `marker`, with the
/// resource file `resource`.
fn bundle_at(dir: &Path, marker: &str, resource: &str) -> PathBuf {
    let app = dir.join("iris.app");
    for sub in ["Contents/MacOS", "Contents/Resources"] {
        std::fs::create_dir_all(app.join(sub)).expect("create bundle");
    }
    std::fs::write(app.join("Contents/MacOS/iris"), marker).expect("write exe");
    std::fs::write(app.join("Contents/Resources").join(resource), marker).expect("write res");
    app
}

/// A compressed DMG of the directory `src`, as make_dmg.sh builds one.
/// `hdiutil create` on a CI runner can fail with "Resource busy" while
/// the system scans the new image; it gets three tries.
fn dmg_of(src: &Path, dir: &Path) -> PathBuf {
    let dmg = dir.join("iris-9.9.9-macos-universal.dmg");
    let create = || {
        Command::new("hdiutil")
            .args([
                "create",
                "-volname",
                "iris",
                "-format",
                "UDZO",
                "-ov",
                "-srcfolder",
            ])
            .arg(src)
            .arg(&dmg)
            .status()
            .expect("run hdiutil create")
    };
    let mut status = create();
    for _ in 1..3 {
        if status.success() {
            break;
        }
        status = create();
    }
    assert!(status.success(), "hdiutil create {status}");
    dmg
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn exe_of(bundle: &Path) -> String {
    std::fs::read_to_string(bundle.join("Contents/MacOS/iris")).unwrap_or_default()
}

#[test]
fn install_swaps_the_new_bundle_in_whole() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src");
    bundle_at(&src, "new", "fresh");
    let apps = tmp.path().join("Applications");
    let target = bundle_at(&apps, "old", "stale");
    let dmg = dmg_of(&src, tmp.path());

    install(&dmg, &target).expect("install");

    assert_eq!(exe_of(&target), "new");
    assert!(target.join("Contents/Resources/fresh").is_file());
    assert!(
        !target.join("Contents/Resources/stale").exists(),
        "the old bundle's files stayed in the new one"
    );
    assert_eq!(names(&apps), ["iris.app"]);
    assert!(!dmg.with_extension("mount").exists());
}

#[test]
fn a_failed_install_leaves_the_installed_bundle_as_it_was() {
    type Source = fn(&Path) -> PathBuf;
    let cases: [(&str, Source); 3] = [
        ("a DMG with no iris.app", |dir| {
            let src = dir.join("src");
            std::fs::create_dir_all(&src).expect("mkdir");
            std::fs::write(src.join("readme.txt"), "iris").expect("write");
            dmg_of(&src, dir)
        }),
        ("an iris.app with no executable", |dir| {
            let src = dir.join("src");
            let app = bundle_at(&src, "new", "fresh");
            std::fs::remove_file(app.join("Contents/MacOS/iris")).expect("rm exe");
            dmg_of(&src, dir)
        }),
        ("a file that is not a disk image", |dir| {
            let dmg = dir.join("iris-9.9.9-macos-universal.dmg");
            std::fs::write(&dmg, "not a disk image").expect("write");
            dmg
        }),
    ];
    for (case, source) in cases {
        let tmp = tempfile::tempdir().expect("tempdir");
        let apps = tmp.path().join("Applications");
        let target = bundle_at(&apps, "old", "stale");
        let dmg = source(tmp.path());

        let err = install(&dmg, &target).expect_err(case);

        assert!(err.starts_with("update: "), "{case}: {err}");
        assert_eq!(exe_of(&target), "old", "{case}");
        assert!(target.join("Contents/Resources/stale").is_file(), "{case}");
        assert_eq!(names(&apps), ["iris.app"], "{case}");
        assert!(!dmg.with_extension("mount").exists(), "{case}");
    }
}
