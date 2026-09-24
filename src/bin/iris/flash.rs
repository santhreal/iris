//! Capture flash: the 180ms soft white blink at the grab moment.
//!
//! A fullscreen transparent popup with a white fill that peaks at 0.38
//! and eases out over 180ms. It never takes focus, and it opens after
//! the frame is grabbed, so it never appears in a capture.

use std::time::Instant;

use gpui::*;

use crate::theme;

const FLASH_MS: f32 = 180.0;
const PEAK: f32 = 0.38;

struct Flash {
    started: Instant,
}

/// Blink every screen once: one window per monitor, each placed
/// and fullscreened by the post-map helper so no monitor is left
/// out. Returns after the windows open; they dismiss themselves.
pub fn show(cx: &mut App) -> Result<(), String> {
    let mut monitors = iris_lib::capture::monitors().unwrap_or_default();
    if monitors.is_empty() {
        monitors.push(iris_lib::capture::WinRect {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
        });
    }
    // One window spanning the monitor union: the flash is a uniform
    // fill, so a single window covers every screen and pays one
    // renderer init instead of one per monitor. The overlay uses the
    // same span for its frozen frame.
    let (mut ux, mut uy, mut ux2, mut uy2) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for m in &monitors {
        ux = ux.min(m.x);
        uy = uy.min(m.y);
        ux2 = ux2.max(m.x + m.width as i32);
        uy2 = uy2.max(m.y + m.height as i32);
    }
    let (uw, uh) = ((ux2 - ux).max(1) as u32, (uy2 - uy).max(1) as u32);
    let s = crate::sys::window::root_scale(cx);
    cx.open_window(
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
            app_id: Some("dev.iris.flash".to_string()),
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
        },
        |window, cx| {
            crate::sys::window::span_after_map(window, ux, uy, uw, uh);
            cx.new(|_| Flash {
                started: Instant::now(),
            })
        },
    )
    .map_err(|e| format!("open flash window: {e}"))?;
    Ok(())
}

impl Render for Flash {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let t = (self.started.elapsed().as_secs_f32() * 1000.0 / FLASH_MS).min(1.0);
        if t >= 1.0 {
            window.remove_window();
        } else {
            window.request_animation_frame();
        }
        // Fast attack, soft decay: opacity peaks immediately, eases out.
        let opacity = PEAK * (1.0 - t) * (1.0 - t);
        div().size_full().bg(theme::alpha(theme::FG, opacity))
    }
}
