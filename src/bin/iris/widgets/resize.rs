//! Resize edges for windows that draw their own frame: eight hit strips
//! along the inside of the window's border, each handing a left press
//! to `sys::window::begin_wm_resize` from its edge or corner. Where the
//! platform frame resizes the window (`sys::window::CLIENT_RESIZE`
//! false), and while the window is maximized or fullscreen, there are
//! none.

use gpui::*;

#[cfg(test)]
mod tests;

/// Edge strip thickness, logical px.
const EDGE: f32 = 6.0;
/// A corner square's side, logical px: it reaches further along both
/// edges than a strip is thick, so a diagonal grab needs no pixel aim.
const CORNER: f32 = 16.0;

/// The strips of a `w`x`h` window as `(edge, [x, y, width, height])`,
/// window-local logical px. Corners take the squares at the window's
/// corners; the four sides take the rest of the border between them.
fn strips(w: f32, h: f32) -> [(ResizeEdge, [f32; 4]); 8] {
    use ResizeEdge::*;
    let (e, c) = (EDGE, CORNER);
    let (span_w, span_h) = ((w - 2.0 * c).max(0.0), (h - 2.0 * c).max(0.0));
    [
        (TopLeft, [0.0, 0.0, c, c]),
        (Top, [c, 0.0, span_w, e]),
        (TopRight, [w - c, 0.0, c, c]),
        (Right, [w - e, c, e, span_h]),
        (BottomRight, [w - c, h - c, c, c]),
        (Bottom, [c, h - e, span_w, e]),
        (BottomLeft, [0.0, h - c, c, c]),
        (Left, [0.0, c, e, span_h]),
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

/// The resize strips for a window whose minimum logical size is `min`.
/// Add them as the root's last child: the strips must be over every
/// other element, or a press on the border reaches the content under
/// it.
pub fn resize_edges(window: &Window, min: Size<Pixels>) -> Option<Div> {
    if !crate::sys::window::CLIENT_RESIZE || window.is_maximized() || window.is_fullscreen() {
        return None;
    }
    let tiling = match window.window_decorations() {
        Decorations::Client { tiling } => tiling,
        Decorations::Server => Tiling::default(),
    };
    let size = window.viewport_size();
    let mut layer = div().absolute().top_0().left_0().size_full();
    for (edge, [x, y, w, h]) in strips(size.width.into(), size.height.into()) {
        if !free(edge, tiling) {
            continue;
        }
        layer = layer.child(
            div()
                .id(("resize-edge", edge as usize))
                .absolute()
                .left(px(x))
                .top(px(y))
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
