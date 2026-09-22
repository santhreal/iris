use std::path::PathBuf;
use std::prelude::v1::test;

use iris_lib::config::{Config, ToastClickAction, ToastPosition};

use super::*;

#[test]
fn commit_edit_updates_fields_and_validates() {
    let mut s = Settings {
        cfg: Config::default(),
        editing: None,
        recording: None,
        open_dropdown: None,
        status: None,
        focus: None,
        class: "test".into(),
    };

    // Storage paths
    s.editing = Some((Field::ScreenshotsDir, "/tmp/test-shots".into()));
    assert!(s.commit_edit().is_ok());
    assert_eq!(s.cfg.screenshots_dir, PathBuf::from("/tmp/test-shots"));

    s.editing = Some((Field::RecordingsDir, "/tmp/test-recs".into()));
    assert!(s.commit_edit().is_ok());
    assert_eq!(s.cfg.recordings_dir, PathBuf::from("/tmp/test-recs"));

    s.editing = Some((Field::ScreenshotsDir, "   ".into()));
    assert!(s.commit_edit().is_err());

    // Template validation
    s.editing = Some((Field::Template, "shot_{date}_{time}".into()));
    assert!(s.commit_edit().is_ok());
    assert_eq!(s.cfg.screenshot_template, "shot_{date}_{time}");

    s.editing = Some((Field::Template, "no_tokens_here".into()));
    assert!(s.commit_edit().is_err());

    // FPS validation
    s.editing = Some((Field::Fps, "60".into()));
    assert!(s.commit_edit().is_ok());
    assert_eq!(s.cfg.recording_fps, 60);

    s.editing = Some((Field::Fps, "0".into()));
    assert!(s.commit_edit().is_err());

    s.editing = Some((Field::Fps, "121".into()));
    assert!(s.commit_edit().is_err());

    s.editing = Some((Field::Fps, "not_a_number".into()));
    assert!(s.commit_edit().is_err());
}

#[test]
fn field_text_covers_all_field_variants() {
    let mut cfg = Config::default();
    cfg.screenshots_dir = PathBuf::from("/custom/shots");
    cfg.recordings_dir = PathBuf::from("/custom/recs");
    cfg.screenshot_template = "tmpl_{date}".into();
    cfg.recording_fps = 45;
    cfg.capture_hotkey = "Ctrl+Shift+3".into();
    cfg.record_hotkey = "Ctrl+Shift+5".into();
    cfg.cancel_keybind = "Ctrl+C".into();
    cfg.confirm_keybind = "Ctrl+Enter".into();

    let s = Settings {
        cfg,
        editing: None,
        recording: None,
        open_dropdown: None,
        status: None,
        focus: None,
        class: "test".into(),
    };

    assert_eq!(s.field_text(Field::ScreenshotsDir), "/custom/shots");
    assert_eq!(s.field_text(Field::RecordingsDir), "/custom/recs");
    assert_eq!(s.field_text(Field::Template), "tmpl_{date}");
    assert_eq!(s.field_text(Field::Fps), "45");
    assert_eq!(s.field_text(Field::CaptureHotkey), "Ctrl+Shift+3");
    assert_eq!(s.field_text(Field::RecordHotkey), "Ctrl+Shift+5");
    assert_eq!(s.field_text(Field::CancelKeybind), "Ctrl+C");
    assert_eq!(s.field_text(Field::ConfirmKeybind), "Ctrl+Enter");
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
#[serial_test::serial]
fn all_twenty_config_fields_persist_and_reload() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(iris_lib::dirs::HOME_ENV, dir.path());

    let mut s = Settings {
        cfg: Config::default(),
        editing: None,
        recording: None,
        open_dropdown: None,
        status: None,
        focus: None,
        class: "test".into(),
    };

    // Edit Storage
    s.cfg.screenshots_dir = PathBuf::from("/custom/screenshots");
    s.cfg.recordings_dir = PathBuf::from("/custom/recordings");
    s.cfg.screenshot_template = "custom_{date}_{time}".into();

    // Edit Capture
    s.cfg.flash_on_capture = false;
    s.cfg.sound_on_capture = false;
    s.cfg.show_toast_after_capture = false;
    s.cfg.copy_to_clipboard = false;

    // Edit Toast
    s.cfg.toast_click_action = ToastClickAction::OpenFolder;
    s.cfg.toast_drag_enabled = false;
    s.cfg.toast_show_actions = false;
    s.cfg.toast_duration_ms = 8000;
    s.cfg.toast_position = ToastPosition::TopLeft;

    // Edit Recording
    s.cfg.recording_fps = 60;
    s.cfg.record_mic_default = true;
    s.cfg.recording_format = iris_lib::config::RecordingFormat::Webm;
    s.cfg.recording_encoder = iris_lib::config::RecordingEncoder::Nvenc;

    // Edit Keyboard
    s.cfg.capture_hotkey = "Ctrl+Shift+A".into();
    s.cfg.record_hotkey = "Ctrl+Shift+B".into();
    s.cfg.cancel_keybind = "Ctrl+Q".into();
    s.cfg.confirm_keybind = "Ctrl+Space".into();

    assert!(s.cfg.store().is_ok());

    let loaded = Config::load();
    assert_eq!(loaded.screenshots_dir, PathBuf::from("/custom/screenshots"));
    assert_eq!(loaded.recordings_dir, PathBuf::from("/custom/recordings"));
    assert_eq!(loaded.screenshot_template, "custom_{date}_{time}");
    assert_eq!(loaded.flash_on_capture, false);
    assert_eq!(loaded.sound_on_capture, false);
    assert_eq!(loaded.show_toast_after_capture, false);
    assert_eq!(loaded.copy_to_clipboard, false);
    assert_eq!(loaded.toast_click_action, ToastClickAction::OpenFolder);
    assert_eq!(loaded.toast_drag_enabled, false);
    assert_eq!(loaded.toast_show_actions, false);
    assert_eq!(loaded.toast_duration_ms, 8000);
    assert_eq!(loaded.recording_fps, 60);
    assert_eq!(loaded.record_mic_default, true);
    assert_eq!(
        loaded.recording_format,
        iris_lib::config::RecordingFormat::Webm
    );
    assert_eq!(
        loaded.recording_encoder,
        iris_lib::config::RecordingEncoder::Nvenc
    );
    assert_eq!(loaded.capture_hotkey, "Ctrl+Shift+A");
    assert_eq!(loaded.record_hotkey, "Ctrl+Shift+B");
    assert_eq!(loaded.cancel_keybind, "Ctrl+Q");
    assert_eq!(loaded.confirm_keybind, "Ctrl+Space");
}

#[test]
fn reset_defaults_restores_default_config_and_status() {
    let mut s = Settings {
        cfg: Config::default(),
        editing: Some((Field::Template, "test_edit".into())),
        recording: Some(Field::CaptureHotkey),
        open_dropdown: Some(DropdownField::ToastPosition),
        status: None,
        focus: None,
        class: "test".into(),
    };

    // Modify some cfg fields
    s.cfg.screenshots_dir = PathBuf::from("/non/default/dir");
    s.cfg.recording_fps = 120;
    s.cfg.flash_on_capture = false;
    s.cfg.toast_click_action = ToastClickAction::None;

    s.reset_defaults_state();

    let def = Config::default();
    assert_eq!(s.cfg.screenshots_dir, def.screenshots_dir);
    assert_eq!(s.cfg.recording_fps, def.recording_fps);
    assert_eq!(s.cfg.flash_on_capture, def.flash_on_capture);
    assert_eq!(s.cfg.toast_click_action, def.toast_click_action);
    assert_eq!(s.editing, None);
    assert_eq!(s.recording, None);
    assert_eq!(s.open_dropdown, None);
    assert_eq!(
        s.status,
        Some("Defaults restored; Save to apply".to_string())
    );
}
