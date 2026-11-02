//! The frame of a rectangle the pointer draws and reshapes: the capture
//! overlay's selection and the editor's pending crop. A white line with
//! a dark rim outside the rect, eight round handles centered on the
//! line, each under a two-headed arrow, and readout tags. The editor's
//! selected mark takes the line and rim alone.

use gpui::*;

use crate::theme;

#[cfg(test)]
mod tests;

/// The drawn diameter of a resize handle.
const HANDLE_DOT: f32 = 9.0;
/// The side of a resize handle's hit square, logical px, centered on
/// the point it drags.
pub const HANDLE_PX: f32 = 16.0;
/// A readout's line height; with its padding the tag is 18 px tall.
const READOUT_LINE: f32 = 14.0;
const READOUT_H: f32 = READOUT_LINE + 4.0;
/// The space between a rect's top edge and the bottom of its size
/// readout above it, or the readout's top and left inside it: the NW
/// handle's dot reaches 4.5 px past the corner either way.
const READOUT_GAP: f32 = 10.0;

/// One readout: coordinates, a rect's size, or the loupe's pixel. A
/// small dark tag with no border, as the size readouts of the macOS
/// document apps are, reads over a white page and a dark desktop alike.
/// Digits are tabular so a number that changes under the pointer keeps
/// its width.
pub fn readout(text: SharedString, color: Rgba) -> Div {
    div()
        .px(px(6.))
        .py(px(2.))
        .rounded(px(5.))
        .bg(theme::READOUT_FILL)
        .text_size(px(theme::TEXT_SMALL))
        .line_height(px(READOUT_LINE))
        .font_weight(FontWeight::MEDIUM)
        .font_features(theme::tabular())
        .whitespace_nowrap()
        .text_color(color)
        .child(text)
}

/// The origin of the size readout of the rect at `(x, y)`: aligned with
/// the white line's outer edge above the rect, or inside its top left
/// corner where less than a readout's room is between the line and
/// `ceiling`, the top of the visible area.
pub fn size_readout_at(x: f32, y: f32, hair: f32, ceiling: f32) -> (f32, f32) {
    if y - hair - ceiling >= READOUT_H + READOUT_GAP {
        (x - hair, y - hair - READOUT_GAP - READOUT_H)
    } else {
        (x + READOUT_GAP, y + READOUT_GAP)
    }
}

/// One device pixel at scale factor `sf`, or the nearest whole number
/// of them to a logical pixel, in logical px: gpui draws a 1 px border
/// that many device pixels wide.
pub fn hairline(sf: f32) -> f32 {
    (sf - 0.5).ceil().max(1.0) / sf
}

/// The edge around a logical rect, outside it: a white `hair` line
/// against the rect and a dark `hair` rim outside the line. The pixels
/// inside the rect are the ones the capture or the crop keeps, so the
/// edge covers none of them.
pub fn selection_edge(x: f32, y: f32, w: f32, h: f32, hair: f32) -> Div {
    div()
        .absolute()
        .left(px(x - 2.0 * hair))
        .top(px(y - 2.0 * hair))
        .w(px(w + 4.0 * hair))
        .h(px(h + 4.0 * hair))
        .border_1()
        .border_color(theme::SELECTION_RIM)
        .child(
            div()
                .size_full()
                .border_1()
                .border_color(theme::SELECTION_EDGE),
        )
}

/// The 8 resize handles of the rect `(x, y, w, h)`: 4 corners (NW, NE,
/// SW, SE) then 4 edges (N, S, W, E), each the point it drags.
pub fn handles(x: f32, y: f32, w: f32, h: f32) -> [(f32, f32); 8] {
    [
        (x, y),
        (x + w, y),
        (x, y + h),
        (x + w, y + h),
        (x + w / 2.0, y),
        (x + w / 2.0, y + h),
        (x, y + h / 2.0),
        (x + w, y + h / 2.0),
    ]
}

/// The side length under which a side has no handle at its middle:
/// that handle's square would overlap the corners' squares.
const EDGE_HANDLE_MIN: f32 = 2.0 * HANDLE_PX;

/// The handles of [`handles`] the rect `(x, y, w, h)` shows, with their
/// indices: the 4 corners, and the middle of each side at least
/// EDGE_HANDLE_MIN long. A small rect shows its corners alone.
pub fn shown_handles(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
) -> impl DoubleEndedIterator<Item = (usize, (f32, f32))> {
    handles(x, y, w, h)
        .into_iter()
        .enumerate()
        .filter(move |&(i, _)| match i {
            0..=3 => true,
            4 | 5 => w >= EDGE_HANDLE_MIN,
            _ => h >= EDGE_HANDLE_MIN,
        })
}

/// The shown handle of `rect` whose hit square, `half` to each side of
/// it, holds `p`. Where squares overlap on a small rect, a corner comes
/// before an edge.
pub fn handle_at(rect: (f32, f32, f32, f32), p: (f32, f32), half: f32) -> Option<usize> {
    let (x, y, w, h) = rect;
    shown_handles(x, y, w, h)
        .find(|(_, (hx, hy))| (p.0 - hx).abs() <= half && (p.1 - hy).abs() <= half)
        .map(|(i, _)| i)
}

/// `rect` with handle `handle` of [`handles`] dragged to `p`: a corner
/// moves its two sides, an edge its one. The sides stay inside
/// `0..=bounds`, and a side dragged past the opposite one turns the
/// rect over instead of giving it a negative size.
pub fn reshape(
    handle: usize,
    rect: (f32, f32, f32, f32),
    p: (f32, f32),
    bounds: (f32, f32),
) -> (f32, f32, f32, f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (rect.0, rect.1, rect.0 + rect.2, rect.1 + rect.3);
    match handle {
        0 => (x0, y0) = p,
        1 => (x1, y0) = p,
        2 => (x0, y1) = p,
        3 => (x1, y1) = p,
        4 => y0 = p.1,
        5 => y1 = p.1,
        6 => x0 = p.0,
        _ => x1 = p.0,
    }
    // Clamp both sides before subtracting: the raw far side grows the
    // rect past the bounds when the handle is dragged outside them.
    let span = |a: f32, b: f32, max: f32| {
        let lo = a.min(b).clamp(0.0, max);
        (lo, a.max(b).clamp(0.0, max) - lo)
    };
    let (x, w) = span(x0, x1, bounds.0);
    let (y, h) = span(y0, y1, bounds.1);
    (x, y, w, h)
}

/// The pointer over handle `handle` of [`handles`]: a two-headed arrow
/// along the axis, or the diagonal, it drags.
pub fn handle_cursor(handle: usize) -> CursorStyle {
    match handle {
        0 | 3 => CursorStyle::ResizeUpLeftDownRight,
        1 | 2 => CursorStyle::ResizeUpRightDownLeft,
        4 | 5 => CursorStyle::ResizeUpDown,
        _ => CursorStyle::ResizeLeftRight,
    }
}

/// The pointer while a drag of a selection, or of the view under it,
/// runs or waits on a press: the dragged handle's arrow while a resize
/// runs, a closed hand while a move or a pan runs, and an open hand
/// while one waits. None while none does, and the surface's own cursor
/// shows.
pub fn grab_cursor(resizing: Option<usize>, moving: bool, waiting: bool) -> Option<CursorStyle> {
    match (resizing, moving, waiting) {
        (Some(handle), ..) => Some(handle_cursor(handle)),
        (None, true, _) => Some(CursorStyle::ClosedHand),
        (None, false, true) => Some(CursorStyle::OpenHand),
        (None, false, false) => None,
    }
}

/// Resize handle `handle` centered on `(hx, hy)`: a white dot with a
/// dark rim and a contact shadow in a HANDLE_PX hit square that shows
/// the handle's cursor. The caller attaches any press.
pub fn handle(id: impl Into<ElementId>, handle: usize, hx: f32, hy: f32) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .left(px(hx - HANDLE_PX / 2.0))
        .top(px(hy - HANDLE_PX / 2.0))
        .size(px(HANDLE_PX))
        .flex()
        .items_center()
        .justify_center()
        .cursor(handle_cursor(handle))
        .child(
            div()
                .size(px(HANDLE_DOT))
                .flex_shrink_0()
                .rounded_full()
                .bg(theme::SELECTION_EDGE)
                .border_1()
                .border_color(theme::SELECTION_RIM)
                .shadow(theme::shadow_knob()),
        )
}
