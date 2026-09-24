use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui::*;
use iris_lib::config::Config;

use crate::{flash, notice, overlay, pipeline, stage};

/// Region capture: the overlay shells map at once, transparent over
/// the live desktop with crosshair and window-snap already live. The
/// grab and the per-monitor slices run behind them and each frozen
/// frame fades in as it lands, so the keypress reads as instant even
/// on a multi-4K setup where the grab takes half a second.
pub(super) fn capture_region(cx: &mut App) -> Result<(), String> {
    let t0 = Instant::now();
    // The grab goes out FIRST: the X server reads the framebuffer
    // when it processes the request, before any overlay pixel can
    // map, so our own windows can never enter the frozen frame.
    let grab = cx
        .background_executor()
        .spawn(async move { pipeline::grab_frame_bgra() });
    // The layout query is cheap (xrandr + window list, single-digit
    // ms); the shell opens on it immediately.
    let layout = overlay::layout();
    // Reuse the pooled window when there is one: GPUI window init is
    // ~65% of keypress-to-overlay latency, and a parked window skips
    // all of it. A live session (visible overlay) swallows the press.
    let pooled = *overlay::POOL.lock();
    let handle = if let Some(h) = pooled {
        let session = h.update(cx, |o, _, cx| {
            if o.hidden {
                o.reset(&layout);
                cx.notify();
                true
            } else {
                false
            }
        });
        match session {
            Ok(true) => {
                let u = &layout.union;
                let _ = h.update(cx, |_, window, _| {
                    crate::sys::window::unpark_span(window, u.x, u.y, u.width, u.height);
                    // Activate on GPUI's own connection too: a window
                    // manager that handles the map after the request
                    // applies its own focus.
                    window.activate_window();
                });
                h
            }
            // Busy mid-session: no stacking. Dead handle: fresh open.
            Ok(false) => return Ok(()),
            Err(e) => {
                iris_lib::ilog!("iris: capture: pooled handle dead ({e:?}), reopening");
                // Drop the dead handle from the pool: leaving it means
                // every later capture reads it, fails update() the same
                // way, and pays a wasted round trip before the fresh
                // open.
                *overlay::POOL.lock() = None;
                overlay::open_shell(cx, &layout)?
            }
        }
    } else {
        overlay::open_shell(cx, &layout)?
    };
    iris_lib::ilog!("iris: capture: shell in {:?}", t0.elapsed());
    cx.spawn(async move |cx| {
        let grabbed = cx
            .background_executor()
            .spawn(async move {
                let t_grab = Instant::now();
                let (width, height, bgra) = grab.await?;
                let grab_ms = t_grab.elapsed();
                let t_slice = Instant::now();
                let img = overlay::slice_frame_bgra(width, height, bgra);
                iris_lib::ilog!(
                    "iris: capture: grab {:?}, slice {:?}",
                    grab_ms,
                    t_slice.elapsed()
                );
                Ok::<(Arc<gpui::RenderImage>, u32, u32), String>((img, width, height))
            })
            .await;
        let _ = cx.update(|cx| match grabbed {
            Ok((img, width, height)) => {
                iris_lib::ilog!("iris: capture: frame landed in {:?}", t0.elapsed());
                let _ = handle.update(cx, |overlay, window, cx| {
                    overlay.set_frame(img, width, height, window, cx);
                    cx.notify();
                });
            }
            Err(e) => {
                iris_lib::ilog!("iris: capture: {e}");
                let _ = handle.update(cx, |overlay, window, cx| {
                    overlay.cancel(window, cx);
                });
                notice::failed(cx, "Capture failed", &e);
            }
        });
    })
    .detach();
    Ok(())
}

/// Full-screen capture, no overlay. The grab goes out at once, so the
/// frame is the screen at the press; the flash opens once that frame is
/// in memory and so can never enter it. The grab and the PNG save run
/// off the main thread: a 4K frame would otherwise freeze the app for
/// seconds.
pub(super) fn capture_fullscreen(cx: &mut App) -> Result<(), String> {
    let grab = cx.background_executor().spawn(async move {
        let frame = pipeline::grab_frame()?;
        pipeline::play_shutter_sound();
        // The region is the whole frame by construction; crop would
        // copy up to 200MB for nothing.
        image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
            .ok_or_else(|| "frame buffer size mismatch".to_string())
    });
    cx.spawn(async move |cx| {
        let done = match grab.await {
            Ok(img) => {
                // The save starts before the flash window opens, so the
                // PNG encode overlaps the window's renderer setup.
                let saved = cx
                    .background_executor()
                    .spawn(async move { pipeline::finalize(img) });
                if Config::load().flash_on_capture {
                    let _ = cx.update(|cx| {
                        if let Err(e) = flash::show(cx) {
                            iris_lib::ilog!("iris: capture flash: {e}");
                        }
                    });
                }
                saved.await
            }
            Err(e) => Err(e),
        };
        let _ = cx.update(|cx| landed(cx, done));
    })
    .detach();
    Ok(())
}

/// Capture the focused window: its rect is read straight off the
/// screen, decorations included, on X11, Windows, and macOS. The
/// Wayland portal cannot name a window, so there this returns an error.
pub(super) fn capture_active_window(cx: &mut App) -> Result<(), String> {
    fn grab_and_finish() -> Result<(PathBuf, iris_lib::library::CaptureEntry), String> {
        let rect = iris_lib::capture::active_window_rect()?;
        // Grab only the window's rect off the root: the whole-screen
        // grab + crop this replaced moved up to 200MB for a window
        // that may be a megabyte. Root pixels keep the decorations.
        let frame = iris_lib::capture::grab_rect(rect)?;
        pipeline::play_shutter_sound();
        let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
            .ok_or("frame buffer size mismatch")?;
        pipeline::finalize(img)
    }
    cx.spawn(async move |cx| {
        let done = cx
            .background_executor()
            .spawn(async move { grab_and_finish() })
            .await;
        let _ = cx.update(|cx| landed(cx, done));
    })
    .detach();
    Ok(())
}

/// A capture with no overlay finished: its toast, or why it failed.
fn landed(cx: &mut App, done: Result<(PathBuf, iris_lib::library::CaptureEntry), String>) {
    match done {
        Ok((path, _)) => {
            if Config::load().show_toast_after_capture {
                stage::show_toast(cx, &path);
            }
        }
        Err(e) => {
            iris_lib::ilog!("iris: capture: {e}");
            notice::failed(cx, "Capture failed", &e);
        }
    }
}
