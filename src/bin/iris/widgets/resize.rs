//! Resize edges for windows that draw their own frame: eight hit strips
//! along the inside of the frame's border, and GRIP past it into the
//! room a window keeps around its frame for the frame's shadow
//! (`sys::window::frame_room`), each handing a left press to
//! `sys::window::begin_wm_resize` from its edge or corner. Where the
//! platform frame resizes the window (`sys::window::CLIENT_RESIZE`
//! false, or `sys::window::platform_frame`), and while the window is
//! maximized or fullscreen, there are none.

use gpui::*;

#[cfg(test)]
mod tests;

/// Edge strip thickness, logical px.
const EDGE: f32 = 6.0;
/// A corner square's side, logical px: it reaches further along both
/// edges than a strip is thick, so a diagonal grab needs no pixel aim.
const CORNER: f32 = 16.0;
/// How far the strips reach past a frame with room around it, logical
/// px: the room holds the frame's shadow and takes no other input.
pub const GRIP: f32 = 8.0;

/// The strips of a `w`x`h` frame as `(edge, [x, y, width, height])`,
/// frame-local logical px, reaching `out` past the frame. Corners take
/// the squares at the frame's corners; the four sides take the rest of
/// the border between them.
fn strips(w: f32, h: f32, out: f32) -> [(ResizeEdge, [f32; 4]); 8] {
    use ResizeEdge::*;
    let (e, c) = (EDGE + out, CORNER + out);
    let (x0, y0, x1, y1) = (-out, -out, w + out, h + out);
    let (span_w, span_h) = ((w - 2.0 * CORNER).max(0.0), (h - 2.0 * CORNER).max(0.0));
    [
        (TopLeft, [x0, y0, c, c]),
        (Top, [x0 + c, y0, span_w, e]),
        (TopRight, [x1 - c, y0, c, c]),
        (Right, [x1 - e, y0 + c, e, span_h]),
        (BottomRight, [x1 - c, y1 - c, c, c]),
        (Bottom, [x0 + c, y1 - e, span_w, e]),
        (BottomLeft, [x0, y1 - c, c, c]),
        (Left, [x0, y0 + c, e, span_h]),
    ]
}

/// Whether `edge` moves only sides that are free to move: a side tiled
/// against a screen edge or a neighbor stays put.
fn free(edge: ResizeEdge, t: Tiling) -> bool {
    use ResizeEdge::*;
    match edge {
        Top => !t.top,
        Bottom => !t.bottom,
        Left => !t.left,
        Right => !t.right,
        TopLeft => !t.top && !t.left,
        TopRight => !t.top && !t.right,
        BottomLeft => !t.bottom && !t.left,
        BottomRight => !t.bottom && !t.right,
    }
}

fn cursor(edge: ResizeEdge) -> CursorStyle {
    use ResizeEdge::*;
    match edge {
        Left | Right => CursorStyle::ResizeLeftRight,
        Top | Bottom => CursorStyle::ResizeUpDown,
        TopLeft | BottomRight => CursorStyle::ResizeUpLeftDownRight,
        TopRight | BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// The resize strips for a window whose frame is at `area`, window-local
/// logical px, and whose frame's minimum logical size is `min`. Add them
/// above the frame: the strips must be over every other element, or a
/// press on the border reaches the content under it.
pub fn resize_edges(window: &Window, area: Bounds<Pixels>, min: Size<Pixels>) -> Option<Div> {
    if !crate::sys::window::CLIENT_RESIZE || window.is_maximized() || window.is_fullscreen() {
        return None;
    }
    // A platform frame resizes the window from its own border.
    let Decorations::Client { tiling } = window.window_decorations() else {
        return None;
    };
    let viewport = window.viewport_size();
    let out = if area.size == viewport { 0.0 } else { GRIP };
    // The window's own minimum, where an X11 resize without the window
    // manager stops: the frame's and the room around it.
    let min = min + (viewport - area.size);
    let (fx, fy) = (f32::from(area.origin.x), f32::from(area.origin.y));
    let mut layer = div().absolute().top_0().left_0().size_full();
    for (edge, [x, y, w, h]) in strips(area.size.width.into(), area.size.height.into(), out) {
        if !free(edge, tiling) {
            continue;
        }
        layer = layer.child(
            div()
                .id(("resize-edge", edge as usize))
                .absolute()
                .left(px(fx + x))
                .top(px(fy + y))
                .w(px(w))
                .h(px(h))
                // Content under the strip neither hovers nor takes
                // the press; the wheel still scrolls it.
                .block_mouse_except_scroll()
                .cursor(cursor(edge))
                .on_mouse_down(MouseButton::Left, move |ev, window, cx| {
                    cx.stop_propagation();
                    crate::sys::window::begin_wm_resize(window, edge, ev.position, min);
                }),
        );
    }
    Some(layer)
}
