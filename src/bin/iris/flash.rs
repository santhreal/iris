//! Capture flash: the 180ms soft white blink at the grab moment.
//!
//! A fullscreen transparent popup with a white fill that peaks at 0.38
//! and eases out over 180ms. It never takes focus or a click, and it
//! opens after the frame is grabbed, so it never appears in a capture.
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

struct Flash {
    /// When the current blink started; `None` while stowed.
    started: Option<Instant>,
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
            flash.started = Some(Instant::now());
            crate::sys::window::show_span(window, ux, uy, uw, uh);
            cx.notify();
        });
        if shown.is_ok() {
            return Ok(());
        }
        *POOL.lock() = None;
    }
    open(cx, (ux, uy, uw, uh), Some(Instant::now()))?;
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
    match open(cx, (ux, uy, 1, 1), None) {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| crate::sys::window::stow(window));
        }
        Err(e) => iris_lib::ilog!("iris: flash warmup: {e}"),
    }
}

/// Open the flash window over `rect` (root pixels), blinking from
/// `started` or transparent when `None`, and pool it where
/// [`poolable`].
fn open(
    cx: &mut App,
    (ux, uy, uw, uh): (i32, i32, u32, u32),
    started: Option<Instant>,
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
            cx.new(|_| Flash { started })
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
        let t = self.started.map_or(1.0, |started| {
            (started.elapsed().as_secs_f32() * 1000.0 / FLASH_MS).min(1.0)
        });
        if t < 1.0 {
            window.request_animation_frame();
        } else if self.started.take().is_some() {
            if poolable() {
                crate::sys::window::stow(window);
            } else {
                window.remove_window();
            }
        }
        // Fast attack, soft decay: opacity peaks immediately, eases out.
        let opacity = PEAK * (1.0 - t) * (1.0 - t);
        div().size_full().bg(theme::alpha(theme::FG, opacity))
    }
}
