//! Applying a change: text and folder edits, shortcut recording,
//! switches, pop-up choices, and the rows that show them. Every change
//! that passes validation is written to config.toml at once.

use std::path::{Path, PathBuf};

use gpui::*;
use iris_lib::config::Config;

use super::panes::{Pane, Pick, Switch};
use super::{Edit, Field, Settings};
use crate::{theme, widgets};

/// Field widths: a path or file name template, a number, a shortcut.
const TEXT_W: f32 = 200.0;
const NUMBER_W: f32 = 64.0;
const SHORTCUT_W: f32 = 150.0;
const PATH_W: f32 = 240.0;

/// A configured directory as its field shows it: `~` for the home
/// directory, the form config.toml stores.
fn shown_dir(dir: &Path) -> String {
    iris_lib::config::contract_home(dir).display().to_string()
}

/// A directory typed into a field, kept as typed. It must be absolute
/// once `~` expands: a relative path would resolve against the daemon's
/// working directory, wherever that was when it started.
fn typed_dir(text: &str) -> Result<PathBuf, String> {
    if text.is_empty() {
        return Err("Enter a folder".into());
    }
    let dir = PathBuf::from(text);
    if !iris_lib::config::expand_home(&dir).is_absolute() {
        return Err("Use a full path or one starting with ~".into());
    }
    Ok(dir)
}

/// The config field a shortcut row sets.
fn shortcut_slot(cfg: &mut Config, field: Field) -> Option<&mut String> {
    Some(match field {
        Field::CaptureHotkey => &mut cfg.capture_hotkey,
        Field::RecordHotkey => &mut cfg.record_hotkey,
        Field::CancelKeybind => &mut cfg.cancel_keybind,
        Field::ConfirmKeybind => &mut cfg.confirm_keybind,
        Field::ScreenshotsDir | Field::RecordingsDir | Field::Template | Field::Fps => return None,
    })
}

/// The shortcut that is active at the same time as `field` and already
/// holds `chord`: the two global hotkeys are grabbed together, and the
/// overlay reads its cancel and confirm keys together. Shortcuts from
/// different sets never fire at once.
pub(super) fn conflict(cfg: &Config, field: Field, chord: &str) -> Option<Field> {
    let (other, held) = match field {
        Field::CaptureHotkey => (Field::RecordHotkey, &cfg.record_hotkey),
        Field::RecordHotkey => (Field::CaptureHotkey, &cfg.capture_hotkey),
        Field::CancelKeybind => (Field::ConfirmKeybind, &cfg.confirm_keybind),
        Field::ConfirmKeybind => (Field::CancelKeybind, &cfg.cancel_keybind),
        Field::ScreenshotsDir | Field::RecordingsDir | Field::Template | Field::Fps => {
            return None
        }
    };
    held.trim().eq_ignore_ascii_case(chord.trim()).then_some(other)
}

/// One row: the label on the left, the control right-aligned.
fn row_shell(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .h(px(theme::ROW_H))
        .px(px(theme::ROW_PAD_X))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_size(px(theme::TEXT_BODY))
                .text_color(theme::FG)
                .child(label),
        )
        .child(control)
}

/// A control with the reason its last value was rejected beside it.
fn with_reason(reason: Option<&str>, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_w_0()
        .children(reason.map(|r| {
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(theme::TEXT_SMALL))
                .text_color(theme::DANGER)
                .child(SharedString::from(r.to_owned()))
        }))
        .child(control)
}

impl Settings {
    pub(super) fn field_text(&self, field: Field) -> String {
        match field {
            Field::ScreenshotsDir => shown_dir(&self.cfg.screenshots_dir),
            Field::RecordingsDir => shown_dir(&self.cfg.recordings_dir),
            Field::Template => self.cfg.screenshot_template.clone(),
            Field::Fps => self.cfg.recording_fps.to_string(),
            Field::CaptureHotkey => self.cfg.capture_hotkey.clone(),
            Field::RecordHotkey => self.cfg.record_hotkey.clone(),
            Field::CancelKeybind => self.cfg.cancel_keybind.clone(),
            Field::ConfirmKeybind => self.cfg.confirm_keybind.clone(),
        }
    }

    /// The reason the last value entered into `field` was rejected.
    fn reason(&self, field: Field) -> Option<&str> {
        self.rejected
            .as_ref()
            .filter(|(f, _)| *f == field)
            .map(|(_, r)| r.as_str())
    }

    /// Write the config to config.toml; with `hotkeys`, reload the
    /// global hotkey grabs. A failure shows in the status pill.
    pub(super) fn persist(&mut self, hotkeys: bool) {
        match self.cfg.store() {
            Ok(()) => {
                if hotkeys {
                    crate::daemon::notify_hotkeys_changed();
                }
                self.status = None;
            }
            Err(e) => self.status = Some(e),
        }
    }

    /// End the open edit: commit a typed value that passes validation,
    /// and stop a shortcut recording without a change. Returns false
    /// when the typed value was rejected: the edit stays open with the
    /// reason beside it.
    pub(super) fn blur(&mut self) -> bool {
        if self.recording.take().is_some() {
            self.rejected = None;
        }
        let Some(edit) = self.editing.take() else {
            return true;
        };
        let text = edit.text.trim();
        if text == self.field_text(edit.field) {
            self.rejected = None;
            return true;
        }
        match self.apply_edit(edit.field, text) {
            Ok(()) => {
                self.rejected = None;
                self.persist(false);
                true
            }
            Err(reason) => {
                self.rejected = Some((edit.field, reason));
                self.editing = Some(edit);
                false
            }
        }
    }

    fn apply_edit(&mut self, field: Field, text: &str) -> Result<(), String> {
        match field {
            Field::ScreenshotsDir => self.cfg.screenshots_dir = typed_dir(text)?,
            Field::RecordingsDir => self.cfg.recordings_dir = typed_dir(text)?,
            Field::Template => {
                if !text.contains("{date}") && !text.contains("{time}") {
                    return Err("Include {date} or {time}".into());
                }
                self.cfg.screenshot_template = text.to_string();
            }
            Field::Fps => {
                let fps: u32 = text.parse().map_err(|_| "Enter a number".to_string())?;
                if !(1..=120).contains(&fps) {
                    return Err("Enter 1 to 120".into());
                }
                self.cfg.recording_fps = fps;
            }
            Field::CaptureHotkey
            | Field::RecordHotkey
            | Field::CancelKeybind
            | Field::ConfirmKeybind => {
                return Err("Press the shortcut instead of typing it".into())
            }
        }
        Ok(())
    }

    /// Open `field` for typing with its stored text. A click on the
    /// field already open keeps its text.
    pub(super) fn begin_edit(&mut self, field: Field) {
        if self.editing.as_ref().is_some_and(|e| e.field == field) || !self.blur() {
            return;
        }
        self.open_pick = None;
        self.editing = Some(Edit {
            field,
            text: self.field_text(field),
        });
    }

    /// Wait for the key combination for shortcut `field`.
    pub(super) fn begin_recording(&mut self, field: Field) {
        if !self.blur() {
            return;
        }
        self.open_pick = None;
        self.recording = Some(field);
    }

    /// Set shortcut `field` to `chord` and stop recording, unless the
    /// shortcut active alongside it holds the same chord: then
    /// recording continues with the reason beside the field.
    pub(super) fn record(&mut self, field: Field, chord: String) {
        if let Some(other) = conflict(&self.cfg, field, &chord) {
            self.rejected = Some((field, format!("Used by {}", other.label())));
            return;
        }
        self.recording = None;
        self.rejected = None;
        let Some(slot) = shortcut_slot(&mut self.cfg, field) else {
            return;
        };
        if *slot == chord {
            return;
        }
        *slot = chord;
        self.persist(matches!(field, Field::CaptureHotkey | Field::RecordHotkey));
    }

    /// Set folder `field` to the directory the picker returned. A picker
    /// that cannot open leaves the path open for typing instead.
    pub(super) fn picked(&mut self, field: Field, picked: Result<PathBuf, String>) {
        match picked {
            Ok(dir) => {
                match field {
                    Field::ScreenshotsDir => self.cfg.screenshots_dir = dir,
                    Field::RecordingsDir => self.cfg.recordings_dir = dir,
                    _ => return,
                }
                self.persist(false);
            }
            Err(e) => {
                self.status = Some(format!("{e}; type the folder path instead"));
                self.begin_edit(field);
            }
        }
    }

    pub(super) fn switch_on(&self, s: Switch) -> bool {
        match s {
            Switch::Login => self.login,
            Switch::Flash => self.cfg.flash_on_capture,
            Switch::Sound => self.cfg.sound_on_capture,
            Switch::Clipboard => self.cfg.copy_to_clipboard,
            Switch::Thumbnail => self.cfg.show_toast_after_capture,
            Switch::ToastDrag => self.cfg.toast_drag_enabled,
            Switch::ToastActions => self.cfg.toast_show_actions,
            Switch::Mic => self.cfg.record_mic_default,
            Switch::AutoCheck => self.cfg.check_for_updates,
        }
    }

    /// Turn `s` over and apply it. A login entry that cannot be written
    /// leaves the switch where it was, with the error in the pill.
    pub(super) fn flip(&mut self, s: Switch) {
        self.open_pick = None;
        if let Some(on) = s.slot(&mut self.cfg) {
            *on = !*on;
            self.persist(false);
            return;
        }
        self.login = !self.login;
        match self.store_login() {
            Ok(()) => self.status = None,
            Err(e) => {
                self.login = !self.login;
                self.status = Some(e);
            }
        }
    }

    /// Apply option `ix` of `p`. Returns whether the value changed.
    pub(super) fn choose(&mut self, p: Pick, ix: usize) -> bool {
        self.open_pick = None;
        let before = p.selected(&self.cfg);
        p.select(&mut self.cfg, ix);
        if p.selected(&self.cfg) == before {
            return false;
        }
        self.persist(false);
        true
    }

    /// Restore every config.toml setting to its default and apply it.
    /// Start at login is the login entry, not a config setting, and
    /// stays.
    pub(super) fn restore_defaults(&mut self) {
        self.cfg = Config::default();
        self.editing = None;
        self.recording = None;
        self.rejected = None;
        self.open_pick = None;
        self.persist(true);
    }

    /// Format a pressed keystroke the way config.toml stores hotkeys:
    /// "Ctrl+Shift+R", "Print", "Escape", "Enter".
    pub(super) fn format_keystroke(ev: &KeyDownEvent) -> Option<String> {
        let key = ev.keystroke.key.as_str();
        // Bare modifier presses are not a hotkey yet.
        if matches!(key, "control" | "shift" | "alt" | "super" | "meta") {
            return None;
        }
        let m = &ev.keystroke.modifiers;
        let mut parts: Vec<&str> = Vec::new();
        if m.control {
            parts.push("Ctrl");
        }
        if m.alt {
            parts.push("Alt");
        }
        if m.shift {
            parts.push("Shift");
        }
        if m.platform && !m.control {
            parts.push("Super");
        }
        let named = match key {
            " " => "Space".to_string(),
            "escape" => "Escape".to_string(),
            "enter" => "Enter".to_string(),
            "print" | "printscreen" => "Print".to_string(),
            k if k.len() == 1 => k.to_uppercase(),
            k => {
                let mut c = k.chars();
                let first = c.next()?;
                first.to_uppercase().collect::<String>() + c.as_str()
            }
        };
        parts.push(&named);
        Some(parts.join("+"))
    }

    pub(super) fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        // Recording takes every combination, Esc included: Esc is the
        // default cancel key. A click away stops it.
        if let Some(field) = self.recording {
            if let Some(chord) = Self::format_keystroke(ev) {
                self.record(field, chord);
                cx.notify();
            }
            return;
        }
        let meta = ks.modifiers.control || ks.modifiers.platform;
        if meta && ks.key == "w" {
            self.blur();
            window.remove_window();
            return;
        }
        if let Some(pane) = Pane::for_key(&ks.key).filter(|_| meta) {
            self.select_pane(pane, window);
            cx.notify();
            return;
        }
        if let Some(edit) = &mut self.editing {
            match ks.key.as_str() {
                "escape" => {
                    self.editing = None;
                    self.rejected = None;
                }
                "enter" => {
                    self.blur();
                }
                "backspace" => {
                    edit.text.pop();
                    self.rejected = None;
                }
                _ => {
                    if let (false, Some(ch)) = (meta, &ks.key_char) {
                        edit.text.push_str(ch);
                        self.rejected = None;
                    }
                }
            }
            cx.notify();
            return;
        }
        if ks.key == "escape" {
            if self.open_pick.take().is_none() {
                window.remove_window();
            }
            cx.notify();
        }
    }

    pub(super) fn switch_row(&self, s: Switch, cx: &mut Context<Self>) -> Div {
        row_shell(
            s.label(),
            widgets::toggle(s.id(), self.switch_on(s)).on_click(cx.listener(move |this, _, _, cx| {
                this.flip(s);
                cx.notify();
            })),
        )
    }

    pub(super) fn pick_row(&self, p: Pick, cx: &mut Context<Self>) -> Div {
        let open = self.open_pick == Some(p);
        let selected = p.selected(&self.cfg);
        // The press toggles from the state this frame drew: with the
        // menu open, its press-outside dismissal runs first, and the
        // button must not open it again.
        let button = widgets::popup_button(p.id(), p.current(&self.cfg), open).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.open_pick = if open { None } else { Some(p) };
                cx.notify();
            }),
        );
        let mut control = div().relative().child(button);
        if open {
            let this = cx.entity().downgrade();
            let dismiss = this.clone();
            control = control.child(widgets::popup_menu(
                p.menu_id(),
                p.options(),
                selected,
                0.0,
                move |ix, _, cx| {
                    this.update(cx, |this, cx| {
                        if this.choose(p, ix) && p == Pick::UpdateChannel {
                            this.check_updates(cx);
                        }
                        cx.notify();
                    })
                    .ok();
                },
                move |_, cx| {
                    dismiss
                        .update(cx, |this, cx| {
                            this.open_pick = None;
                            cx.notify();
                        })
                        .ok();
                },
            ));
        }
        row_shell(p.label(), control)
    }

    /// A text field for `field`, `width` wide, open for typing while it
    /// is the edit in progress.
    fn typed_field(&self, field: Field, width: f32, cx: &mut Context<Self>) -> Div {
        let reason = self.reason(field);
        let (text, state) = match self.editing.as_ref().filter(|e| e.field == field) {
            Some(edit) => (
                edit.text.clone(),
                widgets::FieldState::Editing {
                    invalid: reason.is_some(),
                },
            ),
            None => (self.field_text(field), widgets::FieldState::Rest),
        };
        let input = widgets::text_field(field.id(), text, state)
            .w(px(width))
            .flex_shrink_0()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.begin_edit(field);
                cx.notify();
            }));
        with_reason(reason, input)
    }

    pub(super) fn text_row(&self, field: Field, cx: &mut Context<Self>) -> Div {
        let width = if field == Field::Fps { NUMBER_W } else { TEXT_W };
        row_shell(field.label(), self.typed_field(field, width, cx))
    }

    pub(super) fn folder_row(&self, field: Field, cx: &mut Context<Self>) -> Div {
        if self.editing.as_ref().is_some_and(|e| e.field == field) {
            return row_shell(field.label(), self.typed_field(field, PATH_W, cx));
        }
        let this = cx.entity().downgrade();
        row_shell(
            field.label(),
            widgets::folder_field(field.id(), self.field_text(field), PATH_W, move |picked, _, cx| {
                this.update(cx, |this, cx| {
                    this.picked(field, picked);
                    cx.notify();
                })
                .ok();
            }),
        )
    }

    pub(super) fn hotkey_row(&self, field: Field, cx: &mut Context<Self>) -> Div {
        let (text, state) = if self.recording == Some(field) {
            ("Type shortcut".to_string(), widgets::FieldState::Recording)
        } else {
            (self.field_text(field), widgets::FieldState::Rest)
        };
        let input = widgets::text_field(field.id(), text, state)
            .w(px(SHORTCUT_W))
            .flex_shrink_0()
            .justify_center()
            .font_features(theme::tabular())
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.begin_recording(field);
                cx.notify();
            }));
        row_shell(field.label(), with_reason(self.reason(field), input))
    }

    pub(super) fn restore_row(&self, cx: &mut Context<Self>) -> Div {
        row_shell(
            "Restore every setting to its default",
            widgets::push_button("restore-defaults", "Restore Defaults", widgets::ButtonStyle::Destructive)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.restore_defaults();
                    cx.notify();
                })),
        )
    }
}
