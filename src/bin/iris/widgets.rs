//! Shared chrome widgets: one implementation per control, used by
//! every surface. The register is defined in theme.rs; glyphs in
//! icons.rs. Buttons are ghost by default (hover wash, no fill), as
//! Apple's chrome reads; the accent fill marks the single primary
//! action of a surface. Every icon-only control takes a `tip`.

mod controls;
mod field;
mod image;
mod menu;
mod resize;
mod scroll;
mod tooltip;
mod window_frame;
mod window_move;
mod window_open;

pub use self::controls::{push_button, segmented, toggle, ButtonStyle, Segment, SegmentStyle};
pub use self::field::{folder_field, text_field, FieldState};
pub use self::image::*;
pub use self::menu::{
    menu, menu_row, menu_separator, popup_button, popup_menu, MenuItem, MENU_PAD, MENU_RADIUS,
    MENU_ROW_H, MENU_W,
};
pub use self::resize::resize_edges;
pub use self::scroll::scroll_y;
pub use self::tooltip::tip;
pub use self::window_frame::{toolbar_bar, toolbar_frame, window_corner, Toolbar};
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

/// Toolbar text button with a trailing glyph (a menu's chevron).
pub fn button_with_icon(
    id: &'static str,
    label: &'static str,
    glyph: Icon,
    primary: bool,
) -> Stateful<Div> {
    let color = if primary {
        theme::ACCENT_INK
    } else {
        theme::FG_DIM
    };
    button(id, label, primary).child(icons::icon(glyph, color, 10.0))
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
        if active {
            theme::ACCENT_INK
        } else {
            theme::FG_DIM
        },
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

/// A keyboard-shortcuts sheet: centered card over a dim layer, rows
/// of action / key combination. The surface owns the open state and
/// closes on Esc or a press on the dim layer.
pub fn shortcuts_sheet(rows: Vec<(&'static str, SharedString)>) -> Stateful<Div> {
    let mut list = div().flex().flex_col().gap(px(2.)).mt(px(10.));
    for (action, keys) in rows {
        list = list.child(
            div()
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
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
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
