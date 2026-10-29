//! WHY: a pin opened at one fixed corner whatever it showed, and every
//! pin after the first opened on the same pixel, hiding the pins under
//! it. A pin opens centered on the rect its pixels came from, on a whole
//! pixel, and one that would open on another pin's origin steps down
//! and right until it is alone. Not covered here: reading the open pins'
//! origins and the rect a capture hands over, which `tests/pin.rs`
//! drives through a region capture.

use super::{place, CASCADE, UNPLACED};

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
