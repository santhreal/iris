//! The Normal-window scaffold: the rounded root, the toolbar that is
//! the window's title bar, and the macOS window controls, or a square
//! root with no controls under a platform frame.

use gpui::*;

use super::{move_handle, Double};
use crate::theme;

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
/// Under a platform frame (`sys::window::platform_frame`), which has the
/// window controls and the shape, the root is square and the toolbar has
/// no traffic lights; a toolbar left with nothing in it is dropped.
pub fn window_frame(
    window: &Window,
    title: &'static str,
    resizable: bool,
    right: Vec<AnyElement>,
    content: impl IntoElement,
) -> Div {
    let framed = crate::sys::window::platform_frame(window);
    let root = div()
        .size_full()
        .font_family(theme::FONT)
        .rounded(window_corner(window, 12.))
        .overflow_hidden()
        .bg(theme::BG)
        .flex()
        .flex_col();
    if framed && title.is_empty() && right.is_empty() {
        return root.child(content);
    }
    let mut cluster = div().flex().items_center().gap(px(8.));
    for el in right {
        cluster = cluster.child(el);
    }
    // Buttons and fields in the cluster handle their own presses.
    cluster = cluster.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    root.child(
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
                    .children((!framed).then(|| traffic_lights(resizable)))
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

/// The corner radius of a window root that draws the window's shape:
/// `radius` logical px, or square under a platform frame, where the
/// window has no transparent corners to show the desktop through.
pub fn window_corner(window: &Window, radius: f32) -> Pixels {
    if crate::sys::window::platform_frame(window) {
        px(0.)
    } else {
        px(radius)
    }
}
