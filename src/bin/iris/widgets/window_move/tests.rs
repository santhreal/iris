// WHY: a title bar and a pinned image move their window from a press,
// and a double click on them zooms or closes it. Two classes are closed
// here. "A drag reads as a click": the press after a quick drag lands on
// the window-local spot the drag began at, which the platform counts as
// the drag's second click, so a second drag zoomed the window or closed
// the pin instead of moving it. "A click moves the window": travel
// within SLOP, or a press whose release the handle missed, began a move.
// Every `Double` runs every sequence; `acts` matches the enum
// exhaustively, so a new variant does not compile until it is listed.
// Not covered: the platform move and the title bar action themselves,
// which window-chrome-probe.py drives through the pointer.

use std::prelude::v1::test;

use gpui::{point, px, Pixels, Point};

use super::{Double, Press, SLOP};

const ALL: [Double; 3] = [Double::Nothing, Double::TitleBar, Double::Close];

/// Whether a double click on a handle with `double` runs an action.
fn acts(double: Double) -> bool {
    match double {
        Double::Nothing => false,
        Double::TitleBar | Double::Close => true,
    }
}

fn at(x: f32, y: f32) -> Point<Pixels> {
    point(px(x), px(y))
}

const GRAB: (f32, f32) = (40.0, 20.0);

fn grab() -> Point<Pixels> {
    at(GRAB.0, GRAB.1)
}

/// A press at the grab point, `clicks`-th of the platform's sequence,
/// released in place. Whether it was a double click.
fn click(p: &mut Press, clicks: usize, double: Double) -> bool {
    let is_double = p.down(grab(), clicks, double);
    p.up();
    is_double
}

/// A press at the grab point dragged by `(dx, dy)`: where the move
/// began, if it did.
fn drag(
    p: &mut Press,
    clicks: usize,
    double: Double,
    (dx, dy): (f32, f32),
) -> Option<Point<Pixels>> {
    p.down(grab(), clicks, double);
    // The window follows the pointer: the release lands, window-local,
    // where the press did.
    let from = p.motion(at(GRAB.0 + dx, GRAB.1 + dy), true);
    p.up();
    from
}

#[test]
fn a_second_click_in_place_is_a_double_click() {
    for double in ALL {
        let mut p = Press::default();
        assert!(!click(&mut p, 1, double), "{double:?}");
        assert_eq!(p.down(grab(), 2, double), acts(double), "{double:?}");
        // The double click's press moves nothing; any other press does.
        let from = p.motion(at(GRAB.0 + 60.0, GRAB.1), true);
        assert_eq!(from.is_none(), acts(double), "{double:?}");
    }
}

#[test]
fn the_press_after_a_drag_starts_a_new_click_sequence() {
    for double in ALL {
        let mut p = Press::default();
        assert_eq!(
            drag(&mut p, 1, double, (120.0, 80.0)),
            Some(grab()),
            "{double:?}"
        );
        // The platform counts the drag back as the second click: it is
        // a first, and it moves the window.
        assert_eq!(
            drag(&mut p, 2, double, (-120.0, -80.0)),
            Some(grab()),
            "{double:?}"
        );
        assert_eq!(
            drag(&mut p, 3, double, (0.0, 90.0)),
            Some(grab()),
            "{double:?}"
        );
        // Two clicks in place after the drags: a double click.
        assert!(!click(&mut p, 4, double), "{double:?}");
        assert_eq!(click(&mut p, 5, double), acts(double), "{double:?}");
    }
}

#[test]
fn a_click_after_a_drag_is_a_first_click() {
    for double in ALL {
        let mut p = Press::default();
        drag(&mut p, 1, double, (0.0, -30.0));
        assert!(!click(&mut p, 2, double), "{double:?}");
        assert_eq!(click(&mut p, 3, double), acts(double), "{double:?}");
        assert!(
            !click(&mut p, 4, double),
            "a third click is no double click: {double:?}"
        );
    }
}

#[test]
fn a_new_click_sequence_forgets_the_drag() {
    for double in ALL {
        let mut p = Press::default();
        drag(&mut p, 1, double, (50.0, 0.0));
        drag(&mut p, 2, double, (-50.0, 0.0));
        // The platform started over: the drags no longer count.
        assert!(!click(&mut p, 1, double), "{double:?}");
        assert_eq!(click(&mut p, 2, double), acts(double), "{double:?}");
    }
}

#[test]
fn travel_within_slop_is_a_click() {
    for double in ALL {
        let mut p = Press::default();
        p.down(grab(), 1, double);
        let d = SLOP / std::f32::consts::SQRT_2 - 0.01;
        for (dx, dy) in [
            (SLOP, 0.0),
            (0.0, -SLOP),
            (-SLOP, 0.0),
            (0.0, SLOP),
            (d, d),
            (-d, d),
        ] {
            assert_eq!(
                p.motion(at(GRAB.0 + dx, GRAB.1 + dy), true),
                None,
                "{double:?} ({dx}, {dy})"
            );
        }
        p.up();
        assert_eq!(p.down(grab(), 2, double), acts(double), "{double:?}");
    }
}

#[test]
fn travel_past_slop_moves_once_from_the_press() {
    for (dx, dy) in [
        (SLOP + 0.01, 0.0),
        (0.0, -SLOP - 0.01),
        (3.0, 3.0),
        (-300.0, 200.0),
    ] {
        let mut p = Press::default();
        p.down(grab(), 1, Double::TitleBar);
        assert_eq!(
            p.motion(at(GRAB.0 + dx, GRAB.1 + dy), true),
            Some(grab()),
            "({dx}, {dy})"
        );
        // The platform moves the window from here on.
        assert_eq!(
            p.motion(at(GRAB.0 + 2.0 * dx, GRAB.1 + 2.0 * dy), true),
            None
        );
    }
}

#[test]
fn a_release_disarms() {
    let mut p = Press::default();
    click(&mut p, 1, Double::TitleBar);
    assert_eq!(p.motion(at(GRAB.0 + 100.0, GRAB.1), true), None);
}

#[test]
fn a_release_the_handle_missed_disarms() {
    let mut p = Press::default();
    p.down(grab(), 1, Double::TitleBar);
    // The button is up in the next motion: the release went elsewhere.
    assert_eq!(p.motion(at(GRAB.0 + 1.0, GRAB.1), false), None);
    assert_eq!(p.motion(at(GRAB.0 + 100.0, GRAB.1), true), None);
}

#[test]
fn motion_without_a_press_moves_nothing() {
    let mut p = Press::default();
    assert_eq!(p.motion(at(GRAB.0 + 100.0, GRAB.1), true), None);
    assert_eq!(p, Press::default());
}
