use super::*;

/// Root every iris location in a fresh tempdir via `IRIS_HOME`, on
/// every platform; returns it so the test can seed files.
fn isolated_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(crate::dirs::HOME_ENV, dir.path());
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

/// WHY: the class closed here is "a config file decides updates the
/// release contract did not": update checks default on, the channel
/// defaults to stable, a file written before the keys existed gets
/// those defaults, and each channel and both switch values survive a
/// store and a load under their documented TOML names. Not covered:
/// what the update check does with them (update::tests).
#[test]
#[serial_test::serial]
fn update_keys_default_on_and_stable_and_round_trip_by_name() {
    let cfg = Config::default();
    assert!(cfg.check_for_updates);
    assert_eq!(cfg.update_channel, UpdateChannel::Stable);

    let old: Config = toml::from_str("recording_fps = 24\n").unwrap();
    assert!(old.check_for_updates);
    assert_eq!(old.update_channel, UpdateChannel::Stable);

    for channel in UpdateChannel::ALL {
        for check in [true, false] {
            let _d = isolated_home();
            Config {
                check_for_updates: check,
                update_channel: channel,
                ..Config::default()
            }
            .store()
            .unwrap();
            let text = std::fs::read_to_string(Config::path().unwrap()).unwrap();
            assert!(
                text.contains(&format!("check_for_updates = {check}\n")),
                "{text}"
            );
            assert!(
                text.contains(&format!("update_channel = \"{}\"\n", channel.name())),
                "{text}"
            );
            let back: Config = toml::from_str(&text).unwrap();
            assert_eq!(back.check_for_updates, check);
            assert_eq!(back.update_channel, channel);
        }
    }

    let beta: Config = toml::from_str("update_channel = \"beta\"\n").unwrap();
    assert_eq!(beta.update_channel, UpdateChannel::Beta);
    assert!(toml::from_str::<Config>("update_channel = \"nightly\"\n").is_err());
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

/// A tempdir holding a glint config with `recording_fps = 12`, and its
/// path.
fn glint_config() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("glint/config.toml");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, "recording_fps = 12\n").unwrap();
    (dir, old)
}

#[test]
#[serial_test::serial]
fn glint_config_migrates_to_iris_path() {
    let _d = isolated_home();
    let (_g, old) = glint_config();
    let path = Config::path().unwrap();
    migrate_legacy(&path, &old);
    // The copy landed at the iris path, not just in memory.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "recording_fps = 12\n"
    );
    assert_eq!(Config::load().recording_fps, 12);
}

#[test]
#[serial_test::serial]
fn existing_iris_config_wins_over_glint() {
    let _d = isolated_home();
    let (_g, old) = glint_config();
    let path = Config::path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "recording_fps = 60\n").unwrap();
    migrate_legacy(&path, &old);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "recording_fps = 60\n"
    );
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

// WHY: the class closed here is "the load cache serves a config read
// from another file": it was keyed on mtime and length alone, so a file
// written in the same timestamp tick with the same length read back as
// the first. Not covered: an edit in place that keeps both the mtime
// tick and the length of the file, which the stamp cannot see.
#[test]
#[serial_test::serial]
fn each_config_file_loads_its_own_values_whatever_its_stamp() {
    let tick = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let homes: Vec<_> = [12, 60]
        .into_iter()
        .map(|fps| {
            let home = isolated_home();
            let path = Config::path().unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("recording_fps = {fps}\n")).unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(tick).unwrap();
            (home, fps)
        })
        .collect();
    for (home, fps) in homes.iter().chain(&homes) {
        std::env::set_var(crate::dirs::HOME_ENV, home.path());
        assert_eq!(
            Config::load().recording_fps,
            *fps,
            "{}",
            home.path().display()
        );
    }
}

// WHY: the class closed here is "a default global hotkey belongs to
// someone else, or to no key on the keyboard": `Print` never bound under
// GNOME or with the Windows 11 Snipping Tool, the grab of `Ctrl+Shift+R`
// took hard reload from every browser, and `Ctrl+Print` bound F13 on
// macOS, a key no Mac laptop has. Each row is a chord a stock desktop or
// a common app holds; a default that matches one fails, on either
// platform's defaults, and a macOS default must use a key a Mac
// keyboard has. Not covered: chords a user or a less common desktop
// binds.
#[test]
fn the_default_hotkeys_avoid_chords_desktops_and_apps_hold() {
    const CTRL: u8 = 1;
    const SHIFT: u8 = 2;
    const ALT: u8 = 4;
    const SUPER: u8 = 8;
    let held: &[(&str, u8, &str)] = &[
        (
            "print",
            0,
            "GNOME screenshot UI, Windows 11 Snipping Tool, KDE Spectacle",
        ),
        ("print", SHIFT, "GNOME screenshot, KDE full-screen capture"),
        ("print", ALT, "GNOME window screenshot, Windows window copy"),
        (
            "print",
            SUPER,
            "KDE window capture, Windows screenshot to file",
        ),
        ("print", SUPER | SHIFT, "KDE region capture"),
        ("print", SUPER | CTRL, "KDE monitor capture"),
        ("print", SUPER | ALT, "Windows Game Bar screenshot"),
        ("r", CTRL | SHIFT | ALT, "GNOME screencast"),
        (
            "r",
            SUPER | SHIFT,
            "Windows 11 Snipping Tool recording, KDE region recording",
        ),
        ("r", SUPER | ALT, "Windows Game Bar recording"),
        ("s", SUPER | SHIFT, "Windows Snipping Tool"),
        ("3", SUPER | SHIFT, "macOS screenshot"),
        ("4", SUPER | SHIFT, "macOS region screenshot"),
        ("5", SUPER | SHIFT, "macOS screenshot toolbar"),
        ("6", SUPER | SHIFT, "macOS Touch Bar screenshot"),
        ("r", CTRL, "browser reload"),
        ("r", CTRL | SHIFT, "browser hard reload, editor refactor"),
        ("s", CTRL | SHIFT, "Save As"),
        ("p", CTRL | SHIFT, "command palette, private window"),
        ("f5", 0, "reload"),
        ("f5", CTRL, "hard reload"),
    ];
    for macos in [false, true] {
        let (capture, record) = default_hotkeys(macos);
        for (field, chord) in [("capture_hotkey", capture), ("record_hotkey", record)] {
            for &(key, mods, holder) in held {
                let pressed = |m| mods & m != 0;
                assert!(
                    !keybind_matches(
                        chord,
                        key,
                        pressed(CTRL),
                        pressed(SHIFT),
                        pressed(ALT),
                        pressed(SUPER)
                    ),
                    "macos {macos}: {field} {chord:?} is held by {holder}"
                );
            }
            let key = chord
                .rsplit('+')
                .next()
                .unwrap()
                .trim()
                .to_ascii_lowercase();
            let mac_has = key.len() == 1
                || matches!(
                    key.as_str(),
                    "f1" | "f2"
                        | "f3"
                        | "f4"
                        | "f5"
                        | "f6"
                        | "f7"
                        | "f8"
                        | "f9"
                        | "f10"
                        | "f11"
                        | "f12"
                );
            assert!(
                !macos || mac_has,
                "{field} {chord:?} binds a key a Mac keyboard does not have"
            );
        }
        assert_ne!(
            capture.to_ascii_lowercase(),
            record.to_ascii_lowercase(),
            "macos {macos}: one press would run both commands"
        );
    }
    assert_eq!(
        (
            Config::default().capture_hotkey.as_str(),
            Config::default().record_hotkey.as_str()
        ),
        default_hotkeys(cfg!(target_os = "macos"))
    );
}

// WHY: the class closed here is "a modifier the hotkey grammar reads
// that a pressed-key match ignores": the global hotkey parsers read
// `Option` as Alt and `Command` as the logo key, and a keybind that
// names them must match the same press. Not covered: the parsers
// themselves, which `sys::hotkeys` tests per platform.
#[test]
fn every_modifier_spelling_matches_its_press() {
    // (binding, (ctrl, shift, alt, super))
    for (binding, mods) in [
        ("Ctrl+K", (true, false, false, false)),
        ("control+k", (true, false, false, false)),
        ("Alt+K", (false, false, true, false)),
        ("Option+K", (false, false, true, false)),
        ("Cmd+K", (false, false, false, true)),
        ("Command+K", (false, false, false, true)),
        ("Super+K", (false, false, false, true)),
        ("Win+K", (false, false, false, true)),
        ("Windows+K", (false, false, false, true)),
        ("Meta+K", (false, false, false, true)),
        ("Cmd+Option+Shift+K", (false, true, true, true)),
    ] {
        let (ctrl, shift, alt, super_) = mods;
        assert!(
            keybind_matches(binding, "k", ctrl, shift, alt, super_),
            "{binding:?} does not match its press"
        );
        assert!(
            !keybind_matches(binding, "k", !ctrl, shift, alt, super_)
                && !keybind_matches(binding, "k", ctrl, shift, !alt, super_)
                && !keybind_matches(binding, "k", ctrl, shift, alt, !super_),
            "{binding:?} matches a press with another modifier"
        );
    }
}

/// WHY: the class closed here is "a screenshot file name template that
/// names no single file in the screenshots folder": a path separator
/// writes into a missing subdirectory or out of the folder, and a
/// character Windows rejects fails the save there, so the capture is
/// lost. Every ASCII character and a few non-ASCII ones are tried in a
/// template; an accepted one must expand to a name whose parent is the
/// folder. It does not catch Windows reserved device names such as
/// `CON`, which a `{date}` or `{time}` in the name already avoids.
#[test]
fn a_template_names_one_file_in_the_screenshots_folder() {
    let folder = std::path::Path::new("shots");
    let chars = (0u8..=0x7f)
        .map(char::from)
        .chain(['é', 'ß', '中', '\u{85}']);
    for c in chars {
        let template = format!("a{c}{{date}}");
        match check_template(&template) {
            Ok(()) => {
                let name = format!("{}.png", template.replace("{date}", "2024-01-02"));
                let path = folder.join(&name);
                assert_eq!(path.parent(), Some(folder), "{template:?}");
                assert_eq!(path.file_name(), Some(name.as_ref()), "{template:?}");
                assert!(!c.is_control(), "{template:?} holds a control character");
                assert!(!TEMPLATE_FORBIDDEN.contains(&c), "{template:?}");
            }
            Err(e) if TEMPLATE_FORBIDDEN.contains(&c) => {
                assert_eq!(e, format!("Remove {c}"), "{template:?}")
            }
            Err(e) => {
                assert!(c.is_control(), "{template:?} rejected: {e}");
                assert_eq!(e, "Remove the control character");
            }
        }
    }
    assert_eq!(check_template(""), Err("Enter a file name".into()));
    // No placeholder is required here; Settings asks for one.
    for ok in [DEFAULT_TEMPLATE, "capture", "shot {date} {time}", ".."] {
        assert_eq!(check_template(ok), Ok(()), "{ok:?}");
    }
    assert_eq!(
        check_template(&Config::default().screenshot_template),
        Ok(())
    );
}

/// WHY: a template edited into config.toml by hand is not checked by
/// Settings. Load replaces one that names no single file with the
/// default and keeps every other field and the file as written, the
/// same as an unparsable file keeps its text.
#[test]
#[serial_test::serial]
fn load_replaces_an_unusable_template_and_keeps_the_rest() {
    let d = isolated_home();
    let path = Config::path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for bad in ["a/{date}", "..\\\\{date}", "{date}:{time}", ""] {
        let text = format!("recording_fps = 24\nscreenshot_template = \"{bad}\"\n");
        std::fs::write(&path, &text).unwrap();
        let cfg = Config::load();
        assert_eq!(cfg.screenshot_template, DEFAULT_TEMPLATE, "{bad:?}");
        assert_eq!(cfg.recording_fps, 24, "{bad:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
    std::fs::write(&path, "screenshot_template = \"shot_{time}\"\n").unwrap();
    assert_eq!(Config::load().screenshot_template, "shot_{time}");
    drop(d);
}
