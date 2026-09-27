//! packaging/linux/AppRun starts iris with the environment it was given.
//!
//! WHY: the class closed here is "AppRun changes the environment of iris
//! and of every program iris starts". iris bundles no libraries,
//! programs or data. AppRun prepended the AppDir, a mount that is gone
//! once iris quits, to LD_LIBRARY_PATH, PATH and XDG_DATA_DIRS, and with
//! LD_LIBRARY_PATH unset it left a trailing colon: an empty entry, which
//! the dynamic loader reads as the working directory. A library in the
//! folder the AppImage was started from, such as ~/Downloads, then ran
//! inside iris and inside each program iris started. AppRun also set
//! $APPDIR to its own AppDir, which paired an inherited $APPIMAGE of
//! another AppImage with iris's mount, so iris took that AppImage for
//! its own. Each case runs the AppRun file the AppImage ships, with a
//! copy of `env` as iris, and requires the environment `env` prints to
//! be the caller's, variable for variable: any variable AppRun adds,
//! drops or changes fails it. Not covered: the AppImage runtime, which
//! sets $APPIMAGE and $APPDIR before AppRun runs.

#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

/// Variables the shell that runs AppRun sets itself.
const SHELL_OWNED: [&str; 3] = ["PWD", "SHLVL", "_"];

/// The environment iris starts with when AppRun runs with `caller` in
/// the directory it was started from.
fn environment_of_iris(caller: &BTreeMap<&str, String>) -> BTreeMap<String, String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let appdir = dir.path().join("squashfs-root");
    std::fs::create_dir_all(appdir.join("usr/bin")).expect("mkdir");
    let apprun = appdir.join("AppRun");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/packaging/linux/AppRun"),
        &apprun,
    )
    .expect("copy AppRun");
    let iris = appdir.join("usr/bin/iris");
    std::fs::copy("/usr/bin/env", &iris).expect("copy env");
    for file in [&apprun, &iris] {
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let downloads = dir.path().join("Downloads");
    std::fs::create_dir(&downloads).expect("mkdir");
    let out = Command::new(&apprun)
        .arg("-0")
        .env_clear()
        .envs(caller)
        .current_dir(&downloads)
        .output()
        .expect("run AppRun");
    assert!(out.status.success(), "AppRun: {out:?}");
    out.stdout
        .split(|&b| b == 0)
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let kv = std::str::from_utf8(kv).expect("utf-8");
            let (k, v) = kv.split_once('=').expect("NAME=value");
            (k.to_string(), v.to_string())
        })
        .filter(|(k, _)| !SHELL_OWNED.contains(&k.as_str()))
        .collect()
}

#[test]
fn iris_starts_with_the_callers_environment() {
    let base = || {
        BTreeMap::from([
            ("HOME", "/home/u".to_string()),
            ("PATH", "/usr/local/bin:/usr/bin:/bin".to_string()),
        ])
    };
    let with = |pairs: &[(&'static str, &str)]| {
        let mut env = base();
        env.extend(pairs.iter().map(|&(k, v)| (k, v.to_string())));
        env
    };
    let cases = [
        ("no library or data path", base()),
        (
            "a library and a data path",
            with(&[
                ("LD_LIBRARY_PATH", "/opt/gl/lib"),
                ("XDG_DATA_DIRS", "/usr/local/share:/usr/share"),
            ]),
        ),
        (
            "an empty library path",
            with(&[("LD_LIBRARY_PATH", ""), ("XDG_DATA_DIRS", "")]),
        ),
        (
            "iris's AppImage",
            with(&[
                ("APPIMAGE", "/home/u/Apps/iris.AppImage"),
                ("APPDIR", "/tmp/.mount_irisAb12"),
            ]),
        ),
        (
            "another AppImage's pair, inherited",
            with(&[
                ("APPIMAGE", "/home/u/Apps/kitty.AppImage"),
                ("APPDIR", "/tmp/.mount_kittyCd34"),
            ]),
        ),
    ];
    for (case, caller) in cases {
        let want: BTreeMap<String, String> = caller
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        assert_eq!(environment_of_iris(&caller), want, "{case}");
    }
}
