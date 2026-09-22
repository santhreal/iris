use std::sync::Arc;

#[cfg(target_os = "linux")]
use futures::channel::mpsc::UnboundedSender;
use gpui::*;
use iris_lib::config::Config;
use iris_lib::record;

use crate::{chip, overlay, pipeline};

use super::capture::recording_params;
use super::RECORDING;
use super::{command_tx, Command};

/// ChipFollow that moves the GPUI chip window by XID (no app round-trip
/// per move) and asks the daemon to close it at the end. The XID is
/// resolved lazily: at window-open time the X window may not be in the
/// tree yet. X11-only: the desktop source on Windows/macOS has no
/// per-window chip to follow.
#[cfg(target_os = "linux")]
struct XcbChip {
    xid: std::sync::atomic::AtomicU32,
    conn: Option<&'static x11rb::rust_connection::RustConnection>,
    done: Option<UnboundedSender<Command>>,
}

#[cfg(target_os = "linux")]
impl XcbChip {
    fn resolve_xid(&self) -> u32 {
        let cached = self.xid.load(std::sync::atomic::Ordering::SeqCst);
        if cached != 0 {
            return cached;
        }
        let Some(conn) = &self.conn else { return 0 };
        if let Some(xid) = crate::chip::find_chip_xid_on(conn) {
            self.xid.store(xid, std::sync::atomic::Ordering::SeqCst);
            return xid;
        }
        0
    }
}

#[cfg(target_os = "linux")]
impl record::x11::ChipFollow for XcbChip {
    fn place(&self, rect: record::x11::Rect) {
        let Some(conn) = &self.conn else { return };
        let xid = self.resolve_xid();
        if xid == 0 {
            return;
        }
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt};
        // Bleed-compensated: the pill sits 16px inside the window.
        let x = i32::from(rect.x) + i32::from(rect.w) - 148 - 16;
        let y = (i32::from(rect.y) - 44 - 16).max(0);
        let _ = conn.configure_window(xid, &ConfigureWindowAux::new().x(x).y(y));
        let _ = conn.flush();
    }
    fn hide(&self) {
        if let Some(tx) = &self.done {
            let _ = tx.unbounded_send(Command::ChipHide);
        }
    }
}

/// One action for the record hotkey/tray/CLI: stop when active, start a
/// window-picked recording when idle.
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
    #[cfg(any(windows, target_os = "macos"))]
    let active = start_desktop(cx, None)?;
    #[cfg(target_os = "linux")]
    let active = start_linux(cx)?;
    RECORDING.lock().active = Some(active);
    Ok(())
}

/// Window-picked recording: the Wayland portal, or X11 with the chip
/// following the picked window.
#[cfg(target_os = "linux")]
fn start_linux(cx: &mut App) -> Result<record::ActiveRecording, String> {
    let cfg = Config::load();
    let (output, mic, format, encoder) = recording_params(&cfg);
    let wayland_only =
        std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none();
    let active = if wayland_only {
        record::ActiveRecording::spawn(
            output,
            cfg.recording_fps,
            mic,
            format,
            encoder,
            record::wayland::record_window,
        )
    } else {
        let xid = chip::open(cx, mic, None)?;
        let conn = iris_lib::capture::x11::shared_conn().ok().map(|(c, _)| c);
        let follower = std::sync::Arc::new(XcbChip {
            xid: std::sync::atomic::AtomicU32::new(xid),
            conn,
            done: command_tx(),
        });
        record::ActiveRecording::spawn(
            output,
            cfg.recording_fps,
            mic,
            format,
            encoder,
            move |spec| record::x11::record_window_follow(spec, follower),
        )
    };
    Ok(active)
}

/// Desktop recording on Windows and macOS: the main display, or
/// `region` (root pixels) of the display that holds it. The chip opens
/// above the region; the source closes it when it ends, so an ffmpeg
/// failure does not leave it on screen.
#[cfg(any(windows, target_os = "macos"))]
fn start_desktop(
    cx: &mut App,
    region: Option<iris_lib::capture::WinRect>,
) -> Result<record::ActiveRecording, String> {
    let cfg = Config::load();
    let (output, mic, format, encoder) = recording_params(&cfg);
    // GIF and WebM carry no audio track; the chip shows what is recorded.
    let mic = mic && format == iris_lib::config::RecordingFormat::Mp4;
    chip::open(cx, mic, region)?;
    let done = command_tx();
    Ok(record::ActiveRecording::spawn(
        output,
        cfg.recording_fps,
        mic,
        format,
        encoder,
        move |spec| {
            let result = record::desktop::record_desktop(spec, region);
            if let Some(tx) = &done {
                let _ = tx.unbounded_send(Command::ChipHide);
            }
            result
        },
    ))
}

/// Region recording, step one: open the overlay in pick mode. The
/// frozen frame is not needed for picking, so the shell opens on the
/// live desktop and the grab still runs behind it for the loupe.
pub(super) fn record_region_pick(cx: &mut App) -> Result<(), String> {
    // Same dead end as record_region_start: the picked rect feeds an
    // X11-only source, so on Wayland-only fail before the overlay opens.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
        return Err("region recording needs X11; on Wayland record a window".to_string());
    }
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

/// Region recording, step two: the overlay committed a rect. Open the
#[cfg(target_os = "linux")]
pub(super) fn record_region_start(
    cx: &mut App,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<(), String> {
    // Region recording reads the root window through X11 SHM; the
    // portal cannot name a rect, so on a Wayland-only session this
    // would open the chip and die on the first frame.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none() {
        return Err("region recording needs X11; on Wayland record a window".to_string());
    }
    let mut mgr = RECORDING.lock();
    if mgr.is_active() {
        return Err("a recording is already active".to_string());
    }
    let cfg = Config::load();
    let (output, mic, format, encoder) = recording_params(&cfg);
    let xid = chip::open(cx, mic, None)?;
    let conn = iris_lib::capture::x11::shared_conn().ok().map(|(c, _)| c);
    let follower = std::sync::Arc::new(XcbChip {
        xid: std::sync::atomic::AtomicU32::new(xid),
        conn,
        done: command_tx(),
    });
    let rect = record::x11::Rect {
        x: x as i16,
        y: y as i16,
        w: w as u16,
        h: h as u16,
    };
    let active = record::ActiveRecording::spawn(
        output,
        cfg.recording_fps,
        mic,
        format,
        encoder,
        move |spec| record::x11::record_region(spec, follower, rect),
    );
    mgr.active = Some(active);
    Ok(())
}

/// Region recording, step two, on Windows and macOS: record the
/// committed rect.
#[cfg(any(windows, target_os = "macos"))]
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
    let region = iris_lib::capture::WinRect {
        x,
        y,
        width: w as u32,
        height: h as u32,
    };
    let active = start_desktop(cx, Some(region))?;
    RECORDING.lock().active = Some(active);
    Ok(())
}
