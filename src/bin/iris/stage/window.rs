//! Toast stage window management, sizing, and lifecycle.

use std::{path::Path, sync::Arc, time::Instant};


use super::*;
use crate::motion;

/// The live toast, if any. A new capture replaces the floating
/// thumbnail, like macOS: the previous one fades out fast.
pub(super) static TOAST_HANDLE: parking_lot::Mutex<Option<WindowHandle<ToastStage>>> =
    parking_lot::Mutex::new(None);

/// The toast card's resting rect on a display of `disp_w`x`disp_h`
/// logical px, for a capture of `img_w`x`img_h`. The overlay's
/// capture flight lands exactly here, so the toast must not replay
/// its entrance.
pub fn card_rest_rect(disp_w: f32, disp_h: f32, img_w: u32, img_h: u32) -> (f32, f32, f32, f32) {
    let (iw, ih) = (img_w as f32, img_h as f32);
    let scale = (MAX_W / iw).min(MAX_H / ih).min(1.0);
    let w = (iw * scale).round().max(1.0);
    let h = (ih * scale).round().max(1.0);
    let cfg = iris_lib::config::Config::load();
    match cfg.toast_position {
        iris_lib::config::ToastPosition::BottomRight => {
            (disp_w - MARGIN - w, disp_h - MARGIN - h, w, h)
        }
        iris_lib::config::ToastPosition::BottomLeft => (MARGIN, disp_h - MARGIN - h, w, h),
        iris_lib::config::ToastPosition::TopRight => (disp_w - MARGIN - w, MARGIN, w, h),
        iris_lib::config::ToastPosition::TopLeft => (MARGIN, MARGIN, w, h),
    }
}

/// Open the toast stage window in the bottom-right corner of the primary
/// display. The card image paths must already exist on disk.
pub fn show_toast(
    cx: &mut App,
    path: &Path,
    thumb: &Path,
    width: u32,
    height: u32,
) -> Result<(), String> {
    show_toast_kind(cx, path, thumb, width, height, false, None)
}

/// The capture-flight landing: the card appears at rest, no entrance.
/// `anchor` is the committing monitor's rect in screen coordinates, so
/// the card lands where the flight ended, not on another display.
pub fn show_toast_landed(
    cx: &mut App,
    path: &Path,
    thumb: &Path,
    width: u32,
    height: u32,
    anchor: Option<(f32, f32, f32, f32)>,
) -> Result<(), String> {
    show_toast_kind(cx, path, thumb, width, height, true, anchor)
}

fn show_toast_kind(
    cx: &mut App,
    path: &Path,
    thumb: &Path,
    width: u32,
    height: u32,
    landed: bool,
    anchor: Option<(f32, f32, f32, f32)>,
) -> Result<(), String> {
    let _ = (width, height);
    let path = path.to_path_buf();
    let thumb = thumb.to_path_buf();
    // The PNG decode + Lanczos resize is too slow for the UI thread on
    // a full-size capture; the window opens once the pixels are ready.
    let decoded = cx.background_executor().spawn({
        let thumb = thumb.clone();
        async move { prepare_thumb(&thumb) }
    });
    cx.spawn(async move |cx| {
        let parts = match decoded.await {
            Ok(p) => p,
            Err(e) => {
                iris_lib::ilog!("toast: {e}");
                return;
            }
        };
        let _ = cx.update(|cx| {
            if let Err(e) = open_toast_window(cx, &path, parts, landed, anchor) {
                iris_lib::ilog!("toast: {e}");
            }
        });
    })
    .detach();
    Ok(())
}

fn open_toast_window(
    cx: &mut App,
    path: &Path,
    parts: (Arc<RenderImage>, Arc<Vec<u8>>, (f32, f32)),
    landed: bool,
    anchor: Option<(f32, f32, f32, f32)>,
) -> Result<(), String> {
    let mut stage = ToastStage::from_parts(path, parts.0, parts.1, parts.2);
    if landed {
        // Pretend the entrance finished long ago.
        stage.opened = Some(Instant::now() - motion::tempo(ENTER));
    }
    let (w, h) = stage.dims;

    let cfg = iris_lib::config::Config::load();
    let win_size = size(px(w + BLEED + MARGIN), px(h + BLEED + MARGIN));
    let screen = anchor.or_else(|| crate::sys::window::primary_monitor_rect(cx));
    let origin = match screen {
        Some((sx, sy, sw, sh)) => match cfg.toast_position {
            iris_lib::config::ToastPosition::BottomRight => point(
                px(sx) + px(sw) - win_size.width,
                px(sy) + px(sh) - win_size.height,
            ),
            iris_lib::config::ToastPosition::BottomLeft => {
                point(px(sx), px(sy) + px(sh) - win_size.height)
            }
            iris_lib::config::ToastPosition::TopRight => {
                point(px(sx) + px(sw) - win_size.width, px(sy))
            }
            iris_lib::config::ToastPosition::TopLeft => point(px(sx), px(sy)),
        },
        None => point(px(0.), px(0.)),
    };
    let (ox, oy): (f32, f32) = (origin.x.into(), origin.y.into());
    stage.card_screen = match cfg.toast_position {
        iris_lib::config::ToastPosition::BottomRight => (
            ox + f32::from(win_size.width) - MARGIN - w,
            oy + f32::from(win_size.height) - MARGIN - h,
            w,
            h,
        ),
        iris_lib::config::ToastPosition::BottomLeft => (
            ox + MARGIN,
            oy + f32::from(win_size.height) - MARGIN - h,
            w,
            h,
        ),
        iris_lib::config::ToastPosition::TopRight => (
            ox + f32::from(win_size.width) - MARGIN - w,
            oy + MARGIN,
            w,
            h,
        ),
        iris_lib::config::ToastPosition::TopLeft => (ox + MARGIN, oy + MARGIN, w, h),
    };
    let win_id = crate::sys::window::unique_id("dev.iris.toast");
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin,
                    size: win_size,
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
            |_, cx| cx.new(|_| stage),
        )
        .map_err(|e| format!("open toast window: {e}"))?;

    if let Some(previous) = TOAST_HANDLE.lock().replace(handle) {
        let _ = previous.update(cx, |stage, _window, cx| {
            stage.fade_out_quick(cx);
        });
    }

    handle
        .update(cx, |stage: &mut ToastStage, _window, cx| {
            stage.arm_dismiss(cx);
        })
        .map_err(|e| format!("arm dismiss: {e}"))?;

    // openbox-style WMs apply their own placement at map time; put the
    // window back where the corner is. No-op once the WM honors the
    // requested origin (mutter, KWin).
    let (ox, oy): (f32, f32) = (origin.x.into(), origin.y.into());
    crate::sys::window::place_after_map_kind(win_id, ox, oy, true);
    Ok(())
}
