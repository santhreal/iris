//! Settings window: panes over config.toml in the macOS Settings
//! register.
//!
//! The unified toolbar holds the pane tabs (General, Capture,
//! Recording, Shortcuts, Updates), and the window takes the height of
//! the selected pane. Every change applies at once: a toggle or a
//! pop-up choice writes config.toml on the click, a text field on
//! Return or when it loses focus, a shortcut field as soon as the
//! combination is pressed, and the global hotkey grabs reload after a
//! shortcut changes. A rejected value keeps its field open with the
//! reason beside it; Esc restores the stored value.

use gpui::*;
use iris_lib::config::Config;

use crate::{theme, widgets};

mod app;
mod fields;
mod panes;
use app::Release;
use panes::{Group, Pane, Row};
#[cfg(test)]
mod tests;

/// A setting edited through a text field or recorded from a key press.
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

impl Field {
    #[cfg(test)]
    pub(super) const ALL: [Field; 8] = [
        Field::ScreenshotsDir,
        Field::RecordingsDir,
        Field::Template,
        Field::Fps,
        Field::CaptureHotkey,
        Field::RecordHotkey,
        Field::CancelKeybind,
        Field::ConfirmKeybind,
    ];

    pub(super) fn id(self) -> &'static str {
        match self {
            Field::ScreenshotsDir => "field-shots-dir",
            Field::RecordingsDir => "field-recs-dir",
            Field::Template => "field-template",
            Field::Fps => "field-fps",
            Field::CaptureHotkey => "field-capture-hotkey",
            Field::RecordHotkey => "field-record-hotkey",
            Field::CancelKeybind => "field-cancel-key",
            Field::ConfirmKeybind => "field-confirm-key",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Field::ScreenshotsDir => "Screenshots",
            Field::RecordingsDir => "Recordings",
            Field::Template => "File name",
            Field::Fps => "Frames per second",
            Field::CaptureHotkey => "New screenshot",
            Field::RecordHotkey => "Record window",
            Field::CancelKeybind => "Cancel",
            Field::ConfirmKeybind => "Confirm selection",
        }
    }
}

/// A text field's edit in progress: the field and the typed text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Edit {
    pub(super) field: Field,
    pub(super) text: String,
}

pub struct Settings {
    pub(super) cfg: Config,
    pub(super) pane: Pane,
    pub(super) editing: Option<Edit>,
    /// The shortcut field waiting for a key combination.
    pub(super) recording: Option<Field>,
    /// Why the value last entered into this field was rejected, shown
    /// beside it until the field closes or takes a valid value.
    pub(super) rejected: Option<(Field, String)>,
    pub(super) open_pick: Option<panes::Pick>,
    /// A failure to apply a setting (config.toml, the login entry, the
    /// folder picker), shown in the status pill until the next change
    /// applies.
    pub(super) status: Option<String>,
    pub(super) release: Release,
    /// The update row's state line after a check or an install started
    /// in this window. None shows when the last check ran.
    pub(super) update_note: Option<String>,
    pub(super) last_checked: Option<std::time::SystemTime>,
    /// Start at login: `sys::autostart::enabled` when the window
    /// opened, then the toggle, which writes it.
    pub(super) login: bool,
    pub(super) focus: Option<FocusHandle>,
    pub(super) scroll: ScrollHandle,
    /// An update check started in this window is running.
    pub(super) checking: bool,
}

impl Settings {
    fn new(cfg: Config, login: bool, release: Release, focus: Option<FocusHandle>) -> Self {
        Self {
            cfg,
            pane: Pane::General,
            editing: None,
            recording: None,
            rejected: None,
            open_pick: None,
            status: None,
            release,
            update_note: None,
            last_checked: None,
            login,
            focus,
            scroll: ScrollHandle::new(),
            checking: false,
        }
    }

    /// Show `pane` and fit the window to it. An edit that does not
    /// commit keeps the current pane, so the rejected value stays in
    /// view.
    pub(super) fn select_pane(&mut self, pane: Pane, window: &mut Window) -> bool {
        if !self.blur() {
            return false;
        }
        self.open_pick = None;
        if pane != self.pane {
            self.pane = pane;
            self.scroll = ScrollHandle::new();
            window.resize(size(px(panes::WIDTH), px(pane.height())));
        }
        true
    }
}

/// The settings window's minimum size: the shortest pane. Window
/// managers hold a window at its minimum size hint, so the hint must
/// let every pane's height through.
fn min_size() -> Size<Pixels> {
    let h = Pane::ALL
        .iter()
        .map(|p| p.height())
        .fold(f32::INFINITY, f32::min);
    size(px(panes::WIDTH), px(h))
}

/// Open the settings window, or raise the one already open: two
/// windows would each write their own snapshot of the config over the
/// other's changes.
pub fn open(cx: &mut App) -> Result<(), String> {
    if crate::widgets::raise_open::<Settings>(cx, |_| true) {
        return Ok(());
    }
    let focus = cx.focus_handle();
    let h = Pane::General.height();
    let origin = crate::sys::window::centered_origin(cx, panes::WIDTH, h, (220.0, 140.0));
    crate::widgets::open_window(
        cx,
        "Settings - iris",
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(origin.0), px(origin.1)),
                size: size(px(panes::WIDTH), px(h)),
            })),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_movable: true,
            is_resizable: false,
            is_minimizable: true,
            display_id: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_min_size: Some(min_size()),
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        },
        |window, cx| {
            focus.focus(window, cx);
            cx.new(|cx| {
                // An edit open when the window loses focus commits, as
                // a field does when focus moves within the window.
                cx.observe_window_activation(window, |this: &mut Settings, window, cx| {
                    if !window.is_window_active() && (this.editing.is_some() || this.recording.is_some()) {
                        this.blur();
                        cx.notify();
                    }
                })
                .detach();
                Settings::new(
                    Config::load(),
                    crate::sys::autostart::enabled(),
                    app::offered_release(),
                    Some(focus),
                )
                .with_last_checked()
            })
        },
    )
    .map_err(|e| format!("open settings window: {e}"))?;
    Ok(())
}

impl Settings {
    /// One group: an optional caption header over a card whose rows
    /// are split by hairlines inset to the labels' leading edge.
    fn group(&self, group: &Group, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut card = div()
            .flex()
            .flex_col()
            .rounded(px(theme::RADIUS_GROUP))
            .bg(theme::GROUP_BG);
        for (i, &row) in group.rows.iter().enumerate() {
            if i > 0 {
                card = card.child(
                    div()
                        .h(px(1.))
                        .ml(px(theme::ROW_PAD_X))
                        .bg(theme::SEPARATOR),
                );
            }
            card = card.child(self.row(row, window, cx));
        }
        div()
            .flex()
            .flex_col()
            .children(group.header.map(|title| {
                div()
                    .ml(px(theme::ROW_PAD_X))
                    .mb(px(theme::HEADER_GAP))
                    .text_size(px(theme::TEXT_SMALL))
                    .line_height(px(panes::HEADER_H - theme::HEADER_GAP))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::FG_DIM)
                    .child(title)
            }))
            .child(card)
    }

    fn row(&self, row: Row, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match row {
            Row::Switch(s) => self.switch_row(s, cx).into_any_element(),
            Row::Pick(p) => self.pick_row(p, cx).into_any_element(),
            Row::Text(f) => self.text_row(f, cx).into_any_element(),
            Row::Folder(f) => self.folder_row(f, cx).into_any_element(),
            Row::Hotkey(f) => self.hotkey_row(f, cx).into_any_element(),
            Row::Restore => self.restore_row(cx).into_any_element(),
            Row::Version => self.version_row(window, cx).into_any_element(),
        }
    }
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut form = div()
            .flex()
            .flex_col()
            .gap(px(theme::GROUP_GAP))
            .p(px(theme::INSET));
        for group in self.pane.groups() {
            form = form.child(self.group(group, window, cx));
        }
        // A press on the window outside every control ends the open
        // edit, as a click away from a text field does on macOS.
        let content = widgets::scroll_y(div().id("settings-scroll"), &self.scroll)
            .flex_1()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.editing.is_some() || this.recording.is_some() {
                        this.blur();
                        cx.notify();
                    }
                }),
            )
            .child(form);

        let this = cx.entity().downgrade();
        let tabs = widgets::segmented(
            "settings-pane",
            widgets::SegmentStyle::Tabs,
            Pane::ALL
                .iter()
                .map(|p| widgets::Segment::icon(p.icon()).with_label(p.label()))
                .collect(),
            Some(self.pane.index()),
            move |ix, window, cx| {
                this.update(cx, |this, cx| {
                    this.select_pane(Pane::ALL[ix], window);
                    cx.notify();
                })
                .ok();
            },
        );
        let mut root = div().size_full().font_family(theme::FONT);
        if let Some(focus) = &self.focus {
            root = root.track_focus(focus);
        }
        let mut root = root
            .on_key_down(cx.listener(Self::on_key))
            .child(widgets::toolbar_frame(
                window,
                widgets::Toolbar::default().center(tabs),
                false,
                content,
            ));
        if let Some(status) = &self.status {
            root = root.child(widgets::status_pill(status, 10.0));
        }
        root
    }
}
