// WHY: a resize from one edge moves only that edge. Two classes are
// closed here. "An edge the drag does not name moves": a corner that
// drags one side, or an edge that drags both. "The opposite edge
// travels": a left or top drag clamped at the minimum keeps moving the
// origin, so the window slides instead of stopping. Every ResizeEdge is
// run; `sides` matches the enum exhaustively, so a new variant does not
// compile until it is listed. Not covered: the window manager honoring
// the configure request, which window-chrome-probe.py drives.

use std::prelude::v1::test;

use gpui::ResizeEdge::{self, *};

use super::{resized, Rect};

const START: Rect = Rect {
    x: 100,
    y: 50,
    w: 800,
    h: 600,
};
const MIN: (i32, i32) = (640, 480);
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

/// `r` as (left, top, right, bottom).
fn edges(r: Rect) -> (i32, i32, i32, i32) {
    (r.x, r.y, r.x + r.w, r.y + r.h)
}

#[test]
fn a_dragged_edge_follows_the_pointer_and_the_others_hold() {
    let (l, t, r, b) = edges(START);
    for edge in ALL {
        let (dl, dt, dr, db) = sides(edge);
        for (dx, dy) in [(-50, -30), (70, 40), (0, 0), (-3, 9)] {
            let want = (
                if dl { l + dx } else { l },
                if dt { t + dy } else { t },
                if dr { r + dx } else { r },
                if db { b + dy } else { b },
            );
            let got = edges(resized(START, edge, (dx, dy), (1, 1)));
            assert_eq!(got, want, "{edge:?} by ({dx}, {dy})");
        }
    }
}

#[test]
fn a_size_stops_at_the_minimum_and_the_opposite_edge_holds() {
    let (l, t, r, b) = edges(START);
    for edge in ALL {
        let (dl, dt, dr, db) = sides(edge);
        // Far inward on every side the edge drags.
        let dx = if dl {
            10_000
        } else if dr {
            -10_000
        } else {
            0
        };
        let dy = if dt {
            10_000
        } else if db {
            -10_000
        } else {
            0
        };
        let out = resized(START, edge, (dx, dy), MIN);
        let (ol, ot, or, ob) = edges(out);
        let want_w = if dl || dr { MIN.0 } else { START.w };
        let want_h = if dt || db { MIN.1 } else { START.h };
        assert_eq!((out.w, out.h), (want_w, want_h), "{edge:?}");
        if dl {
            assert_eq!(or, r, "{edge:?} moved the right edge");
        }
        if dr {
            assert_eq!(ol, l, "{edge:?} moved the left edge");
        }
        if dt {
            assert_eq!(ob, b, "{edge:?} moved the bottom edge");
        }
        if db {
            assert_eq!(ot, t, "{edge:?} moved the top edge");
        }
    }
}
