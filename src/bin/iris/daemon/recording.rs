use std::sync::Arc;

use gpui::*;
use iris_lib::config::Config;

use crate::sys::record::{self as source, Params};
use crate::{chip, overlay, pipeline};

use super::RECORDING;

/// One action for the record hotkey/tray/CLI: stop when active, start
/// the platform's default recording when idle.
pub(super) fn toggle_recording(cx: &mut App) -> Result<(), String> {
    if RECORDING.lock().is_active() {
        // The join (ffmpeg's trailer flush) can take seconds on a long
        // recording; it runs off the UI thread so hotkeys and socket
        // commands stay live while the file finishes.
        RECORDING.lock().stop_async(|result| match result {
            Ok(Some(path)) => {
                iris_lib::ilog!("iris: recording saved: {}", path.display());
            }
            Ok(None) => {}
            Err(e) => iris_lib::ilog!("iris: recording stop: {e}"),
        });
        chip::close(cx); // defensive: any exit path that missed hide
        return Ok(());
    }
    let active = source::start_window(cx, Params::from_config(&Config::load()))?;
    RECORDING.lock().active = Some(active);
    Ok(())
}

/// Region recording, step one: open the overlay in pick mode. The
/// frozen frame is not needed for picking, so the shell opens on the
/// live desktop and the grab still runs behind it for the loupe.
pub(super) fn record_region_pick(cx: &mut App) -> Result<(), String> {
    // Fail before the overlay opens when the picked rect has no source.
    crate::sys::record::region_available()?;
    let mgr = RECORDING.lock();
    if mgr.is_active() {
        return Err("a recording is already active".to_string());
    }
    let grab = cx
        .background_executor()
        .spawn(async move { pipeline::grab_frame_bgra() });
    let layout = overlay::layout();
    // Same pool path as capture_region: a parked window skips GPUI's
    // init, and reset() re-arms it before the mode flips to pick.
    let pooled = overlay::POOL.lock().clone();
    let handle = if let Some((h, class)) = pooled {
        let session = h.update(cx, |o, _, cx| {
            if o.hidden {
                o.reset(&layout);
                o.mode = overlay::OverlayMode::RecordPick;
                cx.notify();
                true
            } else {
                false
            }
        });
        match session {
            Ok(true) => {
                let u = &layout.union;
                crate::sys::window::unpark_span(class, u.x, u.y, u.width, u.height);
                let _ = h.update(cx, |_, window, _| window.activate_window());
                h
            }
            Ok(false) => return Ok(()),
            Err(e) => {
                iris_lib::ilog!("iris: record-region: pooled handle dead ({e:?}), reopening");
                // Drop the dead handle from the pool, same as
                // capture_region: a stale handle fails update() on
                // every later pick before the fresh open.
                *overlay::POOL.lock() = None;
                let h = overlay::open_shell(cx, &layout)?;
                h.update(cx, |o, _, cx| {
                    o.mode = overlay::OverlayMode::RecordPick;
                    cx.notify();
                })
                .map_err(|e| format!("set pick mode: {e}"))?;
                h
            }
        }
    } else {
        let h = overlay::open_shell(cx, &layout)?;
        h.update(cx, |o, _, cx| {
            o.mode = overlay::OverlayMode::RecordPick;
            cx.notify();
        })
        .map_err(|e| format!("set pick mode: {e}"))?;
        h
    };
    cx.spawn(async move |cx| {
        let grabbed = cx
            .background_executor()
            .spawn(async move {
                let (width, height, bgra) = grab.await?;
                let img = overlay::slice_frame_bgra(width, height, bgra);
                Ok::<(Arc<gpui::RenderImage>, u32, u32), String>((img, width, height))
            })
            .await;
        let _ = cx.update(|cx| match grabbed {
            Ok((img, width, height)) => {
                let _ = handle.update(cx, |overlay, window, cx| {
                    overlay.set_frame(img, width, height, window, cx);
                    cx.notify();
                });
            }
            Err(e) => {
                iris_lib::ilog!("iris: record-region: {e}");
                let _ = handle.update(cx, |overlay, window, cx| {
                    overlay.cancel(window, cx);
                });
            }
        });
    })
    .detach();
    Ok(())
}

/// Region recording, step two: the overlay committed a rect (root
/// pixels). Start the platform region source.
pub(super) fn record_region_start(
    cx: &mut App,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<(), String> {
    if RECORDING.lock().is_active() {
        return Err("a recording is already active".to_string());
    }
    if w <= 0 || h <= 0 {
        return Err(format!("empty region {w}x{h}"));
    }
    let rect = iris_lib::capture::WinRect {
        x,
        y,
        width: w as u32,
        height: h as u32,
    };
    let active = source::start_region(cx, Params::from_config(&Config::load()), rect)?;
    RECORDING.lock().active = Some(active);
    Ok(())
}
