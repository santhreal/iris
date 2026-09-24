//! WHY: the class closed here is "the chip lands where it hides or gets
//! recorded": inside a rect it could clear (on X11 a region recording
//! then contains the chip), on a monitor the rect is not on, past the
//! edge of its monitor, or over a strip no monitor shows. The chip's
//! open and the X11 follower both place through `origin`, so these
//! cases hold for every placement. Not covered: a window manager moving
//! the chip after placement, and monitors that change mid-recording.

use iris_lib::capture::WinRect;

use super::{origin, WIN_H, WIN_W};

const fn rect(x: i32, y: i32, width: u32, height: u32) -> WinRect {
    WinRect {
        x,
        y,
        width,
        height,
    }
}

/// Left monitor 1000x1000 at the origin (primary); right monitor
/// 600x700 at (1000, 300), leaving a 600x300 strip above it that is on
/// no monitor.
const MONITORS: [WinRect; 2] = [rect(0, 0, 1000, 1000), rect(1000, 300, 600, 700)];

#[test]
fn above_the_rect_when_its_monitor_has_room() {
    assert_eq!(
        origin(Some(rect(300, 300, 500, 400)), &MONITORS, 1.0),
        (800.0 - WIN_W, 300.0 - WIN_H)
    );
}

#[test]
fn room_above_is_exact_at_the_boundary() {
    let top = WIN_H as i32;
    assert_eq!(
        origin(Some(rect(300, top, 500, 400)), &MONITORS, 1.0),
        (800.0 - WIN_W, 0.0)
    );
    // One pixel less and the chip no longer fits above: it goes below.
    assert_eq!(
        origin(Some(rect(300, top - 1, 500, 400)), &MONITORS, 1.0),
        (800.0 - WIN_W, (top - 1 + 400) as f32)
    );
}

#[test]
fn below_the_rect_at_the_top_edge() {
    assert_eq!(
        origin(Some(rect(300, 0, 500, 400)), &MONITORS, 1.0),
        (800.0 - WIN_W, 400.0)
    );
}

#[test]
fn room_below_is_exact_at_the_boundary() {
    let bottom = 1000 - WIN_H as u32;
    assert_eq!(
        origin(Some(rect(300, 0, 500, bottom)), &MONITORS, 1.0),
        (800.0 - WIN_W, bottom as f32)
    );
    // One pixel taller and neither side fits: inside the top edge.
    assert_eq!(
        origin(Some(rect(300, 0, 500, bottom + 1)), &MONITORS, 1.0),
        (800.0 - WIN_W, 0.0)
    );
}

#[test]
fn inside_the_top_edge_when_neither_side_fits() {
    assert_eq!(
        origin(Some(rect(100, 0, 800, 1000)), &MONITORS, 1.0),
        (900.0 - WIN_W, 0.0)
    );
}

#[test]
fn a_rect_past_its_monitor_top_keeps_the_chip_on_the_monitor() {
    assert_eq!(
        origin(Some(rect(100, -50, 800, 1100)), &MONITORS, 1.0),
        (900.0 - WIN_W, 0.0)
    );
}

#[test]
fn stays_on_the_monitor_horizontally() {
    // Narrower than the chip at the right edge: the chip shifts left
    // instead of spilling onto the next monitor.
    assert_eq!(
        origin(Some(rect(900, 400, 90, 200)), &MONITORS, 1.0).0,
        1000.0 - WIN_W
    );
    // Narrower than the chip at the left edge: the chip starts at it.
    assert_eq!(origin(Some(rect(0, 400, 90, 200)), &MONITORS, 1.0).0, 0.0);
}

#[test]
fn a_monitor_narrower_than_the_chip_pins_it_to_its_left_edge() {
    // A 100px-wide monitor right of the primary: the chip overhangs its
    // right edge instead of spilling left onto the primary.
    let monitors = [rect(0, 0, 1000, 1000), rect(1000, 0, 100, 1000)];
    const { assert!(WIN_W > 100.0) };
    assert_eq!(
        origin(Some(rect(1010, 400, 50, 200)), &monitors, 1.0).0,
        1000.0
    );
}

#[test]
fn the_rect_monitor_bounds_the_chip_not_the_primary() {
    // Root space has room above this rect, but on no monitor: the chip
    // goes below it, on the right monitor.
    assert_eq!(
        origin(Some(rect(1100, 300, 400, 300)), &MONITORS, 1.0),
        (1500.0 - WIN_W, 600.0)
    );
}

#[test]
fn a_rect_centered_on_no_monitor_uses_the_primary() {
    assert_eq!(
        origin(Some(rect(1100, 0, 400, 200)), &MONITORS, 1.0),
        (1000.0 - WIN_W, 200.0)
    );
}

#[test]
fn no_rect_puts_the_chip_inside_the_primary_top_right() {
    assert_eq!(origin(None, &MONITORS, 1.0), (1000.0 - WIN_W, 0.0));
}

#[test]
fn scale_maps_root_pixels_to_logical() {
    let monitors = [rect(0, 0, 2000, 2000)];
    assert_eq!(
        origin(Some(rect(600, 600, 1000, 800)), &monitors, 2.0),
        (800.0 - WIN_W, 300.0 - WIN_H)
    );
}

#[test]
fn without_monitors_the_rect_bounds_the_chip() {
    assert_eq!(
        origin(Some(rect(300, 300, 500, 400)), &[], 1.0),
        (800.0 - WIN_W, 300.0)
    );
    assert_eq!(origin(None, &[], 1.0), (0.0, 0.0));
}

/// WHY: a shadow that reaches past the chip window's bleed is cut off at
/// the window edge, a hard-edged gray rectangle around the pill over any
/// light content. Every shadow the pill paints must end inside
/// CHIP_BLEED on every side. Not covered: shadows painted by the pill's
/// children.
#[test]
fn every_pill_shadow_ends_inside_the_window() {
    let reach = crate::theme::shadow_reach(&super::pill_shadow());
    assert!(
        reach <= super::CHIP_BLEED,
        "the pill shadow reaches {reach}px past the pill; the window leaves {}px",
        super::CHIP_BLEED
    );
}
