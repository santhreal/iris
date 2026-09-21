use std::path::PathBuf;

use gpui::*;
use iris_lib::config::Config;

use crate::theme;

use super::{DropdownField, Field, Settings};

const ROW_H: f32 = 38.0;
const FIELD_W: f32 = 280.0;

impl Settings {
    pub(super) fn field_text(&self, field: Field) -> String {
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
    pub(super) fn commit_edit(&mut self) -> Result<(), String> {
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
                let fps: u32 = text
                    .parse()
                    .map_err(|_| "fps must be a number".to_string())?;
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

    pub(super) fn save(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn reset_defaults_state(&mut self) {
        self.cfg = Config::default();
        self.editing = None;
        self.recording = None;
        self.open_dropdown = None;
        self.status = Some("Defaults restored; Save to apply".to_string());
    }

    pub(super) fn reset_to_defaults(&mut self, cx: &mut Context<Self>) {
        self.reset_defaults_state();
        cx.notify();
    }

    /// Check GitHub for a newer release on a background thread and
    /// report the result in the status pill. The network call never
    /// touches the UI loop.
    pub(super) fn check_updates(&mut self, cx: &mut Context<Self>) {
        self.status = Some("checking…".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::update::check() })
                .await;
            let msg = match result {
                Ok(Some(info)) => format!("update available: {}", info.version),
                Ok(None) => "up to date".to_string(),
                Err(e) => e,
            };
            this.update(cx, |this, cx| {
                this.status = Some(msg);
                cx.notify();
            })
            .ok();
        })
        .detach();
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

    pub(super) fn on_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        if self.open_dropdown.is_some() && ev.keystroke.key == "escape" {
            self.open_dropdown = None;
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

    /// A macOS System Settings group: small semibold title over a
    /// rounded card whose rows are split by inset hairlines.
    pub(super) fn section(&self, title: &'static str, rows: Vec<AnyElement>) -> Div {
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

    pub(super) fn row_shell(&self, label: &'static str, control: impl IntoElement) -> Div {
        div()
            .h(px(ROW_H))
            .px(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_sm().text_color(theme::FG).child(label))
            .child(control)
    }

    pub(super) fn text_row(
        &self,
        label: &'static str,
        field: Field,
        cx: &mut Context<Self>,
    ) -> Div {
        let (text, active) = match &self.editing {
            Some((f, buffer)) if *f == field => (format!("{buffer}▏"), true),
            _ => (self.field_text(field), false),
        };
        let box_el = crate::widgets::text_field(ElementId::Name(label.into()), text, active)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.editing = Some((field, this.field_text(field)));
                this.recording = None;
                this.open_dropdown = None;
                cx.notify();
            }));
        self.row_shell(label, div().w(px(FIELD_W)).flex().child(box_el))
    }

    pub(super) fn hotkey_row(
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
        let box_el = crate::widgets::text_field(ElementId::Name(label.into()), text, recording)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.recording = Some(field);
                this.editing = None;
                this.open_dropdown = None;
                cx.notify();
            }));
        self.row_shell(label, div().w(px(FIELD_W)).flex().child(box_el))
    }

    pub(super) fn toggle_row(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Div {
        self.row_shell(label, crate::widgets::toggle(id, on).on_click(on_click))
    }

    pub(super) fn dropdown_row<F>(
        &self,
        id: &'static str,
        label: &'static str,
        field: DropdownField,
        current: impl Into<SharedString>,
        options: &'static [&'static str],
        selected: Option<usize>,
        cx: &mut Context<Self>,
        on_select: F,
    ) -> Div
    where
        F: Fn(&mut Self, usize, &mut Window, &mut Context<Self>) + 'static + Clone,
    {
        let is_open = self.open_dropdown == Some(field);
        let btn = crate::widgets::dropdown(ElementId::Name(id.into()), current, is_open)
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
            for (i, opt) in options.iter().enumerate() {
                let mark = if selected == Some(i) { "✓ " } else { "   " };
                let on_select = on_select.clone();
                let row = crate::widgets::menu_row_owned(
                    ElementId::NamedInteger(id.into(), i as u64),
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
