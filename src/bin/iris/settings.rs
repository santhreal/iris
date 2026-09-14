//! Settings window: one landscape form over config.toml.
//!
//! Every field edits the loaded Config; Save validates and persists
//! with Config::store. Text fields are click-to-edit (type, Enter
//! commits, Esc cancels), hotkey fields are press-to-record: click the
//! field, press the combination, it is captured verbatim.

use std::path::PathBuf;

use iris_lib::config::Config;
use gpui::*;

use crate::theme;

const ROW_H: f32 = 38.0;
const FIELD_W: f32 = 280.0;

#[derive(Clone, Copy, PartialEq)]
enum Field {
    ScreenshotsDir,
    RecordingsDir,
    Template,
    Fps,
    CaptureHotkey,
    RecordHotkey,
}

pub struct Settings {
    cfg: Config,
    editing: Option<(Field, String)>,
    recording: Option<Field>,
    status: Option<String>,
    focus: FocusHandle,
    /// This window's unique WM_CLASS, for the title-bar drag.
    class: String,
}

/// Open the settings window. A second call focuses a new window; the
/// daemon milestone owns single-instance behavior for all windows.
pub fn open(cx: &mut App) -> Result<(), String> {
    let focus = cx.focus_handle();
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
            Field::CaptureHotkey | Field::RecordHotkey => {} // hotkeys are recorded, not typed
        }
        Ok(())
    }

    fn save(&mut self, cx: &mut Context<Self>) {
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

    /// Format a pressed keystroke the way config.toml stores hotkeys:
    /// "Ctrl+Shift+R", "Print".
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
                    _ => {}
                }
            }
            cx.notify();
            return;
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
        self.focus.focus(window);
        let mut form = div().flex().flex_col().gap(px(20.)).p(px(24.));

        form = form
            .child(self.section(
                "Storage",
                vec![
                    self.text_row("Screenshots folder", Field::ScreenshotsDir, cx)
                        .into_any_element(),
                    self.text_row("Recordings folder", Field::RecordingsDir, cx)
                        .into_any_element(),
                    self.text_row("Filename template", Field::Template, cx)
                        .into_any_element(),
                ],
            ))
            .child(self.section(
                "Recording",
                vec![
                    self.text_row("Recording fps", Field::Fps, cx).into_any_element(),
                    self.toggle_row(
                        "tog-mic",
                        "Record microphone by default",
                        self.cfg.record_mic_default,
                        cx.listener(|this, _, _, cx| {
                            this.cfg.record_mic_default = !this.cfg.record_mic_default;
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
                ],
            ))
            .child(self.section(
                "Capture",
                vec![
                    self.toggle_row(
                        "tog-flash",
                        "Flash on capture",
                        self.cfg.flash_on_capture,
                        cx.listener(|this, _, _, cx| {
                            this.cfg.flash_on_capture = !this.cfg.flash_on_capture;
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
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
                ],
            ))
            .child(self.section(
                "Keyboard",
                vec![
                    self.hotkey_row("Capture hotkey", Field::CaptureHotkey, cx)
                        .into_any_element(),
                    self.hotkey_row("Record hotkey", Field::RecordHotkey, cx)
                        .into_any_element(),
                ],
            ));

        let save = crate::widgets::button("save", "Save", true)
            .on_click(cx.listener(|this, _, _, cx| this.save(cx)))
            .into_any_element();
        let content = div().id("settings-scroll").flex_1().overflow_y_scroll().child(form);
        let mut root = div()
            .size_full()
            .font_family(theme::FONT)
            .track_focus(&self.focus)
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
}
