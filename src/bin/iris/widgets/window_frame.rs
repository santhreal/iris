//! The Normal-window scaffold: the rounded root, the unified toolbar
//! that is the window's title bar, and the platform's window controls
//! (`window_controls`), or a square root with no controls under a
//! platform frame.

use std::rc::Rc;

use gpui::*;

use super::window_controls::{self, Register, REGISTER};
use super::{move_handle, Double};
use crate::theme;

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
}

/// The Normal-window scaffold: a rounded root (the window is
/// transparent; this root's 12px corners are the window's shape), a
/// TOOLBAR_H unified toolbar (`toolbar_bar`), then the content. The
/// toolbar has no divider: it shares the window's background with the
/// content. Where the platform draws the outline
/// (`sys::window::platform_shape`), the root is square.
pub fn toolbar_frame(
    window: &Window,
    cx: &App,
    toolbar: Toolbar,
    resizable: bool,
    content: impl IntoElement,
) -> Div {
    div()
        .size_full()
        .relative()
        .font_family(theme::FONT)
        .rounded(window_corner(window, 12.))
        .overflow_hidden()
        .bg(theme::BG)
        .text_color(theme::FG)
        .flex()
        .flex_col()
        .child(toolbar_bar(window, cx, toolbar, resizable))
        .child(content)
        .children(window_edge(window, 12., 1.0))
}

/// The 1px rim of a client-drawn window, `radius` round, for the last
/// child of a `relative` root: it lies over the content's edge pixels
/// and takes no input, so the content keeps its layout and its hit
/// areas. `opacity` scales it with chrome that fades. `None` where the
/// platform draws the outline (`sys::window::platform_shape`).
pub fn window_edge(window: &Window, radius: f32, opacity: f32) -> Option<Div> {
    if crate::sys::window::platform_shape(window) {
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

/// The TOOLBAR_H unified toolbar band on its own: the window controls
/// (`window_controls`; none under a platform frame), the title and
/// caption, the centered element, and the trailing cluster.
/// `toolbar_frame` stacks it over the content; a window that positions
/// its own layers (the editor) places it directly. The band is the
/// window's title bar (`move_handle`): a drag moves the window, and a
/// double click on a resizable one runs the platform's title bar
/// action.
pub fn toolbar_bar(window: &Window, cx: &App, toolbar: Toolbar, resizable: bool) -> Stateful<Div> {
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
    // A platform frame has its own controls.
    let (lead, trail) = if framed {
        (None, None)
    } else {
        let sides = window_controls::sides(REGISTER, cx.button_layout(), resizable);
        (
            window_controls::side(sides.left, resizable, &on_close, window),
            window_controls::side(sides.right, resizable, &on_close, window),
        )
    };
    let pl = match (framed, REGISTER, lead.is_some()) {
        (false, Register::Mac, _) => window_controls::MAC_LEADING,
        (_, _, true) => window_controls::ROUND_EDGE,
        _ => theme::INSET,
    };
    let pr = match (REGISTER, trail.is_some()) {
        (Register::Windows, true) => 0.,
        (_, true) => window_controls::ROUND_EDGE,
        _ => 12.,
    };
    let mut cluster = div().flex().items_center().gap(px(8.));
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
        .gap(px(16.))
        .children(lead)
        .child(titles);
    let trailing = div()
        .flex_1()
        .h_full()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(if REGISTER == Register::Windows {
            8.
        } else {
            12.
        }))
        .child(cluster)
        .children(trail);
    div()
        .id("frame-toolbar")
        .h(px(theme::TOOLBAR_H))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(12.))
        .pl(px(pl))
        .pr(px(pr))
        .child(move_handle(double))
        .child(leading)
        .children(center.map(|c| {
            div()
                .flex()
                .flex_shrink_0()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(c)
        }))
        .child(trailing)
}

/// The corner radius of a window root that draws the window's shape:
/// `radius` logical px, or square where the platform draws the outline
/// (`sys::window::platform_shape`) and clips the window to it.
pub fn window_corner(window: &Window, radius: f32) -> Pixels {
    if crate::sys::window::platform_shape(window) {
        px(0.)
    } else {
        px(radius)
    }
}
