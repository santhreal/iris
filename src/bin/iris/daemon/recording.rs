use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use gpui::*;
use iris_lib::config::Config;
use iris_lib::record::ActiveRecording;

use crate::sys::record::{self as source, Params};
use crate::{chip, notice, overlay, pipeline};

use super::RECORDING;

/// Recordings stopped and still flushing: ffmpeg's trailer and the
/// segment join run on these threads after the chip closes.
static FLUSHING: parking_lot::Mutex<Vec<JoinHandle<()>>> = parking_lot::Mutex::new(Vec::new());

/// Stop the live recording and wait for every flush in flight. The
/// daemon runs this as it quits, on `--quit` or because its display
/// server went away: the segments are joined into the output file
/// last, so an exit before a flush ends loses the recording.
pub(super) fn save_before_exit() {
    let active = RECORDING.lock().take();
    if let Some(rec) = active {
        log_result(&rec.stop());
    }
    let flushing = std::mem::take(&mut *FLUSHING.lock());
    for flush in flushing {
        let _ = flush.join();
    }
}

/// One action for the record hotkey/tray/CLI: stop when active, start
/// the platform's default recording when idle.
pub(super) fn toggle_recording(cx: &mut App) -> Result<(), String> {
    let active = RECORDING.lock().take();
    if let Some(rec) = active {
        finish(cx, rec);
        return Ok(());
    }
    source::ready()?;
    let started = source::start_window(cx, Params::from_config(&Config::load()));
    // A source that fails after opening the chip leaves it on screen.
    let active = started.inspect_err(|_| chip::close(cx))?;
    *RECORDING.lock() = Some(active);
    Ok(())
}

/// A source sent its end notice. One that ended on its own (it failed,
/// lost its target, or its pick was cancelled) is still registered:
/// collect and report it. The notice of a recording already stopped
/// finds a newer recording, or none, and leaves it be.
pub(super) fn collect_ended(cx: &mut App) {
    let ended = RECORDING.lock().take_if(|rec| rec.ended());
    if let Some(rec) = ended {
        finish(cx, rec);
    }
}

/// Close the chip and stop `rec`. The join (ffmpeg's trailer flush and
/// the segment join) can take seconds on a long recording; it runs off
/// the UI thread so hotkeys and socket commands stay live meanwhile,
/// and the result is reported back on it.
fn finish(cx: &mut App, rec: ActiveRecording) {
    chip::close(cx);
    let (tx, rx) = futures::channel::oneshot::channel();
    let flush = rec.stop_async(move |result| {
        log_result(&result);
        let _ = tx.send(result);
    });
    {
        let mut flushing = FLUSHING.lock();
        flushing.retain(|f| !f.is_finished());
        flushing.push(flush);
    }
    cx.spawn(async move |cx| {
        if let Ok(result) = rx.await {
            let _ = cx.update(|cx| report(cx, result));
        }
    })
    .detach();
}

/// Show a finished recording: the saved file, or why it failed. A
/// recording with no file (a cancelled pick) shows nothing.
fn report(cx: &mut App, result: Result<Option<PathBuf>, String>) {
    match result {
        Ok(Some(path)) => notice::saved(cx, "Recording saved", &path),
        Ok(None) => {}
        Err(e) => notice::failed(cx, "Recording failed", &e),
    }
}

/// Log a finished recording: the saved file, or why it failed. Runs
/// where the flush ends, so a flush the exit waits on is logged too.
fn log_result(result: &Result<Option<PathBuf>, String>) {
    match result {
        Ok(Some(path)) => iris_lib::ilog!("iris: recording saved: {}", path.display()),
        Ok(None) => {}
        Err(e) => iris_lib::ilog!("iris: recording: {e}"),
    }
}

/// Region recording, step one: open the overlay in pick mode. The
/// frozen frame is not needed for picking, so the shell opens on the
/// live desktop and the grab still runs behind it for the loupe.
pub(super) fn record_region_pick(cx: &mut App) -> Result<(), String> {
    // Fail before the overlay opens when the picked rect has no source.
    source::ready()?;
    source::region_available()?;
    if RECORDING.lock().is_some() {
        return Err("a recording is already active".to_string());
    }
    // The encoder probes run while the rect is dragged, not after.
    let cfg = Config::load();
    iris_lib::record::codec::warm(cfg.recording_format, cfg.recording_encoder);
    let grab = cx
        .background_executor()
        .spawn(async move { pipeline::grab_frame_bgra() });
    let layout = overlay::layout();
    // Same pool path as capture_region: a parked window skips GPUI's
    // init, and reset() re-arms it before the mode flips to pick.
    let pooled = *overlay::POOL.lock();
    let handle = if let Some(h) = pooled {
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
                let _ = h.update(cx, |_, window, _| {
                    crate::sys::window::unpark_span(window, u.x, u.y, u.width, u.height);
                    window.activate_window();
                });
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
                notice::failed(cx, "Recording failed", &e);
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
    if RECORDING.lock().is_some() {
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
    let started = source::start_region(cx, Params::from_config(&Config::load()), rect);
    let active = started.inspect_err(|_| chip::close(cx))?;
    *RECORDING.lock() = Some(active);
    Ok(())
}
