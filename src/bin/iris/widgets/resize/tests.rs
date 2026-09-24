// WHY: the strips are the only resize affordance a Linux window has.
// Three classes are closed here. "A border pixel falls through": a gap
// between strips hands the press to the content under it, which moves
// the window from the toolbar or draws on the editor canvas. "A strip
// drags the wrong side": a corner that resizes one axis, a swapped
// diagonal cursor. "A tiled side resizes": a strip on a side held
// against the screen edge. Every ResizeEdge is run; `sides` matches the
// enum exhaustively, so a new variant does not compile until it is
// listed. Not covered: the strips being painted above the content,
// which window-chrome-probe.py drives through the pointer.

use std::prelude::v1::test;

use gpui::ResizeEdge::{self, *};
use gpui::{CursorStyle, Tiling};

use super::{cursor, free, strips, CORNER, EDGE};

const ALL: [ResizeEdge; 8] = [
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    TopLeft,
];

/// The sides `edge` drags: (left, top, right, bottom).
fn sides(edge: ResizeEdge) -> (bool, bool, bool, bool) {
    match edge {
        Top => (false, true, false, false),
        TopRight => (false, true, true, false),
        Right => (false, false, true, false),
        BottomRight => (false, false, true, true),
        Bottom => (false, false, false, true),
        BottomLeft => (true, false, false, true),
        Left => (true, false, false, false),
        TopLeft => (true, true, false, false),
    }
}

#[test]
fn the_border_is_covered_once_and_each_strip_drags_the_sides_it_touches() {
    for (w, h) in [(640.0, 480.0), (1101.0, 721.0)] {
        let strips = strips(w, h);
        // Pixel centers, so no sample sits on a strip boundary.
        for yi in 0..h as usize {
            for xi in 0..w as usize {
                let (x, y) = (xi as f32 + 0.5, yi as f32 + 0.5);
                let mut hits = strips.iter().filter(|(_, [sx, sy, sw, sh])| {
                    x >= *sx && x < sx + sw && y >= *sy && y < sy + sh
                });
                let first = hits.next().map(|(edge, _)| *edge);
                if let Some((second, _)) = hits.next() {
                    panic!("({x}, {y}) in {w}x{h} is under {first:?} and {second:?}");
                }
                // Distance to each side: (left, top, right, bottom).
                let d = (x, y, w - x, h - y);
                let near = |reach: f32| (d.0 < reach, d.1 < reach, d.2 < reach, d.3 < reach);
                let (el, et, er, eb) = near(EDGE);
                let in_ring = el || et || er || eb;
                let (cl, ct, cr, cb) = near(CORNER);
                let in_corner = (cl || cr) && (ct || cb);
                let Some(edge) = first else {
                    assert!(
                        !in_ring,
                        "({x}, {y}) in {w}x{h} is on the border but resizes nothing"
                    );
                    assert!(
                        !in_corner,
                        "({x}, {y}) in {w}x{h} is in a corner but resizes nothing"
                    );
                    continue;
                };
                assert!(
                    in_ring || in_corner,
                    "({x}, {y}) in {w}x{h} is inside but resizes {edge:?}"
                );
                let (l, t, r, b) = sides(edge);
                // A side within EDGE is dragged; a side beyond CORNER is not.
                for (dragged, edge_near, corner_near, side) in [
                    (l, el, cl, "left"),
                    (t, et, ct, "top"),
                    (r, er, cr, "right"),
                    (b, eb, cb, "bottom"),
                ] {
                    assert!(
                        !edge_near || dragged,
                        "({x}, {y}) {edge:?} leaves the {side} side"
                    );
                    assert!(
                        corner_near || !dragged,
                        "({x}, {y}) {edge:?} drags the far {side} side"
                    );
                }
            }
        }
    }
}

#[test]
fn a_strip_shows_the_cursor_of_its_axis() {
    for edge in ALL {
        let (l, t, r, b) = sides(edge);
        let want = match (l || r, t || b) {
            (true, false) => CursorStyle::ResizeLeftRight,
            (false, true) => CursorStyle::ResizeUpDown,
            _ if (l && t) || (r && b) => CursorStyle::ResizeUpLeftDownRight,
            _ => CursorStyle::ResizeUpRightDownLeft,
        };
        assert_eq!(cursor(edge), want, "{edge:?}");
    }
}

#[test]
fn a_tiled_side_does_not_resize() {
    for bits in 0..16u8 {
        let t = Tiling {
            left: bits & 1 != 0,
            top: bits & 2 != 0,
            right: bits & 4 != 0,
            bottom: bits & 8 != 0,
        };
        for edge in ALL {
            let (l, tp, r, b) = sides(edge);
            let blocked = (l && t.left) || (tp && t.top) || (r && t.right) || (b && t.bottom);
            assert_eq!(free(edge, t), !blocked, "{edge:?} with {t:?}");
        }
    }
}
