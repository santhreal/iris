use super::{
    drag_region,
    flight::{flight_at_rest, flight_rect},
    Overlay,
};
use crate::pipeline::Region;

// WHY: the committed-selection handle layout is a fixed contract:
// 4 corners then 4 edges, each centered on the point it drags. A
// reordered or off-center handle breaks resize hit-testing. Not
// covered: the GPU paint of the handles.
#[test]
fn handles_are_corners_then_edges() {
    let h = Overlay::handles(10.0, 20.0, 100.0, 50.0);
    // Corners NW NE SW SE.
    assert_eq!(h[0], (10.0, 20.0));
    assert_eq!(h[1], (110.0, 20.0));
    assert_eq!(h[2], (10.0, 70.0));
    assert_eq!(h[3], (110.0, 70.0));
    // Edges N S W E, centered on the midpoint.
    assert_eq!(h[4], (60.0, 20.0));
    assert_eq!(h[5], (60.0, 70.0));
    assert_eq!(h[6], (10.0, 45.0));
    assert_eq!(h[7], (110.0, 45.0));
}

// WHY: the capture flight hands the card to the toast once the card is
// visually at rest, not when its 500ms clock ends. A criterion that
// skips an edge (only the origin, not the size) cuts the flight while
// the card still grows; one that never holds leaves the card parked
// until the clock. Not covered: the toast's first-present timing
// (measured by the QA rig's frame trace).
#[test]
fn flight_rect_interpolates_every_edge() {
    let from = (10.0, 20.0, 100.0, 50.0);
    let to = (500.0, 400.0, 200.0, 140.0);
    assert_eq!(flight_rect(from, to, 0.0), from);
    assert_eq!(flight_rect(from, to, 1.0), to);
    assert_eq!(flight_rect(from, to, 0.5), (255.0, 210.0, 150.0, 95.0));
}

#[test]
fn flight_rests_within_half_a_pixel_on_every_edge() {
    let from = (0.0, 0.0, 100.0, 100.0);
    // (to, spring progress, at rest): each case moves one edge only.
    let cases = [
        // Left edge travels 600px: 0.6px left, then 0.3px.
        ((600.0, 0.0, 100.0, 100.0), 0.999, false),
        ((600.0, 0.0, 100.0, 100.0), 0.9995, true),
        // Top edge travels 600px upward.
        ((0.0, -600.0, 100.0, 700.0), 0.999, false),
        // Only the right edge travels (width grows by 1000px): 1px left.
        ((0.0, 0.0, 1100.0, 100.0), 0.999, false),
        ((0.0, 0.0, 1100.0, 100.0), 0.9996, true),
        // Only the bottom edge travels (height shrinks by 90px).
        ((0.0, 0.0, 100.0, 10.0), 0.99, false),
        ((0.0, 0.0, 100.0, 10.0), 0.995, true),
        // No travel: at rest from the first frame.
        (from, 0.0, true),
    ];
    for (to, e, rest) in cases {
        assert_eq!(flight_at_rest(from, to, e), rest, "to={to:?} e={e}");
    }
}

#[test]
fn flight_is_at_rest_when_its_clock_ends() {
    // A 4K-wide flight: the longest travel a real capture produces.
    let from = (0.0, 0.0, 3840.0, 2160.0);
    let to = (3620.0, 2000.0, 200.0, 113.0);
    assert!(flight_at_rest(from, to, crate::motion::spring(1.0)));
    // The rest arrives before the clock does: the handoff is early.
    let first = (0..=100)
        .map(|i| i as f32 / 100.0)
        .find(|&t| flight_at_rest(from, to, crate::motion::spring(t)))
        .expect("the spring comes to rest");
    assert!(first < 0.8, "at rest only at t={first}");
}

// WHY: the class closed here is "the committed selection differs from
// the one the drag showed": mouse move squared the rect under Shift,
// mouse up recomputed it from the raw corners and committed that. Both
// now go through `drag_region`; these pin its square lock in every
// drag direction and at the window edge. Not covered: a call site that
// bypasses `drag_region` (that needs a live overlay window).
#[test]
fn shift_drag_is_square_in_every_direction() {
    let size = (100.0, 100.0);
    // (anchor, cursor, (x, y, side))
    let cases = [
        ((10.0, 10.0), (50.0, 30.0), (10, 10, 40)), // down-right
        ((60.0, 60.0), (50.0, 20.0), (20, 20, 40)), // up-left
        ((60.0, 10.0), (30.0, 20.0), (30, 10, 30)), // down-left
        ((10.0, 60.0), (20.0, 30.0), (10, 30, 30)), // up-right
        // The window edge caps the side, keeping the square inside.
        ((80.0, 10.0), (100.0, 90.0), (80, 10, 20)),
        // A cursor past the edge is clamped before squaring.
        ((10.0, 10.0), (500.0, 30.0), (10, 10, 90)),
    ];
    for (anchor, cursor, (x, y, side)) in cases {
        assert_eq!(
            drag_region(anchor, cursor, size, true),
            Region {
                x,
                y,
                width: side,
                height: side
            },
            "anchor={anchor:?} cursor={cursor:?}"
        );
    }
}

#[test]
fn plain_drag_is_the_raw_corner_rect() {
    let r = drag_region((60.0, 60.0), (50.0, 20.0), (100.0, 100.0), false);
    assert_eq!(
        r,
        Region {
            x: 50,
            y: 20,
            width: 10,
            height: 40
        }
    );
}

#[test]
fn shift_drag_stays_square_inside_the_window_with_the_anchor_a_corner() {
    let (w, h) = (64u32, 48u32);
    for ax in (0..=w).step_by(8) {
        for ay in (0..=h).step_by(8) {
            for cx in (-16..=80).step_by(8) {
                for cy in (-16..=64).step_by(8) {
                    let (anchor, cursor) = ((ax as f32, ay as f32), (cx as f32, cy as f32));
                    let r = drag_region(anchor, cursor, (w as f32, h as f32), true);
                    let at = format!("anchor={anchor:?} cursor={cursor:?} -> {r:?}");
                    assert_eq!(r.width, r.height, "{at}");
                    assert!(r.x + r.width <= w && r.y + r.height <= h, "{at}");
                    assert!(r.x == ax || r.x + r.width == ax, "{at}");
                    assert!(r.y == ay || r.y + r.height == ay, "{at}");
                }
            }
        }
    }
}
