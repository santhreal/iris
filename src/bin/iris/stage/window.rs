//! Toast stage window management, sizing, and lifecycle.

use std::{
    cell::Cell,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

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
    let (w, h) = card_size(img_w as f32, img_h as f32);
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

/// The live toast's card rect while it rests in its corner, as its
/// last render saw it. A render publishes it, so a notice opened from
/// inside the toast's own update reads it without leasing the toast.
static RESTING: parking_lot::Mutex<Option<Resting>> = parking_lot::Mutex::new(None);

/// A toast window and its card rect: x, y, width, height.
type Resting = (AnyWindowHandle, (f32, f32, f32, f32));

/// `toast` rests with its card at `card`, or no longer rests (closing,
/// morphing into the editor). A stale toast never clears a newer one.
pub(super) fn publish_rest(toast: AnyWindowHandle, card: Option<(f32, f32, f32, f32)>) {
    let mut slot = RESTING.lock();
    match card {
        Some(card) => *slot = Some((toast, card)),
        None if slot.is_some_and(|(h, _)| h == toast) => *slot = None,
        None => {}
    }
}

/// The live toast card's rect on screen while it rests in its corner;
/// a notice stacks beyond it instead of covering it.
pub fn live_card_rect(cx: &App) -> Option<(f32, f32, f32, f32)> {
    let (toast, card) = (*RESTING.lock())?;
    cx.windows().contains(&toast).then_some(card)
}

/// Open the toast stage window in the bottom-right corner of the primary
/// display for the capture at `path`. A toast that cannot open reports
/// why in a notice.
pub fn show_toast(cx: &mut App, path: &Path) {
    let path = path.to_path_buf();
    // Sized for the primary display, where the card opens; a window
    // that renders at another scale re-sizes the pixels itself.
    let scale = crate::sys::window::root_scale(cx);
    // Scaling a full-size capture is too slow for the UI thread; the
    // window opens once the pixels are ready.
    let scaled = cx.background_executor().spawn({
        let path = path.clone();
        async move { prepare_thumb(&path, scale) }
    });
    cx.spawn(async move |cx| {
        let thumb = scaled.await;
        let _ = cx.update(|cx| {
            if let Err(e) = thumb.and_then(|t| open_toast_window(cx, &path, t, false, None)) {
                iris_lib::ilog!("toast: {e}");
                crate::notice::failed(cx, "Toast did not open", &e);
            }
        });
    })
    .detach();
}

/// Runs once when a landed toast is on screen, or when it cannot be.
pub type Handoff = Box<dyn FnOnce(&mut App)>;

/// The capture-flight landing: the card appears at rest, no entrance,
/// with the pixels the flight scaled while it was in the air. `anchor`
/// is the committing monitor's rect in screen coordinates, so the card
/// lands where the flight ended, not on another display. `handoff` runs
/// exactly once: after the toast's first frame is on screen, or at once
/// if the toast cannot open. The action bar fades in after it.
pub fn show_toast_landed(
    cx: &mut App,
    path: &Path,
    thumb: Result<Thumb, String>,
    anchor: Option<(f32, f32, f32, f32)>,
    handoff: Handoff,
) {
    match thumb.and_then(|t| open_toast_window(cx, path, t, true, anchor)) {
        Ok(toast) => {
            let reveal = Box::new(move |cx: &mut App| {
                handoff(cx);
                let _ = toast.update(cx, |stage, _, cx| stage.reveal_bar(cx));
            });
            hand_off_after_present(toast, cx, reveal);
        }
        Err(e) => {
            iris_lib::ilog!("toast: {e}");
            crate::notice::failed(cx, "Toast did not open", &e);
            handoff(cx);
        }
    }
}

/// Bound on the wait for a landed toast's first presented frame: a
/// compositor that never answers a frame callback, or an X server that
/// never reports the damage, still gets the handoff.
const HANDOFF_DEADLINE: Duration = Duration::from_millis(250);

/// Run `handoff` once the toast's first frame is on screen. open_window
/// drew that frame without presenting it; the first frame callback's
/// frame presents it. Where the platform reports a present (X11 damage)
/// the handoff waits for that report, since an X11 frame tick follows a
/// timer and the present lands on the server later. Elsewhere the next
/// frame callback runs after the compositor composited the present.
fn hand_off_after_present(toast: WindowHandle<ToastStage>, cx: &mut App, handoff: Handoff) {
    let until = Instant::now() + HANDOFF_DEADLINE;
    let slot = Rc::new(Cell::new(Some(handoff)));
    let on_present = slot.clone();
    let registered = toast.update(cx, |_, window, _| {
        let watch = crate::sys::window::PresentWatch::open(window, until);
        window.on_next_frame(move |window, cx| match watch.and_then(|w| w.arm()) {
            Some(presented) => cx
                .spawn(async move |cx| {
                    let _ = presented.await;
                    let _ = cx.update(|cx| run_handoff(&on_present, cx));
                })
                .detach(),
            // The handoff parks another window; run it after this
            // window's frame callback returns.
            None => window.on_next_frame(move |_, cx| {
                cx.defer(move |cx| run_handoff(&on_present, cx));
            }),
        });
    });
    if registered.is_err() {
        run_handoff(&slot, cx);
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(HANDOFF_DEADLINE).await;
        let _ = cx.update(|cx| run_handoff(&slot, cx));
    })
    .detach();
}

fn run_handoff(slot: &Cell<Option<Handoff>>, cx: &mut App) {
    if let Some(handoff) = slot.take() {
        handoff(cx);
    }
}

fn open_toast_window(
    cx: &mut App,
    path: &Path,
    thumb: Thumb,
    landed: bool,
    anchor: Option<(f32, f32, f32, f32)>,
) -> Result<WindowHandle<ToastStage>, String> {
    let mut stage = ToastStage::from_parts(path, thumb);
    if landed {
        // Pretend the entrance finished long ago. The flight card it
        // replaces has no action bar; the handoff fades it in.
        stage.opened = Some(Instant::now() - motion::tempo(ENTER));
        stage.bar = Bar::Hidden;
    }
    let (w, h) = stage.thumb.dims;

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
    let card = stage.card_screen;
    let handle = crate::widgets::open_window(
        cx,
        "Screenshot - iris",
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
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        },
        |_, cx| cx.new(|_| stage),
    )
    .map_err(|e| format!("open toast window: {e}"))?;

    if let Some(previous) = TOAST_HANDLE.lock().replace(handle) {
        let _ = previous.update(cx, |stage, _window, cx| {
            stage.fade_out_quick(cx);
        });
    }
    crate::notice::yield_to(cx, card);

    handle
        .update(cx, |stage: &mut ToastStage, _window, cx| {
            stage.arm_dismiss(cx);
        })
        .map_err(|e| format!("arm dismiss: {e}"))?;
    Ok(handle)
}
