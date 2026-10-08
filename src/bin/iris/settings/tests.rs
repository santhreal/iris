use std::path::PathBuf;
use std::prelude::v1::test;
use std::time::{Duration, SystemTime};

use iris_lib::config::Config;

use super::panes::{Pick, Switch};
use super::*;

fn settings(release: Release) -> Settings {
    Settings::new(Config::default(), false, release, None)
}

/// A fresh config directory under a temp home for a test that writes
/// config.toml. The returned guard keeps the directory alive.
fn temp_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(iris_lib::dirs::HOME_ENV, dir.path());
    dir
}

fn edit(s: &mut Settings, field: Field, text: &str) {
    s.editing = Some(Edit {
        field,
        text: text.into(),
    });
}

// WHY: the class closed here is "a change shows in the window but never
// reaches config.toml", the failure mode of instant apply. Every switch,
// every option of every pop-up, a committed text field, a recorded
// shortcut, a chosen folder, and Restore Defaults are each read back
// from the file, not from the window's copy. The member lists come from
// the pane tables' ALL arrays, which the coverage test below ties to the
// panes. Not covered: the hotkey regrab after a shortcut change, which
// the X11 rig exercises.
#[test]
#[serial_test::serial]
fn every_switch_writes_config_toml_on_the_click() {
    let _home = temp_home();
    let mut s = settings(Release::None);
    for sw in Switch::ALL {
        if sw == Switch::Login {
            continue;
        }
        let before = s.switch_on(sw);
        s.flip(sw);
        assert_eq!(s.switch_on(sw), !before, "{sw:?} in the window");
        let mut on_disk = Config::load();
        assert_eq!(
            sw.slot(&mut on_disk).copied(),
            Some(!before),
            "{sw:?} in config.toml"
        );
        s.flip(sw);
        let mut on_disk = Config::load();
        assert_eq!(sw.slot(&mut on_disk).copied(), Some(before), "{sw:?} back");
    }
    assert_eq!(s.status, None);
}

#[test]
#[serial_test::serial]
fn every_option_of_every_pop_up_round_trips_through_config_toml() {
    let _home = temp_home();
    let mut s = settings(Release::None);
    for p in Pick::ALL {
        let options = p.options();
        assert!(options.len() >= 2, "{p:?} offers a choice");
        // Last to first, so the first pick changes the default value.
        for ix in (0..options.len()).rev() {
            s.choose(p, ix);
            assert_eq!(p.selected(&s.cfg), Some(ix), "{p:?} option {ix}");
            assert_eq!(p.current(&s.cfg), options[ix]);
            assert_eq!(
                p.selected(&Config::load()),
                Some(ix),
                "{p:?} option {ix} in config.toml"
            );
        }
        // A pick of the value already set changes nothing.
        assert!(!s.choose(p, 0), "{p:?} reselect");
        assert_eq!(s.open_pick, None);
    }
}

#[test]
#[serial_test::serial]
fn a_text_field_applies_on_commit_and_keeps_a_rejected_value_open() {
    let _home = temp_home();
    let mut s = settings(Release::None);

    let shots = std::env::temp_dir().join("test-shots");
    edit(&mut s, Field::ScreenshotsDir, &shots.display().to_string());
    assert!(s.blur());
    assert_eq!(s.editing, None);
    assert_eq!(Config::load().screenshots_dir, shots);

    edit(&mut s, Field::Fps, " 60 ");
    assert!(s.blur());
    assert_eq!(Config::load().recording_fps, 60);

    edit(&mut s, Field::Template, "shot_{date}_{time}");
    assert!(s.blur());
    assert_eq!(Config::load().screenshot_template, "shot_{date}_{time}");

    // A rejected value stays in its field, as typed, with the reason
    // beside it; config.toml keeps the last good value.
    let cases: [(Field, &str, &str); 8] = [
        (Field::ScreenshotsDir, "   ", "Enter a folder"),
        (
            Field::ScreenshotsDir,
            "test-shots",
            "Use a full path or one starting with ~",
        ),
        (
            Field::ScreenshotsDir,
            "~user/shots",
            "Use a full path or one starting with ~",
        ),
        (
            Field::Template,
            "no_tokens_here",
            "Include {date} or {time}",
        ),
        (Field::Fps, "0", "Enter 1 to 120"),
        (Field::Fps, "121", "Enter 1 to 120"),
        (Field::Fps, "sixty", "Enter a number"),
        (
            Field::CaptureHotkey,
            "Ctrl+K",
            "Press the shortcut instead of typing it",
        ),
    ];
    for (field, bad, reason) in cases {
        edit(&mut s, field, bad);
        assert!(!s.blur(), "{bad:?}");
        assert_eq!(s.editing.as_ref().map(|e| e.text.as_str()), Some(bad));
        assert_eq!(s.rejected, Some((field, reason.to_string())), "{bad:?}");
        // A second commit of the same text is rejected again.
        assert!(!s.blur());
        s.editing = None;
        s.rejected = None;
    }
    let on_disk = Config::load();
    assert_eq!(on_disk.screenshots_dir, shots);
    assert_eq!(on_disk.screenshot_template, "shot_{date}_{time}");
    assert_eq!(on_disk.recording_fps, 60);
    assert_eq!(on_disk.capture_hotkey, Config::default().capture_hotkey);

    // Correcting a rejected value commits it and clears the reason.
    edit(&mut s, Field::Fps, "0");
    assert!(!s.blur());
    edit(&mut s, Field::Fps, "24");
    assert!(s.blur());
    assert_eq!(s.rejected, None);
    assert_eq!(Config::load().recording_fps, 24);
}

#[test]
fn opening_a_field_starts_from_the_stored_value_and_keeps_its_own_edit() {
    let mut s = settings(Release::None);
    s.begin_edit(Field::Template);
    assert_eq!(
        s.editing,
        Some(Edit {
            field: Field::Template,
            text: "{date}_{time}".into()
        })
    );
    // A click into the field already open keeps the typed text.
    s.editing.as_mut().unwrap().text.push_str("_x");
    s.begin_edit(Field::Template);
    assert_eq!(s.editing.as_ref().unwrap().text, "{date}_{time}_x");
    // A rejected edit keeps focus: another field does not open.
    edit(&mut s, Field::Fps, "0");
    s.begin_edit(Field::Template);
    assert_eq!(s.editing.as_ref().unwrap().field, Field::Fps);
    s.begin_recording(Field::RecordHotkey);
    assert_eq!(s.recording, None);
}

#[test]
#[serial_test::serial]
fn a_recorded_shortcut_applies_unless_its_partner_holds_it() {
    let _home = temp_home();
    let mut s = settings(Release::None);

    s.begin_recording(Field::CaptureHotkey);
    s.record(Field::CaptureHotkey, "Ctrl+Shift+3".into());
    assert_eq!(s.recording, None);
    assert_eq!(Config::load().capture_hotkey, "Ctrl+Shift+3");

    // The two global hotkeys are grabbed together; the overlay reads
    // cancel and confirm together. Case differs in a hand-edited file.
    let pairs = [
        (Field::RecordHotkey, Field::CaptureHotkey, "ctrl+shift+3"),
        (Field::CaptureHotkey, Field::RecordHotkey, "Ctrl+Shift+R"),
        (Field::ConfirmKeybind, Field::CancelKeybind, "Escape"),
        (Field::CancelKeybind, Field::ConfirmKeybind, "Enter"),
    ];
    for (field, partner, chord) in pairs {
        let before = s.field_text(field);
        s.begin_recording(field);
        s.record(field, chord.into());
        assert_eq!(s.recording, Some(field), "{field:?} keeps recording");
        assert_eq!(
            s.rejected,
            Some((field, format!("Used by {}", partner.label())))
        );
        assert_eq!(s.field_text(field), before, "{field:?} unchanged");
        s.blur();
        assert_eq!(s.rejected, None, "a click away clears the reason");
    }
    // Shortcuts from different sets may share a chord.
    s.begin_recording(Field::ConfirmKeybind);
    s.record(Field::ConfirmKeybind, "Ctrl+Shift+3".into());
    assert_eq!(Config::load().confirm_keybind, "Ctrl+Shift+3");
}

#[test]
#[serial_test::serial]
fn a_chosen_folder_applies_and_a_picker_failure_opens_the_path_for_typing() {
    let _home = temp_home();
    let mut s = settings(Release::None);
    let recs = std::env::temp_dir().join("test-recs");
    s.picked(Field::RecordingsDir, Ok(recs.clone()));
    assert_eq!(Config::load().recordings_dir, recs);

    s.picked(
        Field::ScreenshotsDir,
        Err("folder picker: no portal".into()),
    );
    assert_eq!(
        s.status.as_deref(),
        Some("folder picker: no portal; type the folder path instead")
    );
    assert_eq!(
        s.editing.as_ref().map(|e| e.field),
        Some(Field::ScreenshotsDir)
    );
}

#[test]
#[serial_test::serial]
fn restore_defaults_writes_the_defaults() {
    let _home = temp_home();
    let mut s = settings(Release::None);
    s.flip(Switch::Flash);
    s.choose(Pick::ToastPosition, 3);
    edit(&mut s, Field::Fps, "0");
    s.rejected = Some((Field::Fps, "Enter 1 to 120".into()));
    s.open_pick = Some(Pick::RecordingFormat);

    s.restore_defaults();
    let def = Config::default();
    let on_disk = Config::load();
    assert_eq!(on_disk.flash_on_capture, def.flash_on_capture);
    assert_eq!(on_disk.toast_position, def.toast_position);
    assert_eq!(s.editing, None);
    assert_eq!(s.rejected, None);
    assert_eq!(s.open_pick, None);
}

// WHY: closes "a setting has no row, or two rows" and "a pane does not
// fit its window". Every switch, pop-up, and field must appear in
// exactly one pane: a new member of any of the three enums fails here
// until it is placed. A pane's window height is computed from its rows;
// the window's minimum size must let the shortest pane through (a
// window manager holds a window at its minimum size hint), and the
// tallest pane must fit a 768-pixel-high screen. Not covered: the
// rendered row heights matching Row::height, which docshots check.
#[test]
fn every_setting_has_exactly_one_row_and_every_pane_fits_its_window() {
    let rows: Vec<Row> = Pane::ALL
        .iter()
        .flat_map(|p| p.groups().iter().flat_map(|g| g.rows.iter().copied()))
        .collect();
    let count = |want: Row| rows.iter().filter(|&&r| r == want).count();
    for sw in Switch::ALL {
        assert_eq!(count(Row::Switch(sw)), 1, "{sw:?}");
    }
    for p in Pick::ALL {
        assert_eq!(count(Row::Pick(p)), 1, "{p:?}");
    }
    for f in Field::ALL {
        let n = count(Row::Text(f)) + count(Row::Folder(f)) + count(Row::Hotkey(f));
        assert_eq!(n, 1, "{f:?}");
    }
    assert_eq!(count(Row::Version), 1);
    assert_eq!(count(Row::Restore), 1);

    let min = min_size();
    for p in Pane::ALL {
        let h = p.height();
        assert!(px(h) >= min.height, "{p:?}: {h} under the minimum");
        assert!(
            h <= 700.0,
            "{p:?}: {h} is taller than a 768px screen allows"
        );
        assert_eq!(Pane::ALL[p.index()], p);
    }
}

// WHY: Ctrl/Cmd+1..5 select the tabs in their drawn order. An
// off-by-one maps Ctrl+1 to the second tab, and an unchecked index
// panics on Ctrl+0 or Ctrl+9; both fail here.
#[test]
fn pane_shortcuts_follow_tab_order_and_reject_other_keys() {
    for (i, p) in Pane::ALL.iter().enumerate() {
        assert_eq!(Pane::for_key(&(i + 1).to_string()), Some(*p));
    }
    let past = (Pane::ALL.len() + 1).to_string();
    for key in ["0", past.as_str(), "9", "a", "", "-1", "f1"] {
        assert_eq!(Pane::for_key(key), None, "{key:?}");
    }
}

/// The storage fields show the home directory as `~`, the form
/// config.toml stores; a directory elsewhere shows in full.
#[test]
fn field_text_covers_all_field_variants() {
    let home = directories::UserDirs::new()
        .unwrap()
        .home_dir()
        .to_path_buf();
    let cfg = Config {
        screenshots_dir: PathBuf::from("/custom/shots"),
        recordings_dir: home.join("Videos").join("iris"),
        screenshot_template: "tmpl_{date}".into(),
        recording_fps: 45,
        capture_hotkey: "Ctrl+Shift+3".into(),
        record_hotkey: "Ctrl+Shift+5".into(),
        cancel_keybind: "Ctrl+C".into(),
        confirm_keybind: "Ctrl+Enter".into(),
        ..Config::default()
    };
    let s = Settings::new(cfg, false, Release::None, None);

    assert_eq!(s.field_text(Field::ScreenshotsDir), "/custom/shots");
    assert_eq!(
        s.field_text(Field::RecordingsDir),
        std::path::Path::new("~")
            .join("Videos")
            .join("iris")
            .display()
            .to_string()
    );
    assert_eq!(s.field_text(Field::Template), "tmpl_{date}");
    assert_eq!(s.field_text(Field::Fps), "45");
    assert_eq!(s.field_text(Field::CaptureHotkey), "Ctrl+Shift+3");
    assert_eq!(s.field_text(Field::RecordHotkey), "Ctrl+Shift+5");
    assert_eq!(s.field_text(Field::CancelKeybind), "Ctrl+C");
    assert_eq!(s.field_text(Field::ConfirmKeybind), "Ctrl+Enter");
}

#[test]
fn a_hand_written_duration_shows_as_itself() {
    let cfg = Config {
        toast_duration_ms: 4500,
        ..Config::default()
    };
    assert_eq!(Pick::ToastDuration.selected(&cfg), None);
    assert_eq!(Pick::ToastDuration.current(&cfg), "4.5 seconds");
    let cfg = Config {
        toast_duration_ms: 1000,
        ..Config::default()
    };
    assert_eq!(Pick::ToastDuration.current(&cfg), "1 second");
}

#[test]
fn format_keystroke_formats_keys_and_modifiers() {
    let make_event =
        |key: &str, ctrl: bool, alt: bool, shift: bool, super_key: bool| KeyDownEvent {
            keystroke: Keystroke {
                modifiers: Modifiers {
                    control: ctrl,
                    alt,
                    shift,
                    platform: super_key,
                    function: false,
                },
                key: key.to_string(),
                key_char: None,
            },
            is_held: false,
            prefer_character_input: false,
        };

    // Bare modifier is ignored
    assert_eq!(
        Settings::format_keystroke(&make_event("control", true, false, false, false)),
        None
    );
    assert_eq!(
        Settings::format_keystroke(&make_event("shift", false, false, true, false)),
        None
    );

    // Named keys
    assert_eq!(
        Settings::format_keystroke(&make_event("escape", false, false, false, false)),
        Some("Escape".into())
    );
    assert_eq!(
        Settings::format_keystroke(&make_event("enter", false, false, false, false)),
        Some("Enter".into())
    );
    assert_eq!(
        Settings::format_keystroke(&make_event("print", false, false, false, false)),
        Some("Print".into())
    );
    assert_eq!(
        Settings::format_keystroke(&make_event(" ", false, false, false, false)),
        Some("Space".into())
    );

    // Single characters uppercase
    assert_eq!(
        Settings::format_keystroke(&make_event("a", false, false, false, false)),
        Some("A".into())
    );

    // Function keys capitalized
    assert_eq!(
        Settings::format_keystroke(&make_event("f9", false, false, false, false)),
        Some("F9".into())
    );

    // Modifier combos
    assert_eq!(
        Settings::format_keystroke(&make_event("r", true, false, true, false)),
        Some("Ctrl+Shift+R".into())
    );
    assert_eq!(
        Settings::format_keystroke(&make_event("s", true, true, false, false)),
        Some("Ctrl+Alt+S".into())
    );
    assert_eq!(
        Settings::format_keystroke(&make_event("p", false, false, false, true)),
        Some("Super+P".into())
    );
}

#[test]
fn the_last_check_reads_today_or_its_date() {
    let now = SystemTime::now();
    let line = app::checked_line(now, now);
    assert!(line.starts_with("Last checked today at "), "{line}");
    let (.., h, mi, _) = iris_lib::time::local_fields(
        now.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    );
    assert!(line.ends_with(&format!("{h:02}:{mi:02}")), "{line}");
    let earlier = app::checked_line(now - Duration::from_secs(3 * 86_400), now);
    assert!(earlier.starts_with("Last checked on "), "{earlier}");
}

fn update(patch: u64) -> crate::update::UpdateInfo {
    crate::update::UpdateInfo {
        version: semver::Version::new(9, 9, patch),
        asset_url: String::new(),
        asset_name: String::new(),
        checksum_url: String::new(),
        signature_url: String::new(),
    }
}

/// The version `release` offers to install, or its state's name.
fn offered(release: &Release) -> String {
    match release {
        Release::None => "none".to_string(),
        Release::Found(info) => info.version.to_string(),
        Release::Installing => "installing".to_string(),
    }
}

// WHY: the classes closed here are "Install shows for a release no
// check found", "a second Install click, or a check that finishes
// during the download, starts a second install or changes the one
// running", and "a failed download leaves no Install to retry". Not
// covered: the download and hand-off, which update::tests and the
// Settings QA rig run.
#[test]
fn a_check_offers_install_only_for_a_newer_release() {
    let cases: [(Result<Option<_>, String>, &str, &str); 3] = [
        (Ok(Some(update(3))), "Version 9.9.3 is available", "9.9.3"),
        (Ok(None), "iris is up to date", "none"),
        (
            Err("update: GET x: dns".into()),
            "update: GET x: dns",
            "none",
        ),
    ];
    for (result, note, want) in cases {
        let mut s = settings(Release::Found(update(1)));
        s.checked(result);
        assert_eq!(s.update_note.as_deref(), Some(note));
        assert_eq!(offered(&s.release), want, "{note}");
    }
    let mut s = settings(Release::None);
    assert!(s.start_install().is_none());
    assert_eq!(offered(&s.release), "none");
    assert_eq!(s.update_note, None);
}

#[test]
fn install_runs_once_and_offers_itself_again_when_it_fails() {
    let mut s = settings(Release::Found(update(1)));
    let info = s.start_install().expect("a release to install");
    assert_eq!(info.version.to_string(), "9.9.1");
    assert_eq!(s.update_note.as_deref(), Some("Downloading 9.9.1\u{2026}"));
    assert_eq!(offered(&s.release), "installing");

    assert!(s.start_install().is_none(), "a second click");
    s.checked(Ok(Some(update(2))));
    s.checked(Err("update: GET x: dns".into()));
    assert_eq!(offered(&s.release), "installing");
    assert_eq!(s.update_note.as_deref(), Some("Downloading 9.9.1\u{2026}"));

    s.installed(info, Err("update: GET x: status code 404".into()));
    assert_eq!(
        s.update_note.as_deref(),
        Some("update: GET x: status code 404")
    );
    assert_eq!(offered(&s.release), "9.9.1");

    let info = s.start_install().expect("offered again");
    s.installed(info, Ok(()));
    assert_eq!(
        s.update_note.as_deref(),
        Some("Installing 9.9.1; iris restarts when it is done")
    );
    assert_eq!(offered(&s.release), "installing");
}
