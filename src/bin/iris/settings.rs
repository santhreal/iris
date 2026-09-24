//! Settings window: one landscape form over config.toml.
//!
//! Every field edits the loaded Config; Save validates and persists
//! with Config::store. Text fields are click-to-edit (type, Enter
//! commits, Esc cancels), hotkey fields are press-to-record: click the
//! field, press the combination, it is captured verbatim. Dropdowns
//! toggle a floating option list and write the selected value on click.

use gpui::*;
use iris_lib::config::{Config, ToastClickAction, ToastPosition};

use crate::theme;

mod fields;
use fields::Choice;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Field {
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
pub(super) enum DropdownField {
    ToastClickAction,
    ToastDuration,
    ToastPosition,
    RecordingFormat,
    RecordingEncoder,
}

pub struct Settings {
    pub(super) cfg: Config,
    pub(super) editing: Option<(Field, String)>,
    pub(super) recording: Option<Field>,
    pub(super) open_dropdown: Option<DropdownField>,
    pub(super) status: Option<String>,
    pub(super) focus: Option<FocusHandle>,
}

/// The settings window's minimum logical size, where its resize stops.
const MIN_SIZE: Size<Pixels> = size(px(640.), px(600.));

/// Open the settings window, or raise the one already open: two
/// windows would each save their own snapshot of the config over the
/// other's edits.
pub fn open(cx: &mut App) -> Result<(), String> {
    if crate::widgets::raise_open::<Settings>(cx, |_| true) {
        return Ok(());
    }
    let focus = Some(cx.focus_handle());
    let win = (680.0f32, 650.0f32);
    let origin = crate::sys::window::centered_origin(cx, win.0, win.1, (220.0, 140.0));
    crate::widgets::open_window(
        cx,
        "Settings - iris",
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
            window_min_size: Some(MIN_SIZE),
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        },
        |_, cx| {
            cx.new(|_| Settings {
                cfg: Config::load(),
                editing: None,
                recording: None,
                open_dropdown: None,
                status: None,
                focus,
            })
        },
    )
    .map_err(|e| format!("open settings window: {e}"))?;
    Ok(())
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
        const TOAST_CLICK_OPTIONS: &[&str] = &["Markup", "Copy", "Open folder", "Nothing"];
        let toast_click_options = TOAST_CLICK_OPTIONS;
        // &'static str: the dropdown stores the label without a
        // per-render String allocation.
        let toast_click_current: &'static str = match self.cfg.toast_click_action {
            ToastClickAction::Markup => "Markup",
            ToastClickAction::Copy => "Copy",
            ToastClickAction::OpenFolder => "Open folder",
            ToastClickAction::None => "Nothing",
        };
        let toast_click_selected = match self.cfg.toast_click_action {
            ToastClickAction::Markup => Some(0),
            ToastClickAction::Copy => Some(1),
            ToastClickAction::OpenFolder => Some(2),
            ToastClickAction::None => Some(3),
        };

        const TOAST_DURATION_OPTIONS: &[&str] = &["2s", "3s", "5s", "8s", "10s"];
        let toast_duration_options = TOAST_DURATION_OPTIONS;
        let toast_duration_current = match self.cfg.toast_duration_ms {
            2000 => "2s".to_string(),
            3000 => "3s".to_string(),
            5000 => "5s".to_string(),
            8000 => "8s".to_string(),
            10000 => "10s".to_string(),
            ms if ms.is_multiple_of(1000) => format!("{}s", ms / 1000),
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

        const TOAST_POS_OPTIONS: &[&str] =
            &["Bottom right", "Bottom left", "Top right", "Top left"];
        let toast_pos_options = TOAST_POS_OPTIONS;
        let toast_pos_current: &'static str = match self.cfg.toast_position {
            ToastPosition::BottomRight => "Bottom right",
            ToastPosition::BottomLeft => "Bottom left",
            ToastPosition::TopRight => "Top right",
            ToastPosition::TopLeft => "Top left",
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
                    Choice::new(toast_click_current, toast_click_options, toast_click_selected),
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
                    Choice::new(
                        toast_duration_current,
                        toast_duration_options,
                        toast_duration_selected,
                    ),
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
                    Choice::new(toast_pos_current, toast_pos_options, toast_pos_selected),
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
        const REC_FMT_OPTIONS: &[&str] = &["MP4 (H.264)", "GIF", "WebM (VP9)"];
        let rec_fmt_options = REC_FMT_OPTIONS;
        let rec_fmt_current: &'static str = match self.cfg.recording_format {
            iris_lib::config::RecordingFormat::Mp4 => "MP4 (H.264)",
            iris_lib::config::RecordingFormat::Gif => "GIF",
            iris_lib::config::RecordingFormat::Webm => "WebM (VP9)",
        };
        let rec_fmt_selected = match self.cfg.recording_format {
            iris_lib::config::RecordingFormat::Mp4 => Some(0),
            iris_lib::config::RecordingFormat::Gif => Some(1),
            iris_lib::config::RecordingFormat::Webm => Some(2),
        };
        const REC_ENC_OPTIONS: &[&str] = &["Auto", "Software (x264)", "NVIDIA (NVENC)"];
        let rec_enc_options = REC_ENC_OPTIONS;
        let rec_enc_current: &'static str = match self.cfg.recording_encoder {
            iris_lib::config::RecordingEncoder::Auto => "Auto",
            iris_lib::config::RecordingEncoder::Libx264 => "Software (x264)",
            iris_lib::config::RecordingEncoder::Nvenc => "NVIDIA (NVENC)",
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
                    Choice::new(rec_fmt_current, rec_fmt_options, rec_fmt_selected),
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
                    Choice::new(rec_enc_current, rec_enc_options, rec_enc_selected),
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

        // Updates section: current version + a manual check. The check
        // runs off the UI loop and reports through the status pill.
        let check_btn = crate::widgets::button("check-update", "Check for updates", false)
            .on_click(cx.listener(|this, _, _, cx| this.check_updates(cx)));
        form = form.child(self.section(
            "Updates",
            vec![
                self.row_shell(
                    "Version",
                    div()
                        .text_size(px(theme::TEXT_BODY))
                        .text_color(theme::FG_DIM)
                        .child(env!("CARGO_PKG_VERSION").to_string()),
                )
                .into_any_element(),
                self.row_shell("Latest release", check_btn).into_any_element(),
            ],
        ));

        form = form.child(
            div().flex().justify_start().pt(px(4.)).child(
                crate::widgets::button("reset-defaults", "Reset to defaults", false)
                    .on_click(cx.listener(|this, _, _, cx| this.reset_to_defaults(cx))),
            ),
        );

        let save = crate::widgets::button("save", "Save", true)
            .on_click(cx.listener(|this, _, _, cx| this.save(cx)))
            .into_any_element();
        let content = div()
            .id("settings-scroll")
            .flex_1()
            .overflow_y_scroll()
            .child(form);
        let mut root = div().size_full().font_family(theme::FONT);
        if let Some(focus) = &self.focus {
            root = root.track_focus(focus);
        }
        let mut root = root
            .on_key_down(
                cx.listener(|this, ev: &KeyDownEvent, window, cx| this.on_key(ev, window, cx)),
            )
            .child(crate::widgets::window_frame(
                "Settings",
                true,
                vec![save],
                content,
            ));

        if let Some(status) = &self.status {
            root = root.child(crate::widgets::status_pill(status, 10.0));
        }
        root.children(crate::widgets::resize_edges(window, MIN_SIZE))
    }
}
