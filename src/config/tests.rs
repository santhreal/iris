use super::*;

/// Root every iris location in a fresh tempdir via `IRIS_HOME`, on
/// every platform; returns it so the test can seed files.
fn isolated_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(crate::dirs::HOME_ENV, dir.path());
    dir
}

/// XDG layout with `IRIS_HOME` cleared, for the legacy-path
/// migration that only exists on Linux.
#[cfg(target_os = "linux")]
fn xdg_without_iris_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::remove_var(crate::dirs::HOME_ENV);
    std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
    dir
}

#[test]
#[serial_test::serial]
fn toml_round_trip_preserves_every_field() {
    let _d = isolated_home();
    let cfg = Config::default();
    let text = toml::to_string_pretty(&cfg).unwrap();
    let back: Config = toml::from_str(&text).unwrap();
    assert_eq!(back.screenshots_dir, cfg.screenshots_dir);
    assert_eq!(back.recordings_dir, cfg.recordings_dir);
    assert_eq!(back.screenshot_template, cfg.screenshot_template);
    assert_eq!(back.recording_fps, cfg.recording_fps);
    assert_eq!(back.capture_hotkey, cfg.capture_hotkey);
    assert_eq!(back.record_hotkey, cfg.record_hotkey);
    assert_eq!(back.toast_click_action, cfg.toast_click_action);
    assert_eq!(back.toast_position, cfg.toast_position);
    assert_eq!(back.toast_pin_enabled, cfg.toast_pin_enabled);
    assert_eq!(back.toast_duration_ms, cfg.toast_duration_ms);
    assert_eq!(back.cancel_keybind, cfg.cancel_keybind);
    assert_eq!(back.confirm_keybind, cfg.confirm_keybind);
}

#[test]
#[serial_test::serial]
fn old_config_without_new_fields_loads_defaults() {
    // A config.toml written before the toast/keybind fields existed
    // must still load; serde(default) fills them.
    let d = isolated_home();
    let path = Config::path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "recording_fps = 24\n").unwrap();
    let cfg = Config::load();
    assert_eq!(cfg.recording_fps, 24);
    assert_eq!(cfg.toast_click_action, ToastClickAction::Markup);
    assert_eq!(cfg.toast_position, ToastPosition::BottomRight);
    assert!(cfg.toast_pin_enabled);
    assert_eq!(cfg.toast_duration_ms, 5000);
    assert_eq!(cfg.cancel_keybind, "Escape");
    drop(d);
}

#[test]
#[serial_test::serial]
fn enum_fields_parse_kebab_case() {
    let cfg: Config =
        toml::from_str("toast_click_action = \"copy\"\ntoast_position = \"top-left\"\n").unwrap();
    assert_eq!(cfg.toast_click_action, ToastClickAction::Copy);
    assert_eq!(cfg.toast_position, ToastPosition::TopLeft);
}

#[test]
#[serial_test::serial]
fn tilde_dirs_expand_to_home() {
    let _d = isolated_home();
    let home = directories::UserDirs::new()
        .unwrap()
        .home_dir()
        .to_path_buf();
    let mut cfg = Config {
        screenshots_dir: PathBuf::from("~/shots"),
        recordings_dir: PathBuf::from("~/recs"),
        ..Config::default()
    };
    cfg.expand_dirs();
    assert_eq!(cfg.screenshots_dir, home.join("shots"));
    assert_eq!(cfg.recordings_dir, home.join("recs"));
}

/// WHY: the class closed here is "a directory in `~` form reaches a
/// reader unexpanded". `store` cached the config as given, so after
/// the settings window saved "~/shots" every `load` handed that to
/// the capture pipeline, which wrote into a directory named "~"
/// under the daemon's cwd. The file keeps the `~` form; readers get
/// the expanded path. Not covered: a relative directory, which the
/// settings window rejects before it gets here.
#[test]
#[serial_test::serial]
fn a_stored_tilde_dir_reaches_readers_expanded_and_stays_tilde_on_disk() {
    let _d = isolated_home();
    let home = home_dir().unwrap();
    Config {
        screenshots_dir: PathBuf::from("~/shots"),
        recordings_dir: home.join("recs"),
        ..Config::default()
    }
    .store()
    .unwrap();
    let live = Config::load();
    assert_eq!(live.screenshots_dir, home.join("shots"));
    assert_eq!(live.recordings_dir, home.join("recs"));
    let text = std::fs::read_to_string(Config::path().unwrap()).unwrap();
    let file: Config = toml::from_str(&text).unwrap();
    assert_eq!(file.screenshots_dir, Path::new("~").join("shots"));
    assert_eq!(file.recordings_dir, Path::new("~").join("recs"));
}

#[test]
fn home_paths_round_trip_through_the_tilde_form() {
    let home = home_dir().unwrap();
    let inside = home.join("Pictures").join("iris");
    let short = Path::new("~").join("Pictures").join("iris");
    assert_eq!(contract_home(&inside), short);
    assert_eq!(expand_home(&short), inside);
    assert_eq!(contract_home(&home), PathBuf::from("~"));
    // A sibling whose name extends the home directory's is outside it.
    let mut sibling = home.clone().into_os_string();
    sibling.push("-other");
    let sibling = PathBuf::from(sibling).join("x");
    assert_eq!(contract_home(&sibling), sibling);
    for plain in [PathBuf::from("shots"), Path::new("~user").join("x")] {
        assert_eq!(expand_home(&plain), plain);
        assert_eq!(contract_home(&plain), plain);
    }
}

#[test]
#[serial_test::serial]
fn load_writes_defaults_when_missing() {
    let _d = isolated_home();
    let cfg = Config::load();
    assert!(Config::path().unwrap().exists());
    assert_eq!(cfg.recording_fps, 30);
    // The defaults are written in `~` form, as `store` writes them.
    let text = std::fs::read_to_string(Config::path().unwrap()).unwrap();
    let file: Config = toml::from_str(&text).unwrap();
    assert_eq!(file.screenshots_dir, contract_home(&cfg.screenshots_dir));
    assert_eq!(file.recordings_dir, contract_home(&cfg.recordings_dir));
}

#[test]
#[serial_test::serial]
fn load_reads_stored_values() {
    let d = isolated_home();
    let path = Config::path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "recording_fps = 24\ncapture_hotkey = \"F9\"\n").unwrap();
    let cfg = Config::load();
    assert_eq!(cfg.recording_fps, 24);
    assert_eq!(cfg.capture_hotkey, "F9");
    drop(d);
}

#[cfg(target_os = "linux")]
#[test]
#[serial_test::serial]
fn glint_config_migrates_to_iris_path() {
    let d = xdg_without_iris_home();
    let old = d.path().join("config/glint/config.toml");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, "recording_fps = 12\n").unwrap();
    let cfg = Config::load();
    assert_eq!(cfg.recording_fps, 12);
    // The copy landed at the iris path, not just in memory.
    assert!(Config::path().unwrap().exists());
}

#[cfg(target_os = "linux")]
#[test]
#[serial_test::serial]
fn existing_iris_config_wins_over_glint() {
    let d = xdg_without_iris_home();
    let old = d.path().join("config/glint/config.toml");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, "recording_fps = 12\n").unwrap();
    let path = Config::path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "recording_fps = 60\n").unwrap();
    assert_eq!(Config::load().recording_fps, 60);
}

#[test]
#[serial_test::serial]
fn invalid_toml_falls_back_to_defaults() {
    let _d = isolated_home();
    let path = Config::path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "this is not toml = = =\n").unwrap();
    let cfg = Config::load();
    assert_eq!(cfg.recording_fps, 30);
    // The file is left for its owner to fix, not replaced by defaults.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "this is not toml = = =\n"
    );
}
