// WHY: a client-drawn frame rounds a corner only between two free
// sides. Closed here: a maximized or tiled window whose corners stay
// round, which shows the desktop through a transparent notch at the
// screen's corner or against the neighbor it is tiled beside, and a
// free corner beside a tiled side that turns square on the wrong axis.
// Each side is tiled alone, with its opposite, and with a neighbor.
// Not covered: the tiling a window manager reports, and the platforms
// that draw the outline themselves (`sys::window::platform_shape`),
// which take every side as tiled.

use std::prelude::v1::test;

use gpui::{px, Tiling};

use super::corners;

#[test]
fn a_corner_is_round_only_between_two_free_sides() {
    // (left, top, right, bottom) tiled -> (top left, top right, bottom
    // right, bottom left) radius.
    let cases = [
        ((false, false, false, false), [12., 12., 12., 12.]),
        ((true, false, false, false), [0., 12., 12., 0.]),
        ((false, true, false, false), [0., 0., 12., 12.]),
        ((false, false, true, false), [12., 0., 0., 12.]),
        ((false, false, false, true), [12., 12., 0., 0.]),
        ((true, true, false, false), [0., 0., 12., 0.]),
        ((false, false, true, true), [12., 0., 0., 0.]),
        ((true, false, true, false), [0., 0., 0., 0.]),
        ((false, true, false, true), [0., 0., 0., 0.]),
        ((true, true, true, true), [0., 0., 0., 0.]),
    ];
    for ((left, top, right, bottom), want) in cases {
        let t = Tiling {
            left,
            top,
            right,
            bottom,
        };
        let c = corners(t);
        assert_eq!(
            [c.top_left, c.top_right, c.bottom_right, c.bottom_left],
            want.map(px),
            "{t:?}"
        );
    }
}
