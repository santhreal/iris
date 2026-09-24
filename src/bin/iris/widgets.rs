//! Shared chrome widgets: one implementation per control, used by
//! every surface. The register lives in theme.rs; glyphs live in
//! icons.rs. Buttons are ghost by default (hover wash, no fill)
//! because that is how Apple's chrome reads; filled is reserved for
//! the single primary action.

mod image;
mod resize;
mod window_move;

pub use self::image::*;
pub use self::resize::resize_edges;
pub use self::window_move::{move_handle, Double};

use gpui::*;

use crate::{icons, theme};
use icons::Icon;

/// Text button. Ghost (hover wash) unless `primary`, which is the
/// one filled action on a surface.
pub fn button(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    let base = div()
        .id(ElementId::Name(id.into()))
        .h(px(theme::CONTROL_H))
        .px(px(12.))
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(theme::RADIUS_CONTROL))
        .text_size(px(theme::TEXT_BODY))
        .whitespace_nowrap()
        .cursor_pointer()
        .child(label);
    if primary {
        base.bg(theme::ACCENT)
            .text_color(theme::ACCENT_INK)
            .font_weight(FontWeight::SEMIBOLD)
            .active(|s| s.bg(theme::alpha(theme::ACCENT, 0.82)))
    } else {
        base.text_color(theme::FG_DIM)
            .hover(|s| s.bg(theme::SURFACE_HOVER).text_color(theme::FG))
            .active(|s| s.bg(theme::SURFACE_PRESS))
    }
}

/// Text button with a trailing glyph (dropdown chevrons).
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

/// Icon-only button, `size`px square. `active` fills the chip and
/// inverts the glyph; otherwise ghost with a hover wash.
pub fn icon_button(
    id: impl Into<ElementId>,
    glyph: Icon,
    active: bool,
    size: f32,
) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(size))
        .h(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.))
        .cursor_pointer()
        .flex_shrink_0()
        .bg(if active {
            theme::FG
        } else {
            theme::alpha(theme::FG, 0.0)
        })
        .hover(move |s| {
            if active {
                s
            } else {
                s.bg(theme::SURFACE_HOVER)
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
            if active {
                theme::ACCENT_INK
            } else {
                theme::FG_DIM
            },
            size * 0.5,
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
        .rounded(px(theme::RADIUS_SM))
        .bg(if active {
            theme::FG
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
                s.bg(theme::SURFACE_HOVER)
            }
        })
        .child(icons::icon(
            glyph,
            if active { theme::ACCENT_INK } else { theme::FG },
            14.0,
        ))
}

/// Menu geometry, fixed so callers can place and hit-test a menu
/// without measuring: rows are exactly MENU_ROW_H tall and the panel
/// is MENU_W wide. A menu of `rows` is MENU_ROW_H*rows + 10 tall.
pub const MENU_W: f32 = 152.0;
pub const MENU_ROW_H: f32 = 28.0;

/// Dropdown menu surface. Caller fills it with `menu_row`s.
pub fn menu() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(10.))
        .bg(theme::alpha(theme::BG_ELEV, 0.96))
        .border_1()
        .border_color(theme::HAIRLINE)
        .shadow(theme::shadow_float())
        .py(px(4.))
        .w(px(MENU_W))
}

pub fn menu_row(id: &'static str, label: &'static str) -> Stateful<Div> {
    menu_row_owned(ElementId::Name(id.into()), label)
}

/// A menu row with a runtime label (dropdown options, file names).
/// `&'static str` stores without an allocation; a `String` moves
/// into the Arc.
pub fn menu_row_owned(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(MENU_ROW_H))
        .px(px(12.))
        .flex()
        .items_center()
        .text_size(px(theme::TEXT_BODY))
        .whitespace_nowrap()
        .text_color(theme::FG)
        .cursor_pointer()
        // A menu floats above other interactive surfaces; without
        // this the press falls through to whatever is beneath (the
        // toast card's drag tracking, the editor's topbar buttons).
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .hover(|s| s.bg(theme::SURFACE_HOVER))
        .child(label.into())
}

/// A dropdown field: a button showing the current value with a
/// chevron. The parent owns the open state and builds the option
/// menu beneath it when open (see `dropdown_row` in settings).
pub fn dropdown(
    id: impl Into<ElementId>,
    current: impl Into<SharedString>,
    open: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(theme::CONTROL_H))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .rounded(px(theme::RADIUS_CONTROL))
        .bg(theme::FIELD_BG)
        .border_1()
        .border_color(if open { theme::ACCENT } else { theme::HAIRLINE })
        .text_size(px(theme::TEXT_BODY))
        .text_color(theme::FG)
        .cursor_pointer()
        .child(current.into())
        .child(icons::icon(Icon::ChevronDown, theme::FG_DIM, 10.0))
}

/// macOS-style toggle: 40x22 pill, 16px knob. The knob is white in
/// both states; the track carries the state.
pub fn toggle(id: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(ElementId::Name(id.into()))
        .w(px(40.))
        .h(px(22.))
        .rounded_full()
        .bg(if on {
            theme::ACCENT
        } else {
            theme::alpha(theme::FG, 0.16)
        })
        .cursor_pointer()
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .rounded_full()
                .bg(if on { theme::ACCENT_INK } else { theme::FG })
                // macOS toggle knobs carry a small drop shadow so they
                // read as a raised control, not a flat disc.
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::hsla(0.0, 0.0, 0.0, 0.28),
                    offset: gpui::point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                }])
                .ml(if on { px(21.) } else { px(3.) })
                .mt(px(3.)),
        )
}

/// Single-line text field box. Resting state has no border, like a
/// macOS form field; editing draws a soft ring.
pub fn text_field(id: impl Into<ElementId>, text: String, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h(px(theme::CONTROL_H))
        .px(px(10.))
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .rounded(px(theme::RADIUS_CONTROL))
        .bg(theme::FIELD_BG)
        .border_1()
        .border_color(if active {
            theme::alpha(theme::FG, 0.4)
        } else {
            theme::alpha(theme::FG, 0.0)
        })
        .text_size(px(theme::TEXT_BODY))
        .text_color(theme::FG)
        .cursor_text()
        .child(text)
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

/// One window-control dot (close/minimize/zoom). 12px, brightens on
/// hover, darkens on press.
fn win_dot(id: &'static str, color: Rgba) -> Stateful<Div> {
    div()
        .id(ElementId::Name(id.into()))
        .w(px(12.))
        .h(px(12.))
        .rounded_full()
        .bg(color)
        .cursor_pointer()
        .hover(move |s| s.bg(theme::alpha(color, 0.85)))
        .active(move |s| s.bg(theme::alpha(color, 0.65)))
        // A press on a window control must not start a title-bar drag.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// The three window controls, macOS order and register: close,
/// minimize, zoom. Used by window_frame so every Normal window offers
/// the same controls in the same place. A fixed-size window gets a
/// dimmed, inert zoom control, as on macOS.
fn traffic_lights(resizable: bool) -> Div {
    let zoom = if resizable {
        win_dot("win-zoom", theme::WIN_ZOOM)
            .on_click(|_, window, _| crate::sys::window::zoom_control(window))
    } else {
        div()
            .id("win-zoom")
            .w(px(12.))
            .h(px(12.))
            .rounded_full()
            .bg(theme::alpha(theme::FG, 0.18))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            win_dot("win-close", theme::WIN_CLOSE).on_click(|_, window, _| window.remove_window()),
        )
        .child(win_dot("win-min", theme::WIN_MIN).on_click(|_, window, _| window.minimize_window()))
        .child(zoom)
}

/// The shared Normal-window scaffold: rounded root (the window is
/// transparent; this root's 12px corners are the window's shape),
/// a 56px toolbar with traffic lights, a semibold title and the
/// caller's right-side cluster, then the content. The toolbar is the
/// window's title bar (`move_handle`): a drag moves the window, and a
/// double click on a resizable one runs the platform's title bar action.
pub fn window_frame(
    title: &'static str,
    resizable: bool,
    right: Vec<AnyElement>,
    content: impl IntoElement,
) -> Div {
    let mut cluster = div().flex().items_center().gap(px(8.));
    for el in right {
        cluster = cluster.child(el);
    }
    // Buttons and fields in the cluster handle their own presses.
    cluster = cluster.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    div()
        .size_full()
        .font_family(theme::FONT)
        .rounded(px(12.))
        .overflow_hidden()
        .bg(theme::BG)
        .flex()
        .flex_col()
        .child(
            div()
                .id("frame-toolbar")
                .h(px(56.))
                .flex()
                .items_center()
                .justify_between()
                .px(px(20.))
                .border_b_1()
                .border_color(theme::HAIRLINE)
                .child(move_handle(if resizable {
                    Double::TitleBar
                } else {
                    Double::Nothing
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .child(traffic_lights(resizable))
                        .child(
                            div()
                                .text_size(px(theme::TEXT_TITLE))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::FG)
                                .child(title),
                        ),
                )
                .child(cluster),
        )
        .child(content)
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
                        .bg(theme::FIELD_BG)
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
