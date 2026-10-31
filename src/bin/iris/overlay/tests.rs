use super::{
    bar::{bar_origin, bar_width, BAR_H, BAR_LIFT},
    drag_region,
    flight::{flight_at_rest, flight_rect},
    input::frame_rect,
    slide_rect, Overlay,
};
use crate::pipeline::Region;
use iris_lib::capture::WinRect;

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

// WHY: Space during a drag and the arrow keys move a selection at its
// size and stop it at the window's edges, and the drag's anchor moves
// by the move made, not the move asked for: an unclamped move would
// commit a rect past the screen, and an anchor moved by the asked
// delta would reshape the selection on the next pointer motion. Not
// covered: the key and pointer events, which `tests/loupe.rs` drives.
#[test]
fn slide_rect_moves_at_size_and_stops_at_the_edges() {
    let size = (1280.0, 1024.0);
    let rect = (200.0, 200.0, 200.0, 100.0);
    let cases = [
        ((100.0, 50.0), (300.0, 250.0), (100.0, 50.0)),
        ((1000.0, 700.0), (1080.0, 900.0), (880.0, 700.0)),
        ((-500.0, -10.0), (0.0, 190.0), (-200.0, -10.0)),
        ((0.0, -900.0), (200.0, 0.0), (0.0, -200.0)),
        ((0.0, 0.0), (200.0, 200.0), (0.0, 0.0)),
    ];
    for (delta, at, made) in cases {
        let (moved, d) = slide_rect(rect, delta, size);
        assert_eq!(moved, (at.0, at.1, 200.0, 100.0), "moved by {delta:?}");
        assert_eq!(d, made, "the move made for {delta:?}");
    }
    // A rect that fills the window cannot move.
    let full = (0.0, 0.0, 1280.0, 1024.0);
    assert_eq!(slide_rect(full, (5.0, 5.0), size), (full, (0.0, 0.0)));
}

// WHY: the capture flight hands the card to the toast once the card is
// visually at rest, not when its clock ends. A criterion that
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

// WHY: the class closed here is "the capture bar leaves its monitor":
// the bar sits on the monitor the session opened on, centered and
// BAR_LIFT above its bottom edge, and stays inside a monitor too short
// or too narrow for that. A bar placed from the virtual screen's origin
// instead of the monitor's lands on the wrong monitor or off the
// screen. Not covered: which monitor `bar_monitor` picks, which needs a
// live overlay window (`tests/capture_bar.rs` clicks the bar).
#[test]
fn the_capture_bar_centers_on_its_monitor_and_stays_inside_it() {
    let w = bar_width();
    let cases = [
        // A 1280x1024 monitor at the origin.
        (
            (0.0, 0.0, 1280.0, 1024.0),
            ((640.0 - w / 2.0).round(), 1024.0 - BAR_LIFT - BAR_H),
        ),
        // A monitor left of and above the primary.
        (
            (-1920.0, -200.0, 1920.0, 1080.0),
            (
                (-1920.0 + 960.0 - w / 2.0).round(),
                -200.0 + 1080.0 - BAR_LIFT - BAR_H,
            ),
        ),
        // Narrower than the bar: its left edge at the monitor's.
        (
            (100.0, 0.0, w - 40.0, 800.0),
            (100.0, 800.0 - BAR_LIFT - BAR_H),
        ),
        // Shorter than the bar and its lift: its top at the monitor's.
        ((0.0, 50.0, 1280.0, 60.0), ((640.0 - w / 2.0).round(), 50.0)),
    ];
    for (monitor, want) in cases {
        assert_eq!(bar_origin(monitor, w), want, "{monitor:?}");
    }
}

// WHY: the class closed here is "a selection crops pixels other than
// the ones under it". The frozen frame starts at the virtual screen's
// origin, which is negative on Windows and macOS with a monitor left of
// or above the primary, and the selection added that origin a second
// time: a drag on the left monitor cropped nothing, one on the primary
// cropped the left monitor, and the loupe magnified the wrong pixels. A
// monitor taken to the window by `to_logical` and back by `frame_rect`
// and `frame_pos` lands on its own pixels in the frame, at every
// origin and scale. Not covered: each platform's grab, which must start
// its frame at the same origin, and the toast's host monitor, which
// needs a live overlay window.
#[test]
fn a_monitor_maps_onto_its_own_frame_pixels_at_every_origin() {
    let mon = |x, y, width, height| WinRect {
        x,
        y,
        width,
        height,
    };
    let layouts = [
        vec![mon(0, 0, 2560, 1440)],
        // Windows and macOS: a monitor left of, or above, the primary.
        vec![mon(0, 0, 1920, 1080), mon(-1280, 200, 1280, 800)],
        vec![mon(0, 0, 1920, 1080), mon(0, -1080, 1920, 1080)],
        // X11: the left monitor off, the root left at its old size.
        vec![mon(1920, 0, 1920, 1080)],
    ];
    for sf in [1.0, 1.25, 2.0] {
        for monitors in &layouts {
            let union = WinRect::union(monitors).unwrap();
            let origin = (union.x, union.y);
            let view = (union.width, union.height);
            for m in monitors {
                let logical = Overlay::to_logical(origin, m, sf, 1.0, 1.0);
                let r = frame_rect(sf, (1.0, 1.0), view, logical);
                let want = (m.x - origin.0, m.y - origin.1);
                assert_eq!(
                    r,
                    Region {
                        x: want.0 as u32,
                        y: want.1 as u32,
                        width: m.width,
                        height: m.height,
                    },
                    "monitor at ({}, {}), scale {sf}",
                    m.x,
                    m.y
                );
                assert_eq!(
                    Overlay::frame_pos(sf, 1.0, 1.0, logical.0, logical.1),
                    (i64::from(want.0), i64::from(want.1)),
                    "monitor at ({}, {}), scale {sf}",
                    m.x,
                    m.y
                );
            }
        }
    }
}
