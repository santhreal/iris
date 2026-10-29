//! WHY: a pin opened at one fixed corner whatever it showed, and every
//! pin after the first opened on the same pixel, hiding the pins under
//! it. A pin opens centered on the rect its pixels came from, on a whole
//! pixel, and one that would open on another pin's origin steps down
//! and right until it is alone. Not covered here: reading the open pins'
//! origins and the rect a capture hands over, which `tests/pin.rs`
//! drives through a region capture.

use super::{place, zoom_range, CASCADE, MIN_EDGE, UNPLACED, ZOOM_MAX};

#[test]
fn a_pin_opens_centered_on_its_rect() {
    let cases = [
        // Native scale: the pin covers its rect.
        ((320.0, 240.0), (200.0, 200.0, 320.0, 240.0), (200.0, 200.0)),
        // Scaled to the 720x540 cap: centered in its rect.
        ((720.0, 405.0), (0.0, 0.0, 1920.0, 1080.0), (600.0, 338.0)),
        // A monitor left of and above the primary one.
        (
            (100.0, 50.0),
            (-1280.0, -200.0, 100.0, 50.0),
            (-1280.0, -200.0),
        ),
        // An odd difference lands on a whole pixel.
        ((101.0, 51.0), (10.0, 10.0, 200.0, 100.0), (60.0, 35.0)),
    ];
    for (size, over, want) in cases {
        assert_eq!(place(size, Some(over), &[]), want, "{size:?} over {over:?}");
    }
    assert_eq!(place((320.0, 240.0), None, &[]), UNPLACED);
}

#[test]
fn a_pin_on_another_pins_origin_cascades_until_alone() {
    let over = Some((200.0, 200.0, 320.0, 240.0));
    let size = (320.0, 240.0);
    let step = |n: f32| (200.0 + n * CASCADE, 200.0 + n * CASCADE);
    // Unrelated pins leave the origin alone.
    assert_eq!(place(size, over, &[(0.0, 0.0), (500.0, 200.0)]), step(0.0));
    // A pin under it, and one moved off by under a pixel.
    assert_eq!(place(size, over, &[step(0.0)]), step(1.0));
    assert_eq!(place(size, over, &[(200.4, 199.6)]), step(1.0));
    // A chain in any order ends past its last member.
    let chain = [step(2.0), step(0.0), step(1.0)];
    assert_eq!(place(size, over, &chain), step(3.0));
    // A gap in the chain is taken.
    assert_eq!(place(size, over, &[step(0.0), step(2.0)]), step(1.0));
    // Many pins on one origin: one step clears them all.
    assert_eq!(place(size, over, &[step(0.0); 5]), step(1.0));
    assert_eq!(
        place(size, None, &[UNPLACED]),
        (80.0 + CASCADE, 80.0 + CASCADE)
    );
}

/// WHY: a pin zooms between a floor where it is still a window to
/// grab and a ceiling it can still be drawn and seen at. The cases pin
/// each bound: the shorter edge for the floor, native scale for an
/// image under it, ZOOM_MAX or the screen's narrower ratio for the
/// ceiling, and the floor where the screen is smaller still. Not
/// covered: the window resize, which `tests/pin.rs` reads.
#[test]
fn a_pins_zoom_stays_between_its_floor_and_ceiling() {
    let screen = Some((1920.0, 1080.0));
    let cases = [
        // A small capture: its shorter edge floors it, ZOOM_MAX tops it.
        ((320.0, 240.0), screen, (MIN_EDGE / 240.0, ZOOM_MAX)),
        // Wide and tall captures: the screen's narrower ratio tops it.
        ((1280.0, 720.0), screen, (MIN_EDGE / 720.0, 1.5)),
        ((400.0, 900.0), screen, (MIN_EDGE / 400.0, 1.2)),
        // A capture larger than the screen zooms only down.
        ((3840.0, 2160.0), screen, (MIN_EDGE / 2160.0, 0.5)),
        // An image under the floor stays at native scale or larger.
        ((20.0, 30.0), screen, (1.0, ZOOM_MAX)),
        // No display: ZOOM_MAX.
        ((320.0, 240.0), None, (MIN_EDGE / 240.0, ZOOM_MAX)),
        // A screen smaller than the floor: the floor wins.
        ((20.0, 30.0), Some((10.0, 10.0)), (1.0, 1.0)),
    ];
    for (native, screen, want) in cases {
        assert_eq!(zoom_range(native, screen), want, "{native:?} on {screen:?}");
    }
}
