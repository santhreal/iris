//! Settings: panes over config.toml, the app window's settings page.
//!
//! The window's sidebar lists the panes (General, Capture, Recording,
//! Shortcuts, Updates) and the page shows the selected one, scrolling
//! when it is taller than the window. Every change applies at once: a
//! toggle or a pop-up choice writes config.toml on the click, a text
//! field on Return or when it loses focus, a shortcut field as soon as
//! the combination is pressed, and the global hotkey grabs reload after
//! a shortcut changes. A rejected value keeps its field open with the
//! reason beside it; Esc restores the stored value. Tab and Shift+Tab
//! move a focus ring over the pane's controls, Space or Return runs the
//! control under it, Ctrl+Tab and Ctrl+Shift+Tab switch panes, and Esc
//! with nothing open returns to the library.

use gpui::*;
use iris_lib::config::Config;

use crate::{theme, widgets};

mod app;
mod fields;
mod panes;
use app::Release;
pub(crate) use panes::Pane;
use panes::{Group, Row};
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
            Field::RecordHotkey => crate::sys::record::START_NAME,
            Field::CancelKeybind => "Cancel",
            Field::ConfirmKeybind => "Confirm selection",
        }
    }
}

/// A text field's edit in progress: the field and the typed text with
/// its caret.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Edit {
    pub(super) field: Field,
    pub(super) line: crate::widgets::Line,
}

pub struct Settings {
    pub(super) cfg: Config,
    pub(super) pane: Pane,
    pub(super) editing: Option<Edit>,
    /// How far left the open text field moves its text to keep the
    /// caret in view, as of the last frame ([`widgets::field_scroll`]).
    pub(super) field_scroll: std::cell::Cell<f32>,
    /// Where the open text field's text starts, recorded as it is
    /// drawn, for a drag that selects in it.
    pub(super) edit_origin: widgets::TextOrigin,
    /// A press on the open text field that a drag extends into a
    /// selection, until the first button is released.
    pub(super) field_drag: bool,
    /// The shortcut field waiting for a key combination.
    pub(super) recording: Option<Field>,
    /// Why the value last entered into this field was rejected, shown
    /// beside it until the field closes or takes a valid value.
    pub(super) rejected: Option<(Field, String)>,
    pub(super) open_pick: Option<panes::Pick>,
    /// The highlighted option of the open pop-up menu: the selected one
    /// when it opens, then the one the pointer or an arrow key moves to.
    pub(super) menu_lit: Option<usize>,
    /// The control the keyboard focus ring is on, which Space or Return
    /// runs. Tab sets it; a pane change or a mouse press clears it.
    pub(super) ring: Option<Row>,
    /// A failure to apply a setting (config.toml, the login entry, the
    /// folder picker), shown in the status pill until the next change
    /// applies.
    pub(super) status: Option<String>,
    /// Advice for the edit open on its field, shown in the status pill
    /// while that edit stays open: a folder picker that did not open
    /// leaves its path open for typing.
    pub(super) edit_note: Option<(Field, String)>,
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
            field_scroll: std::cell::Cell::new(0.0),
            edit_origin: widgets::TextOrigin::default(),
            field_drag: false,
            recording: None,
            rejected: None,
            open_pick: None,
            menu_lit: None,
            ring: None,
            status: None,
            edit_note: None,
            release,
            update_note: None,
            last_checked: None,
            login,
            focus,
            scroll: ScrollHandle::new(),
            checking: false,
        }
    }

    /// Show `pane`. An edit that does not commit keeps the current
    /// pane, so the rejected value stays in view.
    pub(crate) fn select_pane(&mut self, pane: Pane) -> bool {
        if !self.blur() {
            return false;
        }
        self.open_pick = None;
        if pane != self.pane {
            self.pane = pane;
            self.ring = None;
            self.scroll = ScrollHandle::new();
        }
        true
    }

    /// The pane shown.
    pub(crate) fn pane(&self) -> Pane {
        self.pane
    }

    /// The status pill's text: a failure to apply a setting, else the
    /// open edit's note.
    pub(super) fn pill(&self) -> Option<&str> {
        self.status.as_deref().or_else(|| {
            let (field, note) = self.edit_note.as_ref()?;
            let open = self.editing.as_ref().is_some_and(|e| e.field == *field);
            open.then_some(note.as_str())
        })
    }

    /// The config as last written: the library reads its shortcuts
    /// from it when the window returns to it.
    pub(crate) fn config(&self) -> &Config {
        &self.cfg
    }

    /// Move the keyboard into the page.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(focus) = &self.focus {
            focus.focus(window, cx);
        }
    }
}

/// The settings page of the app window, created the first time the
/// window shows it. The window holds one: two pages would each write
/// their own snapshot of the config over the other's changes.
pub fn page(window: &mut Window, cx: &mut App) -> Entity<Settings> {
    let focus = cx.focus_handle();
    cx.new(|cx| {
        // An edit open when the window loses focus commits, as a field
        // does when focus moves within the page.
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
}

impl Settings {
    /// One group: an optional caption header over a card whose rows
    /// are split by hairlines inset to the rows' padding on both sides.
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
                        .mx(px(theme::ROW_PAD_X))
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
        let el = match row {
            Row::Switch(s) => self.switch_row(s, cx).into_any_element(),
            Row::Pick(p) => self.pick_row(p, cx).into_any_element(),
            Row::Text(f) => self.text_row(f, window, cx).into_any_element(),
            Row::Folder(f) => self.folder_row(f, window, cx).into_any_element(),
            Row::Hotkey(f) => self.hotkey_row(f, cx).into_any_element(),
            Row::Restore => self.restore_row(cx).into_any_element(),
            Row::Version => self.version_row(window, cx).into_any_element(),
        };
        if self.enabled(row) {
            return el;
        }
        // A disabled row draws dimmed, and its control neither hovers
        // nor presses.
        widgets::disabled(el).into_any_element()
    }
}

/// The leading edge of the pane's groups in the page beside the
/// sidebar, logical px: the form centered at its widest
/// (`widgets::page_lead`). The toolbar's title starts there.
pub(crate) fn lead(window: &Window) -> f32 {
    let page = f32::from(window.client_area().size.width) - theme::SIDEBAR_W;
    widgets::page_lead(page, panes::FORM_MAX_W - 2.0 * theme::INSET)
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut form = div()
            .flex()
            .flex_col()
            .gap(px(theme::GROUP_GAP))
            .max_w(px(panes::FORM_MAX_W))
            .p(px(theme::INSET))
            .ml(px(lead(window) - theme::INSET));
        for group in self.pane.groups() {
            form = form.child(self.group(group, window, cx));
        }
        // A press on the page outside every control ends the open edit,
        // as a click away from a text field does on macOS.
        let scrolled = -f32::from(self.scroll.offset().y) > 0.5;
        let content = widgets::scroll_y(div().id("settings-scroll"), &self.scroll)
            .flex_1()
            .min_h_0()
            // A hairline under the toolbar once the pane scrolls under
            // it, as the library's grid draws.
            .border_t_1()
            .border_color(if scrolled {
                theme::HAIRLINE
            } else {
                theme::alpha(theme::HAIRLINE, 0.0)
            })
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

        let mut root = div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col();
        if let Some(focus) = &self.focus {
            root = root.track_focus(focus);
        }
        let mut root = root
            .on_key_down(cx.listener(Self::on_key))
            // The pointer takes over from the keyboard: the ring goes
            // before the press reaches a control.
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                if this.ring.take().is_some() {
                    cx.notify();
                }
            }))
            // A drag from a press on the open text field selects, here
            // so that it follows the pointer outside the field.
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                if !this.field_drag {
                    return;
                }
                if ev.pressed_button == Some(MouseButton::Left)
                    && this.drag_field(ev.position, window)
                {
                    cx.notify();
                } else {
                    this.field_drag = false;
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.field_drag = false),
            )
            .child(content);
        if let Some(text) = self.pill() {
            root = root.child(widgets::status_pill(text.to_string(), 10.0));
        }
        root
    }
}
