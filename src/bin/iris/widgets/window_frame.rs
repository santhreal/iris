//! The Normal-window scaffold: the rounded root, the unified toolbar
//! that is the window's title bar, and the macOS window controls, or a
//! square root with no controls under a platform frame.

use std::rc::Rc;

use gpui::*;

use super::{move_handle, Double};
use crate::theme;

/// Window-control geometry: 12px dots 8px apart, the first 20px from
/// the window's leading edge, vertically centered in the toolbar.
pub const DOT: f32 = 12.0;
pub const DOT_GAP: f32 = 8.0;
pub const DOT_INSET: f32 = 20.0;
/// The leading edge of toolbar content after the window controls.
pub const AFTER_DOTS: f32 = DOT_INSET + 3.0 * DOT + 2.0 * DOT_GAP + 16.0;

/// One window-control dot (close/minimize/zoom). Brightens on hover,
/// darkens on press.
fn win_dot(id: &'static str, color: Rgba) -> Stateful<Div> {
    div()
        .id(ElementId::Name(id.into()))
        .w(px(DOT))
        .h(px(DOT))
        .rounded_full()
        .bg(color)
        .cursor_pointer()
        .hover(move |s| s.bg(theme::alpha(color, 0.85)))
        .active(move |s| s.bg(theme::alpha(color, 0.65)))
        // A press on a window control must not start a title-bar drag.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// What the close control runs. A window that has work to finish on
/// close (a save, a typed value) runs the same handler from its close
/// shortcut and from the window manager's close.
pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// Route every close of `window` through `f` on the view behind `cx`:
/// the returned handler is for the close control (`Toolbar::on_close`)
/// and the close shortcut, and the window manager's close runs it too.
/// `f` removes the window when it is done, at once or after its outro;
/// a window whose view is gone closes at once.
pub fn close_route<V: 'static>(
    window: &Window,
    cx: &mut Context<V>,
    f: impl Fn(&mut V, &mut Window, &mut Context<V>) + 'static,
) -> OnClose {
    let f = Rc::new(f);
    let view = cx.entity().downgrade();
    let (wm_f, wm_view) = (f.clone(), view.clone());
    window.on_window_should_close(cx, move |window, cx| {
        wm_view
            .update(cx, |view, cx| wm_f(view, window, cx))
            .is_err()
    });
    Rc::new(move |window, cx| {
        if view.update(cx, |view, cx| f(view, window, cx)).is_err() {
            window.remove_window();
        }
    })
}

/// The three window controls, macOS order and register: close,
/// minimize, zoom. Every Normal window offers the same controls in the
/// same place. A fixed-size window gets a dimmed, inert zoom control,
/// as on macOS. Close runs `on_close`, or removes the window.
fn traffic_lights(resizable: bool, on_close: Option<OnClose>) -> Div {
    let zoom = if resizable {
        win_dot("win-zoom", theme::WIN_ZOOM)
            .on_click(|_, window, _| crate::sys::window::zoom_control(window))
    } else {
        div()
            .id("win-zoom")
            .w(px(DOT))
            .h(px(DOT))
            .rounded_full()
            .bg(theme::alpha(theme::FG, 0.18))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    };
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(DOT_GAP))
        .child(win_dot("win-close", theme::WIN_CLOSE).on_click(
            move |_, window, cx| match &on_close {
                Some(close) => close(window, cx),
                None => window.remove_window(),
            },
        ))
        .child(win_dot("win-min", theme::WIN_MIN).on_click(|_, window, _| window.minimize_window()))
        .child(zoom)
}

/// What a unified toolbar holds besides the window controls: a title
/// with an optional caption line under it at the leading edge, an
/// optional element centered in the window, a trailing cluster, and
/// what the close control runs.
#[derive(Default)]
pub struct Toolbar {
    pub title: SharedString,
    pub caption: Option<SharedString>,
    pub center: Option<AnyElement>,
    pub trailing: Vec<AnyElement>,
    pub on_close: Option<OnClose>,
}

impl Toolbar {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    pub fn caption(mut self, caption: impl Into<SharedString>) -> Self {
        self.caption = Some(caption.into());
        self
    }

    pub fn center(mut self, el: impl IntoElement) -> Self {
        self.center = Some(el.into_any_element());
        self
    }

    pub fn trailing(mut self, el: impl IntoElement) -> Self {
        self.trailing.push(el.into_any_element());
        self
    }

    pub fn on_close(mut self, on_close: OnClose) -> Self {
        self.on_close = Some(on_close);
        self
    }

    fn is_empty(&self) -> bool {
        self.title.is_empty()
            && self.caption.is_none()
            && self.center.is_none()
            && self.trailing.is_empty()
    }
}

/// The Normal-window scaffold: a rounded root (the window is
/// transparent; this root's 12px corners are the window's shape), a
/// TOOLBAR_H unified toolbar with the traffic lights, the title and
/// caption, the centered element, and the trailing cluster, then the
/// content. The toolbar has no divider: it shares the window's
/// background with the content. The toolbar is the window's title bar
/// (`move_handle`): a drag moves the window, and a double click on a
/// resizable one runs the platform's title bar action.
///
/// A toolbar with nothing in it takes no band: the content fills the
/// window and the traffic lights and the drag strip lie over its top
/// TOOLBAR_H. Under a platform frame (`sys::window::platform_frame`),
/// which has the window controls and the shape, the root is square and
/// the toolbar has no traffic lights; an empty toolbar is dropped.
pub fn toolbar_frame(
    window: &Window,
    toolbar: Toolbar,
    resizable: bool,
    content: impl IntoElement,
) -> Div {
    let framed = crate::sys::window::platform_frame(window);
    let root = div()
        .size_full()
        .relative()
        .font_family(theme::FONT)
        .rounded(window_corner(window, 12.))
        .overflow_hidden()
        .bg(theme::BG)
        .text_color(theme::FG)
        .flex()
        .flex_col();
    let double = if resizable {
        Double::TitleBar
    } else {
        Double::Nothing
    };
    if toolbar.is_empty() {
        if framed {
            return root.child(content);
        }
        return root
            .child(content)
            .child(
                div()
                    .id("frame-toolbar")
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(theme::TOOLBAR_H))
                    .flex()
                    .items_center()
                    .pl(px(DOT_INSET))
                    .child(move_handle(double))
                    .child(traffic_lights(resizable, toolbar.on_close)),
            )
            .children(window_edge(window, 12., 1.0));
    }
    root.child(toolbar_bar(window, toolbar, resizable))
        .child(content)
        .children(window_edge(window, 12., 1.0))
}

/// The 1px rim of a client-drawn window, `radius` round, for the last
/// child of a `relative` root: it lies over the content's edge pixels
/// and takes no input, so the content keeps its layout and its hit
/// areas. `opacity` scales it with chrome that fades. `None` under a
/// platform frame, which draws its own border.
pub fn window_edge(window: &Window, radius: f32, opacity: f32) -> Option<Div> {
    if crate::sys::window::platform_frame(window) {
        return None;
    }
    Some(
        div()
            .absolute()
            .inset_0()
            .rounded(px(radius))
            .border_1()
            .border_color(theme::alpha(
                theme::WINDOW_EDGE,
                theme::WINDOW_EDGE.a * opacity,
            )),
    )
}

/// The TOOLBAR_H unified toolbar band on its own: the traffic lights
/// (none under a platform frame), the title and caption, the centered
/// element, and the trailing cluster. `toolbar_frame` stacks it over
/// the content; a window that positions its own layers (the editor)
/// places it directly. The band is the window's title bar
/// (`move_handle`).
pub fn toolbar_bar(window: &Window, toolbar: Toolbar, resizable: bool) -> Stateful<Div> {
    let framed = crate::sys::window::platform_frame(window);
    let double = if resizable {
        Double::TitleBar
    } else {
        Double::Nothing
    };
    let Toolbar {
        title,
        caption,
        center,
        trailing,
        on_close,
    } = toolbar;
    let mut cluster = div()
        .flex_1()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.));
    for el in trailing {
        cluster = cluster.child(el);
    }
    // Controls in the toolbar handle their own presses.
    cluster = cluster.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    let titles = div()
        .flex()
        .flex_col()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .child(
            div()
                .text_size(px(theme::TEXT_HEADLINE))
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(px(16.))
                .text_color(theme::FG)
                .text_ellipsis()
                .child(title),
        )
        .children(caption.map(|c| {
            div()
                .text_size(px(theme::TEXT_SMALL))
                .line_height(px(14.))
                .text_color(theme::FG_DIM)
                .font_features(theme::tabular())
                .text_ellipsis()
                .child(c)
        }));
    let leading = div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(AFTER_DOTS - DOT_INSET - 3.0 * DOT - 2.0 * DOT_GAP))
        .children((!framed).then(|| traffic_lights(resizable, on_close)))
        .child(titles);
    div()
        .id("frame-toolbar")
        .h(px(theme::TOOLBAR_H))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(12.))
        .pl(px(if framed { theme::INSET } else { DOT_INSET }))
        .pr(px(12.))
        .child(move_handle(double))
        .child(leading)
        .children(center.map(|c| {
            div()
                .flex()
                .flex_shrink_0()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(c)
        }))
        .child(cluster)
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
