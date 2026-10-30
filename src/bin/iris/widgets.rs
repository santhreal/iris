//! Shared chrome widgets: one implementation per control, used by
//! every surface. The register is defined in theme.rs; glyphs in
//! icons.rs. Buttons are ghost by default (hover wash, no fill), as
//! Apple's chrome reads; the accent fill marks the single primary
//! action of a surface. Every icon-only control takes a `tip`.

mod controls;
mod field;
mod image;
mod keycaps;
mod menu;
mod resize;
mod scroll;
mod tooltip;
mod vanish;
mod window_frame;
mod window_move;
mod window_open;

/// The label for a local action chord. Its handlers accept Control or
/// the platform modifier; macOS presents that choice as Command.
macro_rules! meta_shortcut_for {
    ($macos:expr, $suffix:literal) => {
        if $macos {
            concat!("Cmd+", $suffix)
        } else {
            concat!("Ctrl+", $suffix)
        }
    };
}
pub(crate) use meta_shortcut_for;

macro_rules! meta_shortcut {
    ($suffix:literal) => {
        $crate::widgets::meta_shortcut_for!(cfg!(target_os = "macos"), $suffix)
    };
}
pub(crate) use meta_shortcut;

pub use self::controls::{push_button, segmented, toggle, ButtonStyle, Segment, SegmentStyle};
pub use self::field::{
    choose_folder, edit_line, field_scroll, folder_field, ring_step, ringed, text_field,
    FieldState, Line, LineKey, TextOrigin,
};
pub use self::image::*;
pub use self::keycaps::{chord_label, keycaps, keycaps_in};
pub use self::menu::{
    hover_row, menu, menu_at, menu_key, menu_separator, popup_button, popup_menu, MenuItem,
    MenuKey, MENU_PAD, MENU_RADIUS, MENU_ROW_H, MENU_W,
};
pub use self::resize::resize_edges;
pub use self::scroll::scroll_y;
pub use self::tooltip::tip;
pub use self::vanish::{clear_then_run, close_then_run};
pub use self::window_frame::{
    close_route, toolbar_bar, toolbar_frame, window_corner, window_edge, OnClose, Toolbar,
};
pub use self::window_move::{move_handle, Double};
pub use self::window_open::open_window;

use gpui::*;

use crate::{icons, theme};
use icons::Icon;

/// Toolbar text button, TOOLBAR_CONTROL_H tall. Ghost (hover wash)
/// unless `primary`, the one accent-filled action on a surface. Forms
/// use `push_button`.
pub fn button(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    controls::styled_button(
        div()
            .id(ElementId::Name(id.into()))
            .h(px(theme::TOOLBAR_CONTROL_H))
            .px(px(12.)),
        if primary {
            ButtonStyle::Primary
        } else {
            ButtonStyle::Ghost
        },
    )
    .child(label)
}

/// Toolbar text button with a trailing glyph (a menu's chevron) in the
/// label's color, hover included.
pub fn button_with_icon(
    id: &'static str,
    label: &'static str,
    glyph: Icon,
    primary: bool,
) -> Stateful<Div> {
    button(id, label, primary).child(icons::icon_in_text_color(glyph, 10.0))
}

/// The glyph size for an icon-only control of edge `size`: 16 in a
/// 28px toolbar control, 14 in a 24px form control.
pub fn glyph_for(size: f32) -> f32 {
    (size * 0.57).round()
}

/// Icon-only button, `size`px square (28 in toolbars, the minimum hit
/// area). `active` (a selected tool, a pressed toggle) fills it with
/// the accent and draws the glyph white; otherwise ghost with a hover
/// wash. Pair with `.tooltip(tip(label, shortcut))`.
pub fn icon_button(
    id: impl Into<ElementId>,
    glyph: Icon,
    active: bool,
    size: f32,
) -> Stateful<Div> {
    icon_button_inked(id, glyph, active, size, theme::FG_DIM)
}

/// [`icon_button`] for a HUD that floats over any screen content, the
/// toast's capsule and the recording chip: the glyph is primary text,
/// not secondary, so it reads over a busy image as a HUD control does.
pub fn hud_icon_button(
    id: impl Into<ElementId>,
    glyph: Icon,
    active: bool,
    size: f32,
) -> Stateful<Div> {
    icon_button_inked(id, glyph, active, size, theme::FG)
}

fn icon_button_inked(
    id: impl Into<ElementId>,
    glyph: Icon,
    active: bool,
    size: f32,
    ink: Rgba,
) -> Stateful<Div> {
    let el = div()
        .id(id)
        .w(px(size))
        .h(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_CONTROL))
        .cursor_pointer()
        .flex_shrink_0();
    let el = if active {
        el.bg(theme::ACCENT)
    } else {
        el.hover(|s| s.bg(theme::SURFACE_HOVER))
            .active(|s| s.bg(theme::SURFACE_PRESS))
    };
    el.child(icons::icon(
        glyph,
        if active { theme::ACCENT_INK } else { ink },
        glyph_for(size),
    ))
}

/// Small icon button for card overlays: 26px on a frosted chip.
pub fn overlay_icon_button(id: impl Into<ElementId>, glyph: Icon) -> Stateful<Div> {
    overlay_icon_button_active(id, glyph, false)
}

/// Small icon button for card overlays, with optional active/highlight state.
pub fn overlay_icon_button_active(
    id: impl Into<ElementId>,
    glyph: Icon,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(26.))
        .h(px(26.))
        .rounded(px(theme::RADIUS_CONTROL))
        .bg(if active {
            theme::ACCENT
        } else {
            theme::alpha(theme::BG_ELEV, 0.92)
        })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| {
            if active {
                s
            } else {
                s.bg(theme::alpha(theme::BG_ELEV, 1.0))
            }
        })
        .active(move |s| {
            if active {
                s
            } else {
                s.bg(theme::SURFACE_PRESS)
            }
        })
        .child(icons::icon(
            glyph,
            if active { theme::ACCENT_INK } else { theme::FG },
            theme::ICON_ROW,
        ))
}
/// How long a transient status message stays up.
pub const STATUS_FOR: std::time::Duration = std::time::Duration::from_secs(4);

/// Transient status line, `left` px from the window's left edge,
/// bottom-aligned, frosted.
pub fn status_pill(text: &str, left: f32) -> Div {
    div()
        .absolute()
        .bottom(px(10.))
        .left(px(left))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(theme::RADIUS_SM))
        .bg(theme::alpha(theme::BG_ELEV, 0.95))
        .shadow(theme::shadow_float())
        .text_size(px(theme::TEXT_SMALL))
        .text_color(theme::FG_DIM)
        .child(text.to_string())
}

/// Raise and focus the open window whose root view is a `V` that
/// passes `is`, and return true; return false when there is none, so
/// the caller opens one. A surface with one window per subject opens
/// through this, and a second open brings the first one forward.
pub fn raise_open<V: Render>(cx: &mut App, is: impl Fn(&V) -> bool) -> bool {
    let found = cx
        .windows()
        .into_iter()
        .filter_map(|w| w.downcast::<V>())
        .find(|w| w.read(cx).is_ok_and(&is));
    found.is_some_and(|w| {
        w.update(cx, |_, window, _| window.activate_window())
            .is_ok()
    })
}

/// The least space between the shortcuts sheet and the window edge.
const SHEET_MARGIN: f32 = 16.0;

/// A keyboard-shortcuts sheet: centered card over a dim layer, `lead`
/// under the title where a surface has one, then rows of action / key
/// combination. The surface owns the open state and closes on Esc or a
/// press on the dim layer. The dim layer takes the pointer from the
/// surface under it, and the card stays inside the window: rows that do
/// not fit scroll under the title on `scroll`, which the surface keeps
/// across renders.
pub fn shortcuts_sheet(
    lead: Option<AnyElement>,
    rows: Vec<(&'static str, SharedString)>,
    scroll: &ScrollHandle,
) -> Stateful<Div> {
    let mut list = scroll_y(div().id("sheet-rows"), scroll)
        .flex()
        .flex_col()
        .min_h_0()
        .gap(px(2.))
        .mt(px(10.))
        .children(lead);
    for (action, keys) in rows {
        list = list.child(
            div()
                .flex_shrink_0()
                .h(px(30.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(24.))
                .child(
                    div()
                        .text_size(px(theme::TEXT_BODY))
                        .text_color(theme::FG)
                        .child(action),
                )
                .child(
                    div()
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(px(6.))
                        .bg(theme::CONTROL_BG)
                        .text_size(px(theme::TEXT_SMALL))
                        .text_color(theme::FG_DIM)
                        .child(keys),
                ),
        );
    }
    div()
        .id("sheet-scrim")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(theme::alpha(theme::BG, 0.55))
        .p(px(SHEET_MARGIN))
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .max_h_full()
                .p(px(20.))
                .rounded(px(12.))
                .bg(theme::BG_ELEV)
                .shadow(theme::shadow_float())
                // Presses on the card must not reach the scrim's
                // close handler.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_size(px(theme::TEXT_BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::FG)
                        .child("Keyboard shortcuts"),
                )
                .child(list),
        )
}

#[cfg(test)]
mod shortcut_tests {
    // WHY: local actions accept Command on macOS and Control elsewhere.
    // These cover ordinary keys, modified chords, punctuation, and click
    // instructions on both targets without needing a macOS test process.
    #[test]
    fn local_action_shortcuts_follow_each_platform_convention() {
        let actual = [
            (
                super::meta_shortcut_for!(false, "C"),
                super::meta_shortcut_for!(true, "C"),
            ),
            (
                super::meta_shortcut_for!(false, "Shift+Z"),
                super::meta_shortcut_for!(true, "Shift+Z"),
            ),
            (
                super::meta_shortcut_for!(false, ","),
                super::meta_shortcut_for!(true, ","),
            ),
            (
                super::meta_shortcut_for!(false, "click"),
                super::meta_shortcut_for!(true, "click"),
            ),
        ];
        let expected = [
            ("Ctrl+C", "Cmd+C"),
            ("Ctrl+Shift+Z", "Cmd+Shift+Z"),
            ("Ctrl+,", "Cmd+,"),
            ("Ctrl+click", "Cmd+click"),
        ];
        assert_eq!(actual, expected);
    }
}
