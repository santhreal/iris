//! How far one scroll or pinch event zooms the editor's image.
//!
//! WHY: the class closed here is "a zoom step counted per event, not
//! per distance". Every scroll event zoomed one fixed step, so a
//! touchpad, which reports one gesture as dozens of small deltas, zoomed
//! many times further than a wheel turned the same distance, and a slow
//! scroll zoomed as far as a fast one. The cases split one notch, in
//! lines and in pixels, into ever more events and read that the product
//! of their factors is the one notch's step, and that a flick reporting
//! a huge delta in one event stays bounded.
//!
//! Not covered: the platform's own line and pixel units, which the
//! constants state; the anchoring of the zoom about the pointer, which
//! the X11 suite `zoom` reads from the screen.

use gpui::{point, px, ScrollDelta};

use super::ops::{pinch_zoom, wheel_zoom, MAX_NOTCHES, NOTCH_LINES, NOTCH_PX, WHEEL_STEP};

/// How far two factors may differ: f32 rounding over 64 products.
const EPS: f32 = 1e-3;

fn lines(y: f32) -> ScrollDelta {
    ScrollDelta::Lines(point(0.0, y))
}

fn pixels(y: f32) -> ScrollDelta {
    ScrollDelta::Pixels(point(px(0.0), px(y)))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

#[test]
fn one_notch_is_one_step_in_either_unit_and_direction() {
    for (delta, want) in [
        (lines(NOTCH_LINES), WHEEL_STEP),
        (lines(-NOTCH_LINES), 1.0 / WHEEL_STEP),
        (pixels(NOTCH_PX), WHEEL_STEP),
        (pixels(-NOTCH_PX), 1.0 / WHEEL_STEP),
        (lines(0.0), 1.0),
        (pixels(0.0), 1.0),
    ] {
        let got = wheel_zoom(delta);
        assert!(close(got, want), "{delta:?} zoomed by {got}, want {want}");
    }
}

#[test]
fn a_notch_split_into_many_events_zooms_as_far_as_one_notch() {
    for n in [1usize, 2, 3, 7, 16, 64] {
        for (unit, notch) in [
            (lines as fn(f32) -> ScrollDelta, NOTCH_LINES),
            (pixels as fn(f32) -> ScrollDelta, NOTCH_PX),
        ] {
            for sign in [1.0f32, -1.0] {
                let step = unit(sign * notch / n as f32);
                let got: f32 = (0..n).map(|_| wheel_zoom(step)).product();
                let want = WHEEL_STEP.powf(sign);
                assert!(
                    close(got, want),
                    "{n} events of {step:?} zoomed by {got}, one notch by {want}"
                );
            }
        }
    }
}

#[test]
fn one_event_zooms_at_most_the_bound() {
    let top = WHEEL_STEP.powf(MAX_NOTCHES);
    for (delta, want) in [
        (pixels(1e6), top),
        (pixels(-1e6), 1.0 / top),
        (lines(1e6), top),
        (lines(-1e6), 1.0 / top),
    ] {
        let got = wheel_zoom(delta);
        assert!(close(got, want), "{delta:?} zoomed by {got}, want {want}");
    }
    for (delta, want) in [
        (0.1f32, 1.1f32),
        (-0.1, 0.9),
        (0.0, 1.0),
        (50.0, top),
        (-5.0, 1.0 / top),
    ] {
        let got = pinch_zoom(delta);
        assert!(
            close(got, want),
            "a pinch of {delta} zoomed by {got}, want {want}"
        );
    }
}
