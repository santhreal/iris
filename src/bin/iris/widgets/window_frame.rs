//! The Normal-window scaffold: the frame (`AppFrame`) with its shadow
//! and resize edges, the rounded root, the sidebar beside the unified
//! toolbar that is the window's title bar, and the platform's window
//! controls (`window_controls`), or a square root with no controls
//! under a platform frame.

use std::rc::Rc;

use gpui::*;

use super::window_controls::{self, Register, REGISTER};
use super::{move_handle, resize_edges, Double, GRIP};
use crate::sys::window::{Cutout, Rounded};
use crate::theme;

#[cfg(test)]
mod tests;

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
/// what the close control runs. Beside a sidebar, `lead` is the title's
/// inset from the page's leading edge, the page content's
/// (`page_lead`); INSET when None.
#[derive(Default)]
pub struct Toolbar {
    pub title: SharedString,
    pub caption: Option<SharedString>,
    pub center: Option<AnyElement>,
    pub trailing: Vec<AnyElement>,
    pub on_close: Option<OnClose>,
    pub lead: Option<f32>,
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

    pub fn lead(mut self, lead: f32) -> Self {
        self.lead = Some(lead);
        self
    }
}

/// The 1px rim of a client-drawn window for the last child of its
/// frame: it lies over the content's edge pixels and takes no input, so
/// the content keeps its layout and its hit areas. It follows the
/// frame's corners (`window_corners`), and a side tiled against a
/// screen edge or a neighbor has none. `opacity` scales it with chrome
/// that fades. `None` where the platform draws the outline
/// (`sys::window::platform_shape`) or every side is tiled.
pub fn window_edge(window: &Window, opacity: f32) -> Option<Div> {
    let t = frame_tiling(window);
    if t == Tiling::tiled() {
        return None;
    }
    let side = |tiled: bool| px(if tiled { 0. } else { 1. });
    Some(
        with_corners(div(), corners(t))
            .absolute()
            .inset_0()
            .border_l(side(t.left))
            .border_t(side(t.top))
            .border_r(side(t.right))
            .border_b(side(t.bottom))
            .border_color(theme::alpha(
                theme::WINDOW_EDGE,
                theme::WINDOW_EDGE.a * opacity,
            )),
    )
}

/// The corners of a client-drawn window's frame: RADIUS_WINDOW round
/// between two free sides, square beside a side tiled against a screen
/// edge or a neighbor, and square everywhere the platform draws the
/// outline (`sys::window::platform_shape`) and clips the window to it.
pub fn window_corners(window: &Window) -> Corners<Pixels> {
    corners(frame_tiling(window))
}

/// The corners of a frame tiled on the sides `t` holds.
fn corners(t: Tiling) -> Corners<Pixels> {
    let r = |a: bool, b: bool| px(if a || b { 0. } else { theme::RADIUS_WINDOW });
    Corners {
        top_left: r(t.top, t.left),
        top_right: r(t.top, t.right),
        bottom_right: r(t.bottom, t.right),
        bottom_left: r(t.bottom, t.left),
    }
}

/// The sides of `window`'s frame that meet a screen edge or a neighbor
/// and so have no corner or rim of their own: the tiled ones, every one
/// in fullscreen, and every one where the platform draws the outline.
fn frame_tiling(window: &Window) -> Tiling {
    if crate::sys::window::platform_shape(window) || window.is_fullscreen() {
        return Tiling::tiled();
    }
    match window.window_decorations() {
        Decorations::Client { tiling } => tiling,
        Decorations::Server => Tiling::default(),
    }
}

/// The frame of an app window that draws its own: the window's root at
/// the client area (`Window::client_area`), clipped to the frame's
/// corners, with the frame's shadow (`theme::window_shadow`) in the room
/// the window keeps around it (`sys::window::frame_room`), and the
/// resize edges over it. Only the frame and GRIP past it take pointer
/// input, so a press on the shadow reaches the window beneath. Hold one
/// per window.
#[derive(Default)]
pub struct AppFrame {
    input: Cutout,
    /// Whether the input shape was ever cut, so a window with no room
    /// sends none.
    cut: bool,
}

impl AppFrame {
    /// `root` in the frame. The root lays out in frame-local logical px
    /// (`frame_local`); `fade` scales the shadow with chrome that fades;
    /// `min` is the frame's minimum logical size.
    pub fn wrap(
        &mut self,
        window: &Window,
        root: impl IntoElement,
        fade: f32,
        min: Size<Pixels>,
    ) -> Div {
        let area = window.client_area();
        let room = area.size != window.viewport_size();
        if room || self.cut {
            let grip = if room { GRIP } else { 0. };
            let input = Rounded {
                x: f32::from(area.origin.x) - grip,
                y: f32::from(area.origin.y) - grip,
                w: f32::from(area.size.width) + 2. * grip,
                h: f32::from(area.size.height) + 2. * grip,
                r: 0.,
            };
            self.input.set(window, &[input]);
            self.cut = true;
        }
        let mut frame = window_rounded(div(), window)
            .absolute()
            .left(area.origin.x)
            .top(area.origin.y)
            .w(area.size.width)
            .h(area.size.height)
            .overflow_hidden();
        if room {
            frame = frame.shadow(theme::window_shadow(window.is_window_active(), fade));
        }
        div()
            .size_full()
            .relative()
            .child(frame.child(root))
            .children(resize_edges(window, area, min))
    }
}

/// `p`, a window-local position such as a mouse event's, in the
/// frame-local logical px an `AppFrame`'s root lays out in.
pub fn frame_local(window: &Window, p: Point<Pixels>) -> Point<Pixels> {
    p - window.client_area().origin
}

/// The TOOLBAR_H unified toolbar band on its own: the window controls
/// (`window_controls`; none under a platform frame), the title and
/// caption, the centered element, and the trailing cluster.
/// `sidebar_frame` places it over the content beside a sidebar; a
/// window that positions its own layers (the editor) places it
/// directly. The band is the window's title bar (`move_handle`): a drag
/// moves the window, and a double click on a resizable one runs the
/// platform's title bar action.
pub fn toolbar_bar(window: &Window, cx: &App, toolbar: Toolbar, resizable: bool) -> Stateful<Div> {
    bar(window, cx, toolbar, resizable, true)
}

/// The app window scaffold: a SIDEBAR_W sidebar beside a column of the
/// unified toolbar over `content`. The sidebar's top band is the
/// leading part of the title bar: macOS draws its traffic lights there,
/// and a Linux desktop that puts window buttons on the leading side
/// gets them there; elsewhere it is empty. The toolbar beside it starts
/// its title at the toolbar's `lead` and holds the trailing controls.
pub fn sidebar_frame(
    window: &Window,
    cx: &App,
    sidebar: impl IntoElement,
    toolbar: Toolbar,
    resizable: bool,
    content: impl IntoElement,
) -> Div {
    let framed = crate::sys::window::platform_frame(window);
    let lead = if framed {
        None
    } else {
        let sides = window_controls::sides(REGISTER, cx.button_layout(), resizable);
        window_controls::side(sides.left, resizable, &toolbar.on_close, window)
    };
    let band = div()
        .id("sidebar-band")
        .relative()
        .h(px(theme::TOOLBAR_H))
        .flex_shrink_0()
        .flex()
        .items_center()
        .pl(px(window_controls::ROUND_EDGE))
        .child(move_handle(if resizable {
            Double::TitleBar
        } else {
            Double::Nothing
        }))
        .children(lead);
    let side = div()
        .w(px(theme::SIDEBAR_W))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .bg(theme::SIDEBAR_BG)
        .border_r_1()
        .border_color(theme::HAIRLINE)
        .child(band)
        .child(sidebar);
    let column = div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(bar(window, cx, toolbar, resizable, false))
        .child(content);
    window_rounded(div(), window)
        .size_full()
        .relative()
        .font_family(theme::FONT)
        .overflow_hidden()
        .bg(theme::BG)
        .text_color(theme::FG)
        .flex()
        .child(side)
        .child(column)
        .children(window_edge(window, 1.0))
}

/// Where content `content` wide starts in a page `page` wide beside a
/// sidebar, logical px: centered, with at least INSET on either side,
/// on a whole pixel. The page's toolbar title starts there too
/// (`Toolbar::lead`), so the title and the content share one leading
/// edge at every width.
pub fn page_lead(page: f32, content: f32) -> f32 {
    theme::INSET
        + ((page - 2.0 * theme::INSET - content) / 2.0)
            .max(0.0)
            .round()
}

/// The toolbar band: `lead_controls` false leaves the leading window
/// controls to a sidebar's band and starts the title at the toolbar's
/// `lead`.
fn bar(
    window: &Window,
    cx: &App,
    toolbar: Toolbar,
    resizable: bool,
    lead_controls: bool,
) -> Stateful<Div> {
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
        lead: title_lead,
    } = toolbar;
    // A platform frame has its own controls.
    let (lead, trail) = if framed {
        (None, None)
    } else {
        let sides = window_controls::sides(REGISTER, cx.button_layout(), resizable);
        (
            window_controls::side(sides.left, resizable, &on_close, window)
                .filter(|_| lead_controls),
            window_controls::side(sides.right, resizable, &on_close, window),
        )
    };
    let pl = match (framed, REGISTER, lead.is_some()) {
        _ if !lead_controls => title_lead.unwrap_or(theme::INSET),
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
    // A center element centers in the window, not between the bar's
    // paddings: the side with the smaller padding starts its flex basis
    // at the difference, so both sides reach equally far from the center
    // while their content fits. A side at its content's width lays out
    // as it would without the basis.
    let skew = if center.is_some() { pl - pr } else { 0. };
    let leading = div()
        .flex_1()
        .flex_basis(px((-skew).max(0.)))
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(16.))
        .children(lead)
        .child(titles);
    let trailing = div()
        .flex_1()
        .flex_basis(px(skew.max(0.)))
        .h_full()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(if REGISTER == Register::Windows {
            8.
        } else {
            window_controls::ROUND_LEAD
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

/// `el`, a window root or the frame around it, with the frame's corners
/// (`window_corners`).
pub fn window_rounded<E: Styled>(el: E, window: &Window) -> E {
    with_corners(el, window_corners(window))
}

fn with_corners<E: Styled>(el: E, c: Corners<Pixels>) -> E {
    el.rounded_tl(c.top_left)
        .rounded_tr(c.top_right)
        .rounded_br(c.bottom_right)
        .rounded_bl(c.bottom_left)
}
