//! Capture flash: the 180ms soft white blink at the grab moment.
//!
//! A fullscreen transparent popup with a white fill that peaks at 0.38
//! on its first frame and eases out over 180ms. It never takes focus or
//! a click, and it opens after the frame is grabbed, so it never
//! appears in a capture.
//! A screen that does not blend a window's transparent pixels
//! (`sys::window::blends`) would show the fill over black on every
//! monitor, so there the flash does not open.
//!
//! On X11 the window outlives its blink: it is stowed at one pixel
//! (`sys::window::stow`) and shown again for the next capture, so a
//! capture pays no renderer init on the main thread between the grab and
//! its toast. The daemon builds it stowed at start (`warmup`).

use std::time::Instant;

use gpui::*;

use crate::theme;

const FLASH_MS: f32 = 180.0;
const PEAK: f32 = 0.38;

/// The stowed flash window, X11 only.
static POOL: parking_lot::Mutex<Option<WindowHandle<Flash>>> = parking_lot::Mutex::new(None);

/// Where a flash window is in its blink.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Blink {
    /// Stowed, or transparent after its blink.
    Rest,
    /// Shown at this instant, with no frame drawn over its span yet.
    Due(Instant),
    /// Blinking since the first frame drawn over its span.
    Since(Instant),
}

impl Blink {
    /// The blink's progress, 0 to 1, in a frame drawn at `now`, and its
    /// state after that frame. The clock starts at the first frame over
    /// the span (`spans`), so that frame is the peak however long the
    /// window took to show: a stowed window resized on another
    /// connection draws its next frame at one pixel. A window shown a
    /// blink ago starts its clock regardless.
    fn frame(self, spans: bool, now: Instant) -> (f32, Blink) {
        let ms = |since: Instant| now.saturating_duration_since(since).as_secs_f32() * 1000.0;
        match self {
            Blink::Rest => (1.0, Blink::Rest),
            Blink::Due(shown) if spans || ms(shown) >= FLASH_MS => (0.0, Blink::Since(now)),
            Blink::Due(_) => (0.0, self),
            Blink::Since(at) => ((ms(at) / FLASH_MS).min(1.0), self),
        }
    }
}

struct Flash {
    blink: Blink,
}

/// True where a stowed flash can be shown again: X11, where the pop-up
/// is an override-redirect window that maps and unmaps at once.
fn poolable() -> bool {
    iris_lib::sys::session::x11()
}

/// The union of the monitors in root pixels: x, y, width, height.
fn union() -> (i32, i32, u32, u32) {
    let mut monitors = iris_lib::sys::capture::monitors().unwrap_or_default();
    if monitors.is_empty() {
        monitors.push(iris_lib::capture::WinRect {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
        });
    }
    let (mut ux, mut uy, mut ux2, mut uy2) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for m in &monitors {
        ux = ux.min(m.x);
        uy = uy.min(m.y);
        ux2 = ux2.max(m.x + m.width as i32);
        uy2 = uy2.max(m.y + m.height as i32);
    }
    (ux, uy, (ux2 - ux).max(1) as u32, (uy2 - uy).max(1) as u32)
}

/// Blink every screen once: one window over the union of the monitors,
/// so no monitor is left out. Returns after the window is on screen; it
/// takes itself off. A screen that does not blend a transparent window
/// (`sys::window::blends`) would show the flash black over every
/// monitor, and gets no flash.
pub fn show(cx: &mut App) -> Result<(), String> {
    if !crate::sys::window::blends() {
        return Ok(());
    }
    let (ux, uy, uw, uh) = union();
    let pooled = *POOL.lock();
    if let Some(handle) = pooled {
        let shown = handle.update(cx, |flash, window, cx| {
            flash.blink = Blink::Due(Instant::now());
            crate::sys::window::show_span(window, ux, uy, uw, uh);
            cx.notify();
        });
        if shown.is_ok() {
            return Ok(());
        }
        *POOL.lock() = None;
    }
    open(cx, (ux, uy, uw, uh), Blink::Due(Instant::now()))?;
    Ok(())
}

/// Build the flash window stowed, so the first capture shows it instead
/// of paying a renderer init. It opens at one pixel and transparent, and
/// leaves the screen in the same tick. Only where [`poolable`] and the
/// screen blends.
pub fn warmup(cx: &mut App) {
    if !poolable() || POOL.lock().is_some() || !crate::sys::window::blends() {
        return;
    }
    let (ux, uy, _, _) = union();
    match open(cx, (ux, uy, 1, 1), Blink::Rest) {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| crate::sys::window::stow(window));
        }
        Err(e) => iris_lib::ilog!("iris: flash warmup: {e}"),
    }
}

/// Open the flash window over `rect` (root pixels) in `blink`, and pool
/// it where [`poolable`].
fn open(
    cx: &mut App,
    (ux, uy, uw, uh): (i32, i32, u32, u32),
    blink: Blink,
) -> Result<WindowHandle<Flash>, String> {
    let s = crate::sys::window::root_scale(cx);
    let handle = crate::widgets::open_window(
        cx,
        "Flash - iris",
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                // Monitors are physical pixels; bounds are logical.
                origin: point(px(ux as f32 / s), px(uy as f32 / s)),
                size: size(px(uw as f32 / s), px(uh as f32 / s)),
            })),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        },
        |window, cx| {
            // A click during the blink reaches the window beneath.
            crate::sys::window::pass_input(window);
            cx.new(|_| Flash { blink })
        },
    )
    .map_err(|e| format!("open flash window: {e}"))?;
    if poolable() {
        *POOL.lock() = Some(handle);
    }
    Ok(handle)
}

impl Render for Flash {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let spans = window.viewport_size().width > px(1.0);
        let (t, blink) = self.blink.frame(spans, Instant::now());
        self.blink = blink;
        if t < 1.0 {
            window.request_animation_frame();
        } else if !matches!(std::mem::replace(&mut self.blink, Blink::Rest), Blink::Rest) {
            if poolable() {
                crate::sys::window::stow(window);
            } else {
                window.remove_window();
            }
        }
        // Fast attack, soft decay: the first frame is the peak, which
        // eases out.
        let opacity = PEAK * (1.0 - t) * (1.0 - t);
        div().size_full().bg(theme::alpha(theme::FG, opacity))
    }
}

// WHY: closes "a flash whose clock starts before its first frame over
// the screen". The clock started when a capture showed the window, so
// the time the window took to resize and draw came off the blink: on a
// software renderer at 2x the first frame over the screen showed 0.23 of
// the 0.38 peak. The cases step a blink through frames at set instants:
// a frame at one pixel keeps it due at the peak, the first frame over
// the span starts the clock at the peak, the decay runs from there, and
// a window that never spans still ends a blink after a blink. Not
// covered: the window's size in a real frame, which `tests/popups.rs`
// reads on X11 with the first frame's opacity.
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::Blink;

    #[test]
    fn a_blink_starts_at_its_first_frame_over_its_span() {
        let shown = Instant::now();
        let at = |ms| shown + Duration::from_millis(ms);
        let (t, blink) = Blink::Due(shown).frame(false, at(30));
        assert_eq!((t, blink), (0.0, Blink::Due(shown)), "a one-pixel frame");
        let (t, blink) = blink.frame(true, at(60));
        assert_eq!(
            (t, blink),
            (0.0, Blink::Since(at(60))),
            "the first frame over the span"
        );
        let (t, blink) = blink.frame(true, at(150));
        assert!((t - 0.5).abs() < 1e-4, "90 ms into the blink: t {t}");
        assert_eq!(blink.frame(true, at(240)).0, 1.0, "180 ms into the blink");
        // A window that never spans: due a blink, then a blink more.
        let (t, blink) = Blink::Due(shown).frame(false, at(180));
        assert_eq!(
            (t, blink),
            (0.0, Blink::Since(at(180))),
            "a blink after the show"
        );
        assert_eq!(
            blink.frame(false, at(360)).0,
            1.0,
            "a blink after its clock started"
        );
        assert_eq!(
            Blink::Rest.frame(true, at(5)),
            (1.0, Blink::Rest),
            "at rest"
        );
    }
}
