//! Scrolling. Every iris scroll container scrolls through `scroll_y`,
//! which eases mouse wheel ticks.

use gpui::{ScrollHandle, StatefulInteractiveElement};

/// `el` scrolls vertically and tracks `handle`, which the view keeps
/// across renders. A mouse wheel tick moves the offset on GPUI's wheel
/// spring over about 120 ms instead of by a whole step in one frame;
/// ticks during the motion add to its target. A touchpad scroll applies
/// at once, and under reduced motion so does a wheel tick.
///
/// clippy.toml disallows the overflow scroll methods outside this
/// function.
#[allow(clippy::disallowed_methods)] // the one caller of overflow_y_scroll
pub fn scroll_y<E: StatefulInteractiveElement>(el: E, handle: &ScrollHandle) -> E {
    handle.set_smooth_wheel(true);
    el.overflow_y_scroll().track_scroll(handle)
}

#[cfg(test)]
mod tests {
    use gpui::{div, InteractiveElement, ScrollHandle};

    // WHY: closes "a scroll container jumps a whole step per mouse wheel
    // tick". clippy.toml routes every overflow scroll through scroll_y;
    // this pins that scroll_y eases. Not caught: a container that builds
    // its handle anew each render, which scrolls nowhere at all.
    #[test]
    fn a_container_eases_wheel_ticks() {
        let handle = ScrollHandle::new();
        assert!(!handle.smooth_wheel());
        let _ = super::scroll_y(div().id("list"), &handle);
        assert!(handle.smooth_wheel());
    }
}
