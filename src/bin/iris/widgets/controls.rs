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

/// macOS-style switch: the track is white at 16% when off and accent
/// when on; the white knob slides between its rests over SLIDE (at once
/// under reduced motion), and the track crossfades with it. The caller
/// attaches `on_click`.
pub fn toggle(id: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(ElementId::Name(id.into()))
        .w(px(TOGGLE_W))
        .h(px(TOGGLE_H))
        .flex_shrink_0()
        .rounded_full()
        .bg(theme::alpha(theme::FG, 0.16))
        .cursor_pointer()
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
        div()
            .size_full()
            .rounded_full()
            .bg(theme::alpha(theme::ACCENT, pos))
            .child(
                div()
                    .size(px(KNOB))
                    .mt(px(KNOB_INSET))
                    .ml(px(KNOB_INSET + knob_travel() * pos))
                    .rounded_full()
                    .bg(gpui::white())
                    .shadow(theme::shadow_knob()),
            )
    }
}

/// The look of a push button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonStyle {
    /// Text on a quiet fill: the default form button (Choose…, Check Now).
    Bordered,
    /// Text with a hover wash and no fill.
    Ghost,
    /// The one accent-filled action of a surface.
    Primary,
    /// Destructive text, no fill.
    Destructive,
}

/// A form push button, CONTROL_H tall with a body label.
pub fn push_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    style: ButtonStyle,
) -> Stateful<Div> {
    styled_button(
        div().id(id).h(px(theme::CONTROL_H)).px(px(10.)),
        style,
    )
    .child(label.into())
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
        .whitespace_nowrap()
        .cursor_pointer();
    match style {
        ButtonStyle::Bordered => base
            .bg(theme::CONTROL_BG)
            .text_color(theme::FG)
            .hover(|s| s.bg(theme::alpha(theme::FG, 0.14)))
            .active(|s| s.bg(theme::alpha(theme::FG, 0.20))),
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
            .hover(|s| s.bg(theme::alpha(theme::FG, 0.14)))
            .active(|s| s.bg(theme::alpha(theme::FG, 0.20))),
    }
}

/// One segment of a segmented control: a glyph, a label, or both, and
/// an optional tooltip (label plus shortcut).
pub struct Segment {
    icon: Option<Icon>,
    label: Option<SharedString>,
    tip: Option<(SharedString, Option<SharedString>)>,
}

impl Segment {
    pub fn icon(icon: Icon) -> Self {
        Self {
            icon: Some(icon),
            label: None,
            tip: None,
        }
    }

    /// Add a label beside (Toolbar) or under (Tabs) the glyph.
    pub fn with_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The tooltip after the hover delay: `label`, then `shortcut` in
    /// secondary color.
    pub fn tip(mut self, label: impl Into<SharedString>, shortcut: Option<SharedString>) -> Self {
        self.tip = Some((label.into(), shortcut));
        self
    }
}

/// How a segmented control lays out and marks its selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SegmentStyle {
    /// Toolbar tool picker: TOOLBAR_CONTROL_H segments in a quiet
    /// track; the selected segment is accent-filled with white content.
    Toolbar,
    /// Window pane tabs: glyph over label, no track; the selected tab
    /// has a soft fill and accent-tinted glyph and label.
    Tabs,
}

/// Segment geometry per style: (height, glyph size).
fn segment_metrics(style: SegmentStyle) -> (f32, f32) {
    match style {
        SegmentStyle::Toolbar => (theme::TOOLBAR_CONTROL_H, theme::ICON_TOOLBAR),
        SegmentStyle::Tabs => (44.0, 18.0),
    }
}

/// A segmented control: one choice among `segments`, `selected` marked.
/// A click on a segment runs `on_select(index)`, including on the one
/// already selected.
pub fn segmented(
    id: &'static str,
    style: SegmentStyle,
    segments: Vec<Segment>,
    selected: Option<usize>,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> Div {
    let on_select = Rc::new(on_select);
    let (h, glyph) = segment_metrics(style);
    let mut row = div().flex().flex_shrink_0().items_center();
    row = match style {
        SegmentStyle::Toolbar => row
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(theme::RADIUS_CONTROL + 2.))
            .bg(theme::alpha(theme::FG, 0.06)),
        SegmentStyle::Tabs => row.gap(px(2.)),
    };
    for (i, seg) in segments.into_iter().enumerate() {
        let on = selected == Some(i);
        let ink = match (style, on) {
            (SegmentStyle::Toolbar, true) => theme::ACCENT_INK,
            (SegmentStyle::Tabs, true) => theme::ACCENT,
            (_, false) => theme::FG_DIM,
        };
        let mut cell = div()
            .id(ElementId::NamedInteger(id.into(), i as u64))
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(ink)
            .whitespace_nowrap();
        cell = match style {
            SegmentStyle::Toolbar => {
                let w = if seg.label.is_none() { h - 4.0 } else { 0.0 };
                let c = cell
                    .h(px(h - 4.0))
                    .px(px(if seg.label.is_some() { 10. } else { 0. }))
                    .gap(px(6.))
                    .rounded(px(theme::RADIUS_CONTROL))
                    .text_size(px(theme::TEXT_BODY));
                let c = if w > 0.0 { c.w(px(w)) } else { c };
                if on {
                    c.bg(theme::ACCENT)
                } else {
                    c.hover(|s| s.bg(theme::SURFACE_HOVER).text_color(theme::FG))
                        .active(|s| s.bg(theme::SURFACE_PRESS))
                }
            }
            SegmentStyle::Tabs => {
                let c = cell
                    .h(px(h))
                    .min_w(px(64.))
                    .px(px(8.))
                    .flex_col()
                    .gap(px(3.))
                    .rounded(px(theme::RADIUS_CONTROL + 1.))
                    .text_size(px(theme::TEXT_SMALL))
                    .font_weight(FontWeight::MEDIUM);
                if on {
                    c.bg(theme::alpha(theme::FG, 0.09))
                } else {
                    c.hover(|s| s.bg(theme::SURFACE_HOVER).text_color(theme::FG))
                        .active(|s| s.bg(theme::SURFACE_PRESS))
                }
            }
        };
        if let Some(icon) = seg.icon {
            cell = cell.child(icons::icon(icon, ink, glyph));
        }
        if let Some(label) = seg.label {
            cell = cell.child(label);
        }
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
