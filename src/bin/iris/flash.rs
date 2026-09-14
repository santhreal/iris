//! Capture flash: the 180ms soft white blink at the grab moment.
//!
//! A fullscreen transparent popup with a white fill that peaks at 0.38
//! and eases out over 180ms. It never takes focus, and it is gone
//! before the frozen frame is grabbed, so it never appears in a
//! capture.

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
    #[cfg(target_os = "linux")]
    let monitors = iris_lib::capture::x11::monitors().unwrap_or_default();
    #[cfg(not(target_os = "linux"))]
    let monitors: Vec<iris_lib::capture::WinRect> = Vec::new();
    let mut monitors = monitors;
    if monitors.is_empty() {
        monitors.push(iris_lib::capture::WinRect {
            x: 0,
            y: 0,
            width: 1280,
            height: 800,
        });
    }
    let mut first_err = None;
    for m in monitors {
        let win_id = crate::xwin::unique_id("dev.iris.flash");
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Fullscreen(Bounds {
                    origin: point(px(m.x as f32), px(m.y as f32)),
                    size: size(px(m.width as f32), px(m.height as f32)),
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
                app_id: Some(win_id.clone()),
                window_min_size: None,
                window_decorations: Some(WindowDecorations::Client),
                tabbing_identifier: None,
            },
            |_, cx| {
                cx.new(|_| Flash {
                    started: Instant::now(),
                })
            },
        );
        if let Err(e) = result {
            first_err = Some(format!("open flash window: {e}"));
        } else {
            crate::xwin::fullscreen_after_map(win_id, m.x as f32, m.y as f32);
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Total wall time of the blink; the caller waits this long before
/// grabbing the frame so the flash is never captured.
pub const fn duration_ms() -> u64 {
    FLASH_MS as u64 + 40
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
        div()
            .size_full()
            .bg(theme::alpha(theme::FG, opacity))
    }
}
