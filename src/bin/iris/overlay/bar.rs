//! The capture bar: the overlay's floating control strip, centered near
//! the bottom of the monitor the pointer was on when the session
//! opened, as the macOS screenshot toolbar is. It holds cancel, the
//! four captures (the entire screen or a selection, as a screenshot or
//! as a recording), and the button that runs the chosen one on the
//! committed selection. The confirm and cancel keys run the same two
//! actions; each button's tooltip prints its key.

use std::{sync::Arc, time::Instant};

use gpui::*;
use iris_lib::capture::WinRect;

use super::{Overlay, OverlayMode, MIN_SIZE};
use crate::{
    icons::{self, Icon},
    theme,
    widgets::tip,
};

/// What a press on the frozen frame selects. `Screen` selects the whole
/// monitor under the pointer and a press captures it; `Selection`
/// selects a dragged region or the window under a click.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Screen,
    Selection,
}

/// The edge of a capture choice button, and of the cancel button.
pub(super) const BUTTON: f32 = 36.0;
/// The width of the run button: fixed, so the bar does not change width
/// when its label changes between Capture and Record.
pub(super) const RUN_W: f32 = 80.0;
/// The bar's inner padding and the gap between its children.
pub(super) const PAD: f32 = 6.0;
pub(super) const GAP: f32 = 2.0;
/// The width of the slot a divider line sits centered in.
pub(super) const DIVIDER_W: f32 = 9.0;
/// The bar's height: a button, the padding, and the 1 px border.
pub(super) const BAR_H: f32 = BUTTON + 2.0 * PAD + 2.0;
/// The distance from the monitor's bottom edge to the bar's.
pub(super) const BAR_LIFT: f32 = 64.0;
pub(super) const BAR_RADIUS: f32 = 14.0;
/// The bar's glass: the frozen frame under it averaged into cells this
/// many logical px across, blurred this many cells each way, under a
/// tint dark enough for the bar's white glyphs over a white page.
const GLASS_CELL: f32 = 4.0;
const GLASS_RADIUS: u32 = 3;
pub(super) const GLASS_TINT: f32 = 0.78;

/// The bar's top-left corner and width, logical px, on a monitor at
/// logical `(x, y, w, h)`: centered horizontally, `BAR_LIFT` above the
/// bottom edge, and inside the monitor on one too short or too narrow
/// for that.
pub(super) fn bar_origin(monitor: (f32, f32, f32, f32), bar_w: f32) -> (f32, f32) {
    let (x, y, w, h) = monitor;
    let left = x + ((w - bar_w) / 2.0).max(0.0);
    let top = y + (h - BAR_LIFT - BAR_H).max(0.0);
    (left.round(), top.round())
}

/// The bar's width from its children: cancel, two pairs of choices,
/// three dividers, and the run button, with the gaps between all nine.
pub(super) const fn bar_width() -> f32 {
    2.0 + 2.0 * PAD + 5.0 * BUTTON + 3.0 * DIVIDER_W + RUN_W + 8.0 * GAP
}

/// One capture choice: the four in the bar, in order.
const CHOICES: [(OverlayMode, Scope, Icon, &str); 4] = [
    (
        OverlayMode::Capture,
        Scope::Screen,
        Icon::Screen,
        "Capture Entire Screen",
    ),
    (
        OverlayMode::Capture,
        Scope::Selection,
        Icon::Selection,
        "Capture Selection",
    ),
    (
        OverlayMode::RecordPick,
        Scope::Screen,
        Icon::RecordScreen,
        "Record Entire Screen",
    ),
    (
        OverlayMode::RecordPick,
        Scope::Selection,
        Icon::RecordSelection,
        "Record Selection",
    ),
];

fn divider() -> Div {
    div()
        .w(px(DIVIDER_W))
        .h(px(20.))
        .flex()
        .justify_center()
        .child(div().w(px(1.)).h_full().bg(theme::SEPARATOR))
}

/// A square bar button with `glyph`; `chosen` fills it as the selected
/// capture.
fn square(id: &'static str, glyph: Icon, chosen: bool) -> Stateful<Div> {
    let el = div()
        .id(id)
        .size(px(BUTTON))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_SM))
        .cursor_pointer();
    let el = if chosen {
        el.bg(theme::CHOSEN)
    } else {
        el.hover(|s| s.bg(theme::SURFACE_HOVER))
            .active(|s| s.bg(theme::SURFACE_PRESS))
    };
    el.child(icons::icon(
        glyph,
        if chosen { theme::FG } else { theme::FG_DIM },
        20.0,
    ))
}

impl Overlay {
    /// The monitor the bar sits on, in frame pixels: the one under the
    /// pointer when the session opened, the first, or the whole view
    /// before a layout lands.
    pub(super) fn bar_monitor(&self) -> WinRect {
        let at = self.bar_at.unwrap_or(self.origin);
        WinRect {
            x: at.0,
            y: at.1,
            width: 0,
            height: 0,
        }
        .host(&self.monitors)
        .copied()
        .unwrap_or(WinRect {
            x: self.origin.0,
            y: self.origin.1,
            width: self.view.0,
            height: self.view.1,
        })
    }

    /// The monitor under the logical point `p`, as a logical rect.
    pub(super) fn monitor_at(&self, p: (f32, f32), window: &Window) -> (f32, f32, f32, f32) {
        let sf = window.scale_factor();
        let (sx, sy) = Self::scale(window, self.view);
        let (fx, fy) = Self::frame_pos(sf, sx, sy, p.0, p.1);
        let at = WinRect {
            x: fx as i32 + self.origin.0,
            y: fy as i32 + self.origin.1,
            width: 0,
            height: 0,
        };
        let m = at.host(&self.monitors).copied().unwrap_or(WinRect {
            x: self.origin.0,
            y: self.origin.1,
            width: self.view.0,
            height: self.view.1,
        });
        Self::to_logical(self.origin, &m, sf, sx, sy)
    }

    /// Choose a capture: what the run button does and what a press
    /// selects. The entire screen selects the bar's monitor at once;
    /// a selection keeps the one there is, unless it was a screen.
    pub(super) fn choose(&mut self, mode: OverlayMode, scope: Scope, window: &Window) {
        self.set_mode(mode);
        if scope == Scope::Screen {
            let sf = window.scale_factor();
            let (sx, sy) = Self::scale(window, self.view);
            self.current = Some(Self::to_logical(
                self.origin,
                &self.bar_monitor(),
                sf,
                sx,
                sy,
            ));
            if let Some(old) = self.hovered.take() {
                self.hover_out = Some((old, Instant::now()));
            }
            self.hover_in = None;
        } else if self.scope == Scope::Screen {
            self.current = None;
        }
        self.scope = scope;
    }

    /// The frosted frame under the bar at logical `rect`, rebuilt only
    /// when the rect's frame pixels change; None before the frozen frame
    /// lands, when the bar draws its opaque fill.
    fn glass(
        &mut self,
        window: &Window,
        rect: (f32, f32, f32, f32),
        cx: &mut App,
    ) -> Option<Arc<RenderImage>> {
        let (frame, (fw, fh)) = self.frame_img.as_ref().zip(self.frame_size)?;
        let region = Self::rect_to_frame(window, self.view, rect);
        if let Some((at, glass)) = &self.bar_glass {
            if *at == region {
                return Some(glass.clone());
            }
        }
        let (sx, _) = Self::scale(window, self.view);
        let step = (GLASS_CELL * window.scale_factor() * sx).round().max(1.0) as u32;
        let cells = iris_lib::frost::frost(
            frame.as_bytes(0)?,
            fw,
            fh,
            (region.x, region.y, region.width, region.height),
            step,
            GLASS_RADIUS,
        )?;
        // The frame is BGRA, the RenderImage's own order: the cells
        // keep it, so they go in with no swizzle.
        let glass = Arc::new(RenderImage::new([image::Frame::new(cells)]));
        if let Some((_, old)) = self.bar_glass.replace((region, glass.clone())) {
            crate::widgets::release_render(&old, cx);
        }
        Some(glass)
    }

    /// Whether the run button has a selection to run on.
    fn runnable(&self) -> bool {
        self.current
            .is_some_and(|(_, _, w, h)| w >= MIN_SIZE && h >= MIN_SIZE)
    }

    /// The bar, over everything else. It leaves while a drag, a resize,
    /// or a move is under way, so it neither covers the selection being
    /// shaped nor takes the pointer from it.
    pub(super) fn render_bar(
        &mut self,
        root: Stateful<Div>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        if self.dragging || self.resize.is_some() || self.moving.is_some() {
            self.over_bar = false;
            return root;
        }
        let sf = window.scale_factor();
        let (sx, sy) = Self::scale(window, self.view);
        let monitor = Self::to_logical(self.origin, &self.bar_monitor(), sf, sx, sy);
        let (left, top) = bar_origin(monitor, bar_width());
        // Over the frozen frame the bar is glass: the frame under it,
        // frosted, behind a translucent tint, with the shadow on the
        // glass so it does not darken the bar through the tint.
        let glass = self.glass(window, (left, top, bar_width(), BAR_H), cx);
        let fill = if glass.is_some() { GLASS_TINT } else { 0.96 };
        let glass = glass.map(|frosted| {
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(bar_width()))
                .h(px(BAR_H))
                .rounded(px(BAR_RADIUS))
                .overflow_hidden()
                .shadow(theme::shadow_float())
                .child(
                    img(ImageSource::Render(frosted))
                        .size_full()
                        .object_fit(ObjectFit::Fill)
                        .rounded(px(BAR_RADIUS)),
                )
        });
        let shadow = if glass.is_some() {
            Vec::new()
        } else {
            theme::shadow_float()
        };
        let cancel_keys = SharedString::from(self.cfg.cancel_keybind.clone());
        let confirm_keys = SharedString::from(self.cfg.confirm_keybind.clone());

        let mut bar = div()
            .id("capture-bar")
            .absolute()
            .left(px(left))
            .top(px(top))
            .occlude()
            .flex()
            .items_center()
            .gap(px(GAP))
            .p(px(PAD))
            .rounded(px(BAR_RADIUS))
            .bg(theme::alpha(theme::BG_ELEV, fill))
            .border_1()
            .border_color(theme::HAIRLINE)
            .shadow(shadow)
            .text_size(px(theme::TEXT_BODY))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::FG)
            .cursor_default()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.over_bar = *hovered;
                if *hovered {
                    if let Some(old) = this.hovered.take() {
                        this.hover_out = Some((old, Instant::now()));
                    }
                    this.hover_in = None;
                }
                cx.notify();
            }))
            .child(
                square("bar-cancel", Icon::Close, false)
                    .tooltip(tip("Cancel", Some(cancel_keys)))
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
            )
            .child(divider());
        for (i, (mode, scope, glyph, label)) in CHOICES.into_iter().enumerate() {
            if i == 2 {
                bar = bar.child(divider());
            }
            let chosen = self.mode == mode && self.scope == scope;
            let id = [
                "bar-screen",
                "bar-selection",
                "bar-record-screen",
                "bar-record-selection",
            ][i];
            bar = bar.child(
                square(id, glyph, chosen)
                    .tooltip(tip(label, None))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.choose(mode, scope, window);
                        cx.notify();
                    })),
            );
        }
        let runnable = self.runnable();
        let run = div()
            .id("bar-run")
            .w(px(RUN_W))
            .h(px(BUTTON))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme::RADIUS_SM))
            .bg(theme::ACCENT)
            .text_color(theme::ACCENT_INK)
            .child(self.mode.verb())
            .tooltip(tip(self.mode.verb(), Some(confirm_keys)));
        let run = if runnable {
            run.cursor_pointer()
                .hover(|s| s.bg(theme::alpha(theme::ACCENT, 0.88)))
                .active(|s| s.bg(theme::alpha(theme::ACCENT, 0.74)))
                .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx)))
        } else {
            run.opacity(theme::DISABLED_OPACITY)
        };
        root.children(glass).child(bar.child(divider()).child(run))
    }
}
