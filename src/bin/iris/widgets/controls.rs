//! Form and toolbar controls: the toggle, the push button, and the
//! segmented control.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::*;

use super::tip;
use crate::icons::{self, Icon};
use crate::{motion, theme};

/// Toggle geometry: a 36x20 track with a 16px knob inset 2px.
pub const TOGGLE_W: f32 = 36.0;
pub const TOGGLE_H: f32 = 20.0;
const KNOB: f32 = 16.0;
const KNOB_INSET: f32 = 2.0;
/// The knob's slide between its two rests.
const SLIDE: Duration = Duration::from_millis(150);
/// The group a toggle's knob reads its press from.
const TOGGLE_GROUP: &str = "toggle";

/// macOS-style switch: the track is white at 16% when off and accent
/// when on; the white knob slides between its rests over SLIDE (at once
/// under reduced motion), and the track crossfades with it. The knob
/// grays while the switch is pressed, before the release flips it. The
/// caller attaches `on_click`.
pub fn toggle(id: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(ElementId::Name(id.into()))
        .group(TOGGLE_GROUP)
        .w(px(TOGGLE_W))
        .h(px(TOGGLE_H))
        .flex_shrink_0()
        .rounded_full()
        .bg(theme::alpha(theme::FG, 0.16))
        .child(ToggleFace { on })
}

/// The knob's travel from its off rest, logical px.
const fn knob_travel() -> f32 {
    TOGGLE_W - KNOB - 2.0 * KNOB_INSET
}

/// The knob's position at `elapsed` into a slide toward `on` that began
/// at position `from` (0.0 off, 1.0 on). Saturates at the target, the
/// end condition of the slide's frame loop.
pub(crate) fn knob_position(from: f32, on: bool, elapsed: Duration, slide: Duration) -> f32 {
    let target = if on { 1.0 } else { 0.0 };
    let k = if slide.is_zero() {
        1.0
    } else {
        (elapsed.as_secs_f32() / slide.as_secs_f32()).min(1.0)
    };
    if k >= 1.0 {
        return target;
    }
    from + (target - from) * motion::ease_out(k)
}

/// A toggle's slide, kept across frames in element state.
struct Slide {
    on: bool,
    from: f32,
    since: Option<Instant>,
}

/// The accent fill and the knob of a toggle. A RenderOnce so the slide
/// state is element state of the toggle that draws it: the first frame
/// shows the knob at rest, and a flip starts the slide from wherever
/// the knob is.
#[derive(IntoElement)]
struct ToggleFace {
    on: bool,
}

impl RenderOnce for ToggleFace {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let on = self.on;
        let state = window.use_keyed_state("toggle-slide", cx, |_, _| Slide {
            on,
            from: if on { 1.0 } else { 0.0 },
            since: None,
        });
        let slide = if cx.reduce_motion() {
            Duration::ZERO
        } else {
            motion::tempo(SLIDE)
        };
        let now = Instant::now();
        let pos = state.update(cx, |s, _| {
            if s.on != on {
                let at = s
                    .since
                    .map_or(s.from, |t0| knob_position(s.from, s.on, now - t0, slide));
                *s = Slide {
                    on,
                    from: at,
                    since: Some(now),
                };
            }
            let pos = s
                .since
                .map_or(s.from, |t0| knob_position(s.from, on, now - t0, slide));
            if pos == if on { 1.0 } else { 0.0 } {
                *s = Slide {
                    on,
                    from: pos,
                    since: None,
                };
            }
            pos
        });
        if state.read(cx).since.is_some() {
            window.request_animation_frame();
        }
        // A flex face: a block face would collapse a top margin on the
        // knob through its own top edge and drop the accent fill with it.
        div()
            .size_full()
            .flex()
            .items_center()
            .rounded_full()
            .bg(theme::alpha(theme::ACCENT, pos))
            .child(
                div()
                    .id("knob")
                    .size(px(KNOB))
                    .flex_shrink_0()
                    .ml(px(KNOB_INSET + knob_travel() * pos))
                    .rounded_full()
                    .bg(gpui::white())
                    .group_active(TOGGLE_GROUP, |s| s.bg(theme::KNOB_PRESS))
                    .shadow(theme::shadow_knob()),
            )
    }
}

/// The look of a push button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonStyle {
    /// Text on a quiet fill: the default form button (Choose…, Check Now).
    Bordered,
    /// Text with a hover wash and no fill: a toolbar button.
    Ghost,
    /// The one accent-filled action of a surface.
    Primary,
    /// Destructive text on the quiet fill: a form button that discards
    /// what the form holds (Restore Defaults).
    Destructive,
    /// Destructive text with a hover wash and no fill: a toolbar's
    /// destructive action (Move to Trash).
    GhostDestructive,
    /// A ghost button held down while the menu it opens is open, as a
    /// pull-down button stays down: the press fill and primary text,
    /// under the pointer too.
    Held,
}

/// A form push button, CONTROL_H tall with a body label.
pub fn push_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    style: ButtonStyle,
) -> Stateful<Div> {
    styled_button(div().id(id).h(px(theme::CONTROL_H)).px(px(10.)), style).child(label.into())
}

/// The shared press and color treatment of every text button.
pub(super) fn styled_button(base: Stateful<Div>, style: ButtonStyle) -> Stateful<Div> {
    let base = base
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(theme::RADIUS_CONTROL))
        .text_size(px(theme::TEXT_BODY))
        .whitespace_nowrap();
    match style {
        ButtonStyle::Bordered => base
            .bg(theme::CONTROL_BG)
            .text_color(theme::FG)
            .hover(|s| s.bg(theme::CONTROL_HOVER))
            .active(|s| s.bg(theme::CONTROL_PRESS)),
        ButtonStyle::Ghost => base
            .text_color(theme::FG_DIM)
            .hover(|s| s.bg(theme::SURFACE_HOVER).text_color(theme::FG))
            .active(|s| s.bg(theme::SURFACE_PRESS)),
        ButtonStyle::Primary => base
            .bg(theme::ACCENT)
            .text_color(theme::ACCENT_INK)
            .font_weight(FontWeight::MEDIUM)
            .hover(|s| s.bg(theme::alpha(theme::ACCENT, 0.88)))
            .active(|s| s.bg(theme::alpha(theme::ACCENT, 0.74))),
        ButtonStyle::Destructive => base
            .bg(theme::CONTROL_BG)
            .text_color(theme::DANGER)
            .hover(|s| s.bg(theme::CONTROL_HOVER))
            .active(|s| s.bg(theme::CONTROL_PRESS)),
        ButtonStyle::GhostDestructive => base
            .text_color(theme::DANGER)
            .hover(|s| s.bg(theme::SURFACE_HOVER))
            .active(|s| s.bg(theme::SURFACE_PRESS)),
        ButtonStyle::Held => base.bg(theme::SURFACE_PRESS).text_color(theme::FG),
    }
}

/// One segment of a segmented control: a glyph and an optional tooltip
/// (label plus shortcut).
pub struct Segment {
    icon: Icon,
    tip: Option<(SharedString, Option<SharedString>)>,
}

impl Segment {
    pub fn icon(icon: Icon) -> Self {
        Self { icon, tip: None }
    }

    /// The tooltip after the hover delay: `label`, then `shortcut` in
    /// secondary color.
    pub fn tip(mut self, label: impl Into<SharedString>, shortcut: Option<SharedString>) -> Self {
        self.tip = Some((label.into(), shortcut));
        self
    }
}

/// A toolbar segmented control: one choice among `segments`, square
/// TOOLBAR_CONTROL_H segments in a quiet track, the `selected` one
/// accent-filled with white content. A click on a segment runs
/// `on_select(index)`, including on the one already selected.
pub fn segmented(
    id: &'static str,
    segments: Vec<Segment>,
    selected: Option<usize>,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> Div {
    let on_select = Rc::new(on_select);
    let side = theme::TOOLBAR_CONTROL_H - 4.0;
    let mut row = div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(theme::RADIUS_CONTROL + 2.))
        .bg(theme::alpha(theme::FG, 0.06));
    for (i, seg) in segments.into_iter().enumerate() {
        let on = selected == Some(i);
        let cell = div()
            .id(ElementId::NamedInteger(id.into(), i as u64))
            .size(px(side))
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .rounded(px(theme::RADIUS_CONTROL))
            .text_color(if on { theme::ACCENT_INK } else { theme::FG_DIM });
        let mut cell = if on {
            cell.bg(theme::ACCENT)
        } else {
            cell.hover(|s| s.bg(theme::SURFACE_HOVER).text_color(theme::FG))
                .active(|s| s.bg(theme::SURFACE_PRESS))
        }
        .child(icons::icon_in_text_color(seg.icon, theme::ICON_TOOLBAR));
        if let Some((label, shortcut)) = seg.tip {
            cell = cell.tooltip(tip(label, shortcut));
        }
        let on_select = on_select.clone();
        cell = cell
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| on_select(i, window, cx));
        row = row.child(cell);
    }
    row
}

// WHY: the class closed here is "a toggle's slide never ends or jumps":
// the knob must reach its rest exactly (the frame loop ends on equality),
// a reversed slide must start from where the knob is rather than jump to
// the far rest, and a zero-length slide (reduced motion) must land at
// once. Not covered: the frame scheduling itself.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    #[test]
    fn the_knob_reaches_its_rest_exactly() {
        for on in [false, true] {
            let target = if on { 1.0 } else { 0.0 };
            for extra in [0, 1, 16, 10_000] {
                let t = SLIDE + Duration::from_millis(extra);
                assert_eq!(knob_position(1.0 - target, on, t, SLIDE), target);
            }
            assert_eq!(
                knob_position(1.0 - target, on, Duration::ZERO, Duration::ZERO),
                target
            );
        }
    }

    #[test]
    fn the_slide_moves_monotonically_from_its_start() {
        let mut last = 0.3;
        for ms in 0..=150 {
            let p = knob_position(0.3, true, Duration::from_millis(ms), SLIDE);
            assert!(p >= last && p <= 1.0, "{ms}ms: {p}");
            last = p;
        }
        assert_eq!(knob_position(0.3, true, Duration::ZERO, SLIDE), 0.3);
        let back = knob_position(0.7, false, Duration::from_millis(75), SLIDE);
        assert!(back < 0.7 && back > 0.0, "{back}");
    }

    #[test]
    fn the_knob_stays_inside_the_track() {
        assert_eq!(KNOB_INSET + knob_travel() + KNOB + KNOB_INSET, TOGGLE_W);
        assert_eq!(KNOB + 2.0 * KNOB_INSET, TOGGLE_H);
    }
}
