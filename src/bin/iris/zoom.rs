//! Zoom steps for the scroll wheel, a touchpad pinch, and the `+` and
//! `-` keys, in the editor and in a pin.

use gpui::ScrollDelta;

/// The zoom factor of one `+` or `-` press.
pub const KEY_STEP: f32 = 1.25;

/// The zoom factor of one wheel notch.
pub const WHEEL_STEP: f32 = 1.12;

/// The line scroll of one wheel notch: gpui reports a notch as three
/// lines on Linux and Windows (the Windows default) and as one line on
/// macOS.
#[cfg(target_os = "macos")]
pub const NOTCH_LINES: f32 = 1.0;
#[cfg(not(target_os = "macos"))]
pub const NOTCH_LINES: f32 = 3.0;

/// The pixel scroll that counts as one notch, for a touchpad that
/// reports pixels.
pub const NOTCH_PX: f32 = 60.0;

/// The most notches one scroll event can zoom by: a momentum flick
/// that reports hundreds of pixels at once stays a few steps.
pub const MAX_NOTCHES: f32 = 3.0;

/// The zoom factor for one scroll event. A whole notch is one
/// `WHEEL_STEP` and a fraction of a notch is that fraction of a step,
/// so a touchpad, which reports a gesture as many small deltas, zooms
/// by the distance the fingers travel rather than by the event count.
pub fn wheel_zoom(delta: ScrollDelta) -> f32 {
    let notches = match delta {
        ScrollDelta::Pixels(p) => f32::from(p.y) / NOTCH_PX,
        ScrollDelta::Lines(p) => p.y / NOTCH_LINES,
    };
    WHEEL_STEP.powf(notches.clamp(-MAX_NOTCHES, MAX_NOTCHES))
}

/// The zoom factor for one pinch event: `delta` is the fractional
/// change, 0.1 for a 10% increase.
pub fn pinch_zoom(delta: f32) -> f32 {
    (1.0 + delta).clamp(WHEEL_STEP.powf(-MAX_NOTCHES), WHEEL_STEP.powf(MAX_NOTCHES))
}

#[cfg(test)]
mod tests;
