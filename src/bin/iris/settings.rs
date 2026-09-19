//! Settings window: one landscape form over config.toml.
//!
//! Every field edits the loaded Config; Save validates and persists
//! with Config::store. Text fields are click-to-edit (type, Enter
//! commits, Esc cancels), hotkey fields are press-to-record: click the
//! field, press the combination, it is captured verbatim. Dropdowns
//! toggle a floating option list and write the selected value on click.

use std::path::PathBuf;

use gpui::*;
use iris_lib::config::{Config, ToastClickAction, ToastPosition};

use crate::theme;

const ROW_H: f32 = 38.0;
const FIELD_W: f32 = 280.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    ScreenshotsDir,
    RecordingsDir,
    Template,
    Fps,
    CaptureHotkey,
    RecordHotkey,
    CancelKeybind,
    ConfirmKeybind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DropdownField {
    ToastClickAction,
    ToastDuration,
    ToastPosition,
    RecordingFormat,
    RecordingEncoder,
}

pub struct Settings {
    cfg: Config,
    editing: Option<(Field, String)>,
    recording: Option<Field>,
    open_dropdown: Option<DropdownField>,
    status: Option<String>,
    focus: Option<FocusHandle>,
    /// This window's unique WM_CLASS, for the title-bar drag.
    class: String,
}

/// Open the settings window. A second call focuses a new window; the
/// daemon milestone owns single-instance behavior for all windows.
pub fn open(cx: &mut App) -> Result<(), String> {
    let focus = Some(cx.focus_handle());
    let win = (680.0f32, 650.0f32);
    let origin = crate::xwin::centered_origin(cx, win.0, win.1, (220.0, 140.0));
    let win_id = crate::xwin::unique_id("dev.iris.settings");
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(origin.0), px(origin.1)),
                size: size(px(win.0), px(win.1)),
            })),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_movable: true,
            is_resizable: true,
            is_minimizable: true,
            display_id: None,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some(win_id.clone()),
            window_min_size: Some(size(px(640.), px(600.))),
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
        },
        |_, cx| {
            cx.new(|_| Settings {
                cfg: Config::load(),
                editing: None,
                recording: None,
                open_dropdown: None,
                status: None,
                focus,
                class: win_id.clone(),
            })
        },
    )
    .map_err(|e| format!("open settings window: {e}"))?;
    crate::xwin::place_after_map(win_id, origin.0, origin.1);
    Ok(())
}

impl Settings {
    fn field_text(&self, field: Field) -> String {
        match field {
            Field::ScreenshotsDir => self.cfg.screenshots_dir.display().to_string(),
            Field::RecordingsDir => self.cfg.recordings_dir.display().to_string(),
            Field::Template => self.cfg.screenshot_template.clone(),
            Field::Fps => self.cfg.recording_fps.to_string(),
            Field::CaptureHotkey => self.cfg.capture_hotkey.clone(),
            Field::RecordHotkey => self.cfg.record_hotkey.clone(),
            Field::CancelKeybind => self.cfg.cancel_keybind.clone(),
            Field::ConfirmKeybind => self.cfg.confirm_keybind.clone(),
        }
    }

    /// Commit the editing buffer into the config. Returns Err on a
    /// validation failure; the buffer stays open for correction.
    fn commit_edit(&mut self) -> Result<(), String> {
        let Some((field, text)) = self.editing.take() else {
            return Ok(());
        };
        let text = text.trim().to_string();
        match field {
            Field::ScreenshotsDir => {
                if text.is_empty() {
                    return Err("directory cannot be empty".into());
                }
                self.cfg.screenshots_dir = PathBuf::from(text);
            }
            Field::RecordingsDir => {
                if text.is_empty() {
                    return Err("directory cannot be empty".into());
                }
                self.cfg.recordings_dir = PathBuf::from(text);
            }
            Field::Template => {
                if !text.contains("{date}") && !text.contains("{time}") {
                    return Err("template needs {date} or {time}".into());
                }
                self.cfg.screenshot_template = text;
            }
            Field::Fps => {
                let fps: u32 = text.parse().map_err(|_| "fps must be a number".to_string())?;
                if !(1..=120).contains(&fps) {
                    return Err("fps must be 1-120".into());
                }
                self.cfg.recording_fps = fps;
            }
            Field::CaptureHotkey
            | Field::RecordHotkey
            | Field::CancelKeybind
            | Field::ConfirmKeybind => {} // hotkeys are recorded, not typed
        }
        Ok(())
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.open_dropdown = None;
        if self.editing.is_some() {
            if let Err(e) = self.commit_edit() {
                self.status = Some(e);
                cx.notify();
                return;
            }
        }
        self.status = Some(match self.cfg.store() {
            Ok(()) => {
                crate::daemon::notify_hotkeys_changed();
                "saved".to_string()
            }
            Err(e) => e,
        });
        cx.notify();
    }
    fn reset_defaults_state(&mut self) {
        self.cfg = Config::default();
        self.editing = None;
        self.recording = None;
        self.open_dropdown = None;
        self.status = Some("Defaults restored — Save to apply".to_string());
    }

    fn reset_to_defaults(&mut self, cx: &mut Context<Self>) {
        self.reset_defaults_state();
        cx.notify();
    }

    /// Format a pressed keystroke the way config.toml stores hotkeys:
    /// "Ctrl+Shift+R", "Print", "Escape", "Enter".
    fn format_keystroke(ev: &KeyDownEvent) -> Option<String> {
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
                match c.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                    None => return None,
                }
            }
        };
        parts.push(&named);
        Some(parts.join("+"))
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Hotkey recording captures everything, including Esc.
        if self.recording.is_some() {
            if let Some(hotkey) = Self::format_keystroke(ev) {
                match self.recording.take() {
                    Some(Field::CaptureHotkey) => self.cfg.capture_hotkey = hotkey,
                    Some(Field::RecordHotkey) => self.cfg.record_hotkey = hotkey,
                    Some(Field::CancelKeybind) => self.cfg.cancel_keybind = hotkey,
                    Some(Field::ConfirmKeybind) => self.cfg.confirm_keybind = hotkey,
                    _ => {}
                }
            }
            cx.notify();
            return;
        }
        if self.open_dropdown.is_some() {
            if ev.keystroke.key == "escape" {
                self.open_dropdown = None;
                cx.notify();
                return;
            }
        }
        if self.editing.is_none() {
            if ev.keystroke.key == "escape" {
                window.remove_window();
            }
            return;
        }
        match ev.keystroke.key.as_str() {
            "escape" => {
                self.editing = None;
            }
            "enter" => {
                if let Err(e) = self.commit_edit() {
                    self.status = Some(e);
                }
            }
            "backspace" => {
                if let Some((_, buffer)) = &mut self.editing {
                    buffer.pop();
                }
            }
            _ => {
                if !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform {
                    if let (Some((_, buffer)), Some(ch)) =
                        (&mut self.editing, &ev.keystroke.key_char)
                    {
                        buffer.push_str(ch);
                    }
                }
            }
        }
        cx.notify();
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(focus) = &self.focus {
            focus.focus(window);
        }
        let mut form = div().flex().flex_col().gap(px(20.)).p(px(24.));

        // Storage section
        form = form.child(self.section(
            "Storage",
            vec![
                self.text_row("Screenshots folder", Field::ScreenshotsDir, cx)
                    .into_any_element(),
                self.text_row("Recordings folder", Field::RecordingsDir, cx)
                    .into_any_element(),
                self.text_row("Filename template", Field::Template, cx)
                    .into_any_element(),
            ],
        ));

        // Capture section
        form = form.child(self.section(
            "Capture",
            vec![
                self.toggle_row(
                    "tog-flash",
                    "Flash on capture",
                    self.cfg.flash_on_capture,
                    cx.listener(|this, _, _, cx| {
                        this.cfg.flash_on_capture = !this.cfg.flash_on_capture;
                        this.open_dropdown = None;
                        cx.notify();
                    }),
                )
                .into_any_element(),
                self.toggle_row(
                    "tog-sound",
                    "Shutter sound",
                    self.cfg.sound_on_capture,
                    cx.listener(|this, _, _, cx| {
                        this.cfg.sound_on_capture = !this.cfg.sound_on_capture;
                        this.open_dropdown = None;
                        cx.notify();
                    }),
                )
                .into_any_element(),
                self.toggle_row(
                    "tog-toast",
                    "Show thumbnail after capture",
                    self.cfg.show_toast_after_capture,
                    cx.listener(|this, _, _, cx| {
                        this.cfg.show_toast_after_capture = !this.cfg.show_toast_after_capture;
                        this.open_dropdown = None;
                        cx.notify();
                    }),
                )
                .into_any_element(),
                self.toggle_row(
                    "tog-clipboard",
                    "Copy to clipboard",
                    self.cfg.copy_to_clipboard,
                    cx.listener(|this, _, _, cx| {
                        this.cfg.copy_to_clipboard = !this.cfg.copy_to_clipboard;
                        this.open_dropdown = None;
                        cx.notify();
                    }),
                )
                .into_any_element(),
            ],
        ));

        // Toast section
        let toast_click_options = vec![
            "Markup".to_string(),
            "Copy".to_string(),
            "Open folder".to_string(),
            "Nothing".to_string(),
        ];
        let toast_click_current = match self.cfg.toast_click_action {
            ToastClickAction::Markup => "Markup".to_string(),
            ToastClickAction::Copy => "Copy".to_string(),
            ToastClickAction::OpenFolder => "Open folder".to_string(),
            ToastClickAction::None => "Nothing".to_string(),
        };
        let toast_click_selected = match self.cfg.toast_click_action {
            ToastClickAction::Markup => Some(0),
            ToastClickAction::Copy => Some(1),
            ToastClickAction::OpenFolder => Some(2),
            ToastClickAction::None => Some(3),
        };

        let toast_duration_options = vec![
            "2s".to_string(),
            "3s".to_string(),
            "5s".to_string(),
            "8s".to_string(),
            "10s".to_string(),
        ];
        let toast_duration_current = match self.cfg.toast_duration_ms {
            2000 => "2s".to_string(),
            3000 => "3s".to_string(),
            5000 => "5s".to_string(),
            8000 => "8s".to_string(),
            10000 => "10s".to_string(),
            ms if ms % 1000 == 0 => format!("{}s", ms / 1000),
            ms => format!("{}ms", ms),
        };
        let toast_duration_selected = match self.cfg.toast_duration_ms {
            2000 => Some(0),
            3000 => Some(1),
            5000 => Some(2),
            8000 => Some(3),
            10000 => Some(4),
            _ => None,
        };

        let toast_pos_options = vec![
            "Bottom right".to_string(),
            "Bottom left".to_string(),
            "Top right".to_string(),
            "Top left".to_string(),
        ];
        let toast_pos_current = match self.cfg.toast_position {
            ToastPosition::BottomRight => "Bottom right".to_string(),
            ToastPosition::BottomLeft => "Bottom left".to_string(),
            ToastPosition::TopRight => "Top right".to_string(),
            ToastPosition::TopLeft => "Top left".to_string(),
        };
        let toast_pos_selected = match self.cfg.toast_position {
            ToastPosition::BottomRight => Some(0),
            ToastPosition::BottomLeft => Some(1),
            ToastPosition::TopRight => Some(2),
            ToastPosition::TopLeft => Some(3),
        };

        form = form.child(self.section(
            "Toast",
            vec![
                self.dropdown_row(
                    "drop-toast-click",
                    "Click action",
                    DropdownField::ToastClickAction,
                    toast_click_current,
                    toast_click_options,
                    toast_click_selected,
                    cx,
                    |this, idx, _, _| {
                        this.cfg.toast_click_action = match idx {
                            0 => ToastClickAction::Markup,
                            1 => ToastClickAction::Copy,
                            2 => ToastClickAction::OpenFolder,
                            _ => ToastClickAction::None,
                        };
                    },
                )
                .into_any_element(),
                self.toggle_row(
                    "tog-toast-drag",
                    "Drag to export",
                    self.cfg.toast_drag_enabled,
                    cx.listener(|this, _, _, cx| {
                        this.cfg.toast_drag_enabled = !this.cfg.toast_drag_enabled;
                        this.open_dropdown = None;
                        cx.notify();
                    }),
                )
                .into_any_element(),
                self.toggle_row(
                    "tog-toast-actions",
                    "Show action buttons",
                    self.cfg.toast_show_actions,
                    cx.listener(|this, _, _, cx| {
                        this.cfg.toast_show_actions = !this.cfg.toast_show_actions;
                        this.open_dropdown = None;
                        cx.notify();
                    }),
                )
                .into_any_element(),
                self.dropdown_row(
                    "drop-toast-duration",
                    "Duration",
                    DropdownField::ToastDuration,
                    toast_duration_current,
                    toast_duration_options,
                    toast_duration_selected,
                    cx,
                    |this, idx, _, _| {
                        this.cfg.toast_duration_ms = match idx {
                            0 => 2000,
                            1 => 3000,
                            2 => 5000,
                            3 => 8000,
                            4 => 10000,
                            _ => 5000,
                        };
                    },
                )
                .into_any_element(),
                self.dropdown_row(
                    "drop-toast-pos",
                    "Position",
                    DropdownField::ToastPosition,
                    toast_pos_current,
                    toast_pos_options,
                    toast_pos_selected,
                    cx,
                    |this, idx, _, _| {
                        this.cfg.toast_position = match idx {
                            0 => ToastPosition::BottomRight,
                            1 => ToastPosition::BottomLeft,
                            2 => ToastPosition::TopRight,
                            _ => ToastPosition::TopLeft,
                        };
                    },
                )
                .into_any_element(),
            ],
        ));

        // Recording section
        let rec_fmt_options = vec![
            "MP4 (H.264)".to_string(),
            "GIF".to_string(),
            "WebM (VP9)".to_string(),
        ];
        let rec_fmt_current = match self.cfg.recording_format {
            iris_lib::config::RecordingFormat::Mp4 => "MP4 (H.264)".to_string(),
            iris_lib::config::RecordingFormat::Gif => "GIF".to_string(),
            iris_lib::config::RecordingFormat::Webm => "WebM (VP9)".to_string(),
        };
        let rec_fmt_selected = match self.cfg.recording_format {
            iris_lib::config::RecordingFormat::Mp4 => Some(0),
            iris_lib::config::RecordingFormat::Gif => Some(1),
            iris_lib::config::RecordingFormat::Webm => Some(2),
        };
        let rec_enc_options = vec![
            "Auto".to_string(),
            "Software (x264)".to_string(),
            "NVIDIA (NVENC)".to_string(),
        ];
        let rec_enc_current = match self.cfg.recording_encoder {
            iris_lib::config::RecordingEncoder::Auto => "Auto".to_string(),
            iris_lib::config::RecordingEncoder::Libx264 => "Software (x264)".to_string(),
            iris_lib::config::RecordingEncoder::Nvenc => "NVIDIA (NVENC)".to_string(),
        };
        let rec_enc_selected = match self.cfg.recording_encoder {
            iris_lib::config::RecordingEncoder::Auto => Some(0),
            iris_lib::config::RecordingEncoder::Libx264 => Some(1),
            iris_lib::config::RecordingEncoder::Nvenc => Some(2),
        };
        form = form.child(self.section(
            "Recording",
            vec![
                self.text_row("Recording fps", Field::Fps, cx).into_any_element(),
                self.dropdown_row(
                    "drop-rec-fmt",
                    "Format",
                    DropdownField::RecordingFormat,
                    rec_fmt_current,
                    rec_fmt_options,
                    rec_fmt_selected,
                    cx,
                    |this, idx, _, _| {
                        this.cfg.recording_format = match idx {
                            1 => iris_lib::config::RecordingFormat::Gif,
                            2 => iris_lib::config::RecordingFormat::Webm,
                            _ => iris_lib::config::RecordingFormat::Mp4,
                        };
                    },
                )
                .into_any_element(),
                self.dropdown_row(
                    "drop-rec-enc",
                    "MP4 encoder",
                    DropdownField::RecordingEncoder,
                    rec_enc_current,
                    rec_enc_options,
                    rec_enc_selected,
                    cx,
                    |this, idx, _, _| {
                        this.cfg.recording_encoder = match idx {
                            1 => iris_lib::config::RecordingEncoder::Libx264,
                            2 => iris_lib::config::RecordingEncoder::Nvenc,
                            _ => iris_lib::config::RecordingEncoder::Auto,
                        };
                    },
                )
                .into_any_element(),
                self.toggle_row(
                    "tog-mic",
                    "Record microphone by default",
                    self.cfg.record_mic_default,
                    cx.listener(|this, _, _, cx| {
                        this.cfg.record_mic_default = !this.cfg.record_mic_default;
                        this.open_dropdown = None;
                        cx.notify();
                    }),
                )
                .into_any_element(),
            ],
        ));

        // Keyboard section
        form = form.child(self.section(
            "Keyboard",
            vec![
                self.hotkey_row("Capture hotkey", Field::CaptureHotkey, cx)
                    .into_any_element(),
                self.hotkey_row("Record hotkey", Field::RecordHotkey, cx)
                    .into_any_element(),
                self.hotkey_row("Cancel keybind", Field::CancelKeybind, cx)
                    .into_any_element(),
                self.hotkey_row("Confirm keybind", Field::ConfirmKeybind, cx)
                    .into_any_element(),
            ],
        ));

        form = form.child(
            div()
                .flex()
                .justify_start()
                .pt(px(4.))
                .child(
                    crate::widgets::button("reset-defaults", "Reset to defaults", false)
                        .on_click(cx.listener(|this, _, _, cx| this.reset_to_defaults(cx))),
                ),
        );

        let save = crate::widgets::button("save", "Save", true)
            .on_click(cx.listener(|this, _, _, cx| this.save(cx)))
            .into_any_element();
        let content = div().id("settings-scroll").flex_1().overflow_y_scroll().child(form);
        let mut root = div()
            .size_full()
            .font_family(theme::FONT);
        if let Some(focus) = &self.focus {
            root = root.track_focus(focus);
        }
        let mut root = root
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.on_key(ev, window, cx)
            }))
            .child(crate::widgets::window_frame("Settings", self.class.clone(), vec![save], content));

        if let Some(status) = &self.status {
            root = root.child(crate::widgets::status_pill(status, 10.0));
        }
        root
    }
}

impl Settings {
    /// A macOS System Settings group: small semibold title over a
    /// rounded card whose rows are split by inset hairlines.
    fn section(&self, title: &'static str, rows: Vec<AnyElement>) -> Div {
        let mut card = div()
            .flex()
            .flex_col()
            .rounded(px(theme::RADIUS_GROUP))
            .bg(theme::GROUP_BG);
        let n = rows.len();
        for (i, row) in rows.into_iter().enumerate() {
            card = card.child(row);
            if i + 1 < n {
                card = card.child(div().h(px(1.)).ml(px(12.)).bg(theme::SEPARATOR));
            }
        }
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .ml(px(4.))
                    .mb(px(6.))
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::FG_DIM)
                    .child(title),
            )
            .child(card)
    }

    fn row_shell(&self, label: &'static str, control: impl IntoElement) -> Div {
        div()
            .h(px(ROW_H))
            .px(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_sm().text_color(theme::FG).child(label))
            .child(control)
    }

    fn text_row(
        &self,
        label: &'static str,
        field: Field,
        cx: &mut Context<Self>,
    ) -> Div {
        let (text, active) = match &self.editing {
            Some((f, buffer)) if *f == field => (format!("{buffer}▏"), true),
            _ => (self.field_text(field), false),
        };
        let box_el = crate::widgets::text_field(format!("field-{label}"), text, active)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.editing = Some((field, this.field_text(field)));
                this.recording = None;
                this.open_dropdown = None;
                cx.notify();
            }));
        self.row_shell(label, div().w(px(FIELD_W)).flex().child(box_el))
    }

    fn hotkey_row(
        &self,
        label: &'static str,
        field: Field,
        cx: &mut Context<Self>,
    ) -> Div {
        let recording = self.recording == Some(field);
        let text = if recording {
            "press keys…".to_string()
        } else {
            self.field_text(field)
        };
        let box_el = crate::widgets::text_field(format!("hotkey-{label}"), text, recording)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.recording = Some(field);
                this.editing = None;
                this.open_dropdown = None;
                cx.notify();
            }));
        self.row_shell(label, div().w(px(FIELD_W)).flex().child(box_el))
    }

    fn toggle_row(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Div {
        self.row_shell(label, crate::widgets::toggle(id, on).on_click(on_click))
    }

    fn dropdown_row<F>(
        &self,
        id: &'static str,
        label: &'static str,
        field: DropdownField,
        current: String,
        options: Vec<String>,
        selected: Option<usize>,
        cx: &mut Context<Self>,
        on_select: F,
    ) -> Div
    where
        F: Fn(&mut Self, usize, &mut Window, &mut Context<Self>) + 'static + Clone,
    {
        let is_open = self.open_dropdown == Some(field);
        let btn = crate::widgets::dropdown(id.to_string(), current, is_open)
            .w_full()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_dropdown = if this.open_dropdown == Some(field) {
                    None
                } else {
                    Some(field)
                };
                this.editing = None;
                this.recording = None;
                cx.notify();
            }));

        let mut control = div().w(px(FIELD_W)).relative().child(btn);

        if is_open {
            let mut menu = crate::widgets::menu()
                .absolute()
                .top(px(32.))
                .right_0()
                .w(px(FIELD_W));
            for (i, opt) in options.into_iter().enumerate() {
                let mark = if selected == Some(i) { "✓ " } else { "   " };
                let on_select = on_select.clone();
                let row = crate::widgets::menu_row_owned(
                    format!("{id}-opt-{i}"),
                    format!("{mark}{opt}"),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    on_select(this, i, window, cx);
                    this.open_dropdown = None;
                    cx.notify();
                }));
                menu = menu.child(row);
            }
            control = control.child(menu);
        }

        self.row_shell(label, control)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;
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
        let make_event = |key: &str, ctrl: bool, alt: bool, shift: bool, super_key: bool| KeyDownEvent {
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
        assert_eq!(Settings::format_keystroke(&make_event("control", true, false, false, false)), None);
        assert_eq!(Settings::format_keystroke(&make_event("shift", false, false, true, false)), None);

        // Named keys
        assert_eq!(Settings::format_keystroke(&make_event("escape", false, false, false, false)), Some("Escape".into()));
        assert_eq!(Settings::format_keystroke(&make_event("enter", false, false, false, false)), Some("Enter".into()));
        assert_eq!(Settings::format_keystroke(&make_event("print", false, false, false, false)), Some("Print".into()));
        assert_eq!(Settings::format_keystroke(&make_event(" ", false, false, false, false)), Some("Space".into()));

        // Single characters uppercase
        assert_eq!(Settings::format_keystroke(&make_event("a", false, false, false, false)), Some("A".into()));

        // Function keys capitalized
        assert_eq!(Settings::format_keystroke(&make_event("f9", false, false, false, false)), Some("F9".into()));

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
        std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
        std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));

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
        assert_eq!(loaded.recording_format, iris_lib::config::RecordingFormat::Webm);
        assert_eq!(loaded.recording_encoder, iris_lib::config::RecordingEncoder::Nvenc);
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
        assert_eq!(s.status, Some("Defaults restored — Save to apply".to_string()));
    }
}
