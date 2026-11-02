use std::prelude::v1::test;

use gpui::CursorStyle;

use super::*;

// WHY: the handle layout is a fixed contract the overlay and the crop
// share: 4 corners then 4 edges, each centered on the point it drags.
// A reordered or off-center handle breaks hit-testing, the cursor, and
// the side a drag moves. Not covered: the GPU paint of the handles.
#[test]
fn handles_are_corners_then_edges() {
    assert_eq!(
        handles(10.0, 20.0, 100.0, 50.0),
        [
            (10.0, 20.0),
            (110.0, 20.0),
            (10.0, 70.0),
            (110.0, 70.0),
            (60.0, 20.0),
            (60.0, 70.0),
            (10.0, 45.0),
            (110.0, 45.0),
        ]
    );
}

// WHY: the class closed here is "a handle drags a side it does not
// show". Each handle moves exactly the sides its position names and
// leaves the others where they were, a side dragged past the opposite
// one turns the rect over with a positive size, and a handle dragged
// outside the bounds stops the side at them rather than growing the
// rect past them or moving the far side. Not covered: the pointer
// events that call it.
#[test]
fn a_handle_moves_the_sides_it_sits_on_and_stays_in_bounds() {
    let rect = (100.0, 100.0, 200.0, 100.0);
    let bounds = (600.0, 400.0);
    // Each handle dragged 10 px out from the rect, along both axes.
    let out = [
        (90.0, 90.0, 210.0, 110.0),
        (100.0, 90.0, 210.0, 110.0),
        (90.0, 100.0, 210.0, 110.0),
        (100.0, 100.0, 210.0, 110.0),
        (100.0, 90.0, 200.0, 110.0),
        (100.0, 100.0, 200.0, 110.0),
        (90.0, 100.0, 210.0, 100.0),
        (100.0, 100.0, 210.0, 100.0),
    ];
    for (h, want) in out.into_iter().enumerate() {
        let (hx, hy) = handles(rect.0, rect.1, rect.2, rect.3)[h];
        let dx = if hx == 100.0 { -10.0 } else { 10.0 };
        let dy = if hy == 100.0 { -10.0 } else { 10.0 };
        assert_eq!(
            reshape(h, rect, (hx + dx, hy + dy), bounds),
            want,
            "handle {h}"
        );
    }
    // The SE corner past the NW one turns the rect over.
    assert_eq!(
        reshape(3, rect, (40.0, 60.0), bounds),
        (40.0, 60.0, 60.0, 40.0)
    );
    // The W edge past the E one.
    assert_eq!(
        reshape(6, rect, (350.0, 0.0), bounds),
        (300.0, 100.0, 50.0, 100.0)
    );
    // Past the bounds: the side stops there, the far side stays.
    assert_eq!(
        reshape(0, rect, (-50.0, -80.0), bounds),
        (0.0, 0.0, 300.0, 200.0)
    );
    assert_eq!(
        reshape(3, rect, (900.0, 700.0), bounds),
        (100.0, 100.0, 500.0, 300.0)
    );
}

// WHY: a press lands on the handle whose square holds it, the corner
// where squares overlap on a small rect, and nowhere past the square's
// edge. A hit test that favors an edge drags one side of a tiny crop
// from its corner; one with the wrong extent grabs a handle from the
// rect's middle or misses it at the square's rim.
#[test]
fn a_press_takes_the_handle_whose_square_holds_it() {
    let rect = (100.0, 100.0, 200.0, 100.0);
    let half = HANDLE_PX / 2.0;
    for (h, &(hx, hy)) in handles(100.0, 100.0, 200.0, 100.0).iter().enumerate() {
        for p in [(hx, hy), (hx - half, hy - half), (hx + half, hy + half)] {
            assert_eq!(handle_at(rect, p, half), Some(h), "{p:?}");
        }
        assert_eq!(handle_at(rect, (hx + half + 0.5, hy), half), None);
        assert_eq!(handle_at(rect, (hx, hy - half - 0.5), half), None);
    }
    assert_eq!(handle_at(rect, (200.0, 150.0), half), None);
    // A rect smaller than a square: every handle overlaps the NW one.
    assert_eq!(
        handle_at((100.0, 100.0, 6.0, 6.0), (103.0, 103.0), half),
        Some(0)
    );
}

// WHY: the class closed here is "handles crowd a small rect": on a side
// shorter than two hit squares, the handle at its middle overlapped the
// corners', covered the rect under eight dots, and took presses meant
// for a corner. Over a grid of sizes around the threshold, every corner
// shows, a side's middle handle shows exactly when its square clears
// both corners' squares, and a press never takes a handle that is not
// drawn. Not covered: the GPU paint of the handles.
#[test]
fn a_short_side_shows_no_middle_handle() {
    let half = HANDLE_PX / 2.0;
    let sizes = [0.0, 1.0, 6.0, 16.0, 31.5, 32.0, 33.0, 64.0, 200.0];
    for w in sizes {
        for h in sizes {
            let all = handles(10.0, 20.0, w, h);
            let shown: Vec<usize> = shown_handles(10.0, 20.0, w, h).map(|(i, _)| i).collect();
            for i in 0..8 {
                let corner = i < 4;
                let clears = all[..4].iter().all(|c| {
                    (c.0 - all[i].0).abs() >= HANDLE_PX || (c.1 - all[i].1).abs() >= HANDLE_PX
                });
                assert_eq!(shown.contains(&i), corner || clears, "{w}x{h} handle {i}");
                let at = handle_at((10.0, 20.0, w, h), all[i], half);
                assert!(
                    at.is_none_or(|j| shown.contains(&j)),
                    "{w}x{h}: a press on handle {i} takes hidden handle {at:?}"
                );
            }
        }
    }
}

// WHY: the arrow over a handle points along the sides it drags, read
// from the drag itself: a handle that moves one side shows a straight
// arrow on that axis, and a corner the diagonal through it and the
// rect's center. A corner under a straight arrow, an edge under a
// diagonal one, or a corner under the other diagonal promises a drag
// the handle does not make.
#[test]
fn a_handle_shows_the_arrow_of_its_drag() {
    let rect = (50.0, 50.0, 100.0, 100.0);
    let center = (100.0, 100.0);
    for (h, &(hx, hy)) in handles(rect.0, rect.1, rect.2, rect.3).iter().enumerate() {
        let (x, y, w, ht) = reshape(h, rect, (hx - 20.0, hy - 20.0), (300.0, 300.0));
        let sideways = (x, w) != (rect.0, rect.2);
        let upward = (y, ht) != (rect.1, rect.3);
        // NW and SE: left of the center exactly when above it.
        let falling = (hx < center.0) == (hy < center.1);
        let want = match (sideways, upward) {
            (true, false) => CursorStyle::ResizeLeftRight,
            (false, true) => CursorStyle::ResizeUpDown,
            _ if falling => CursorStyle::ResizeUpLeftDownRight,
            _ => CursorStyle::ResizeUpRightDownLeft,
        };
        assert_eq!(handle_cursor(h), want, "handle {h}");
    }
}

// WHY: the class closed here is "a drag shows another cursor than the
// one it runs". A pan, the crop's move, and a committed selection's
// move showed the tool's crosshair, which promises a fresh drawing,
// and Space held to pan showed it too. Every state of a resize, a move,
// and a wait: a resize shows its handle's arrow over a move or a wait,
// a move the closed hand over a wait, a wait the open hand, and none of
// them no cursor of its own. Not covered: which element shows the
// cursor; `tests/cursor.rs` reads it on screen.
#[test]
fn a_drag_shows_its_own_cursor_and_a_resize_wins() {
    for resizing in [
        None,
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(5),
        Some(6),
        Some(7),
    ] {
        for moving in [false, true] {
            for waiting in [false, true] {
                let want = match (resizing, moving, waiting) {
                    (Some(0 | 3), ..) => Some(CursorStyle::ResizeUpLeftDownRight),
                    (Some(1 | 2), ..) => Some(CursorStyle::ResizeUpRightDownLeft),
                    (Some(4 | 5), ..) => Some(CursorStyle::ResizeUpDown),
                    (Some(_), ..) => Some(CursorStyle::ResizeLeftRight),
                    (None, true, _) => Some(CursorStyle::ClosedHand),
                    (None, false, true) => Some(CursorStyle::OpenHand),
                    (None, false, false) => None,
                };
                assert_eq!(
                    grab_cursor(resizing, moving, waiting),
                    want,
                    "resizing {resizing:?}, moving {moving}, waiting {waiting}"
                );
            }
        }
    }
}
