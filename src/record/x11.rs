//! X11 per-window recording: click-to-pick a window, then record ONLY that
//! window via its XComposite backing pixmap. Occluding windows, the
//! desktop, and other applications never enter the frame; the pixmap holds
//! the window's own pixels.
//!
//! While recording, the target window wears a thin red override-redirect
//! border with an empty input shape (clicks pass straight through) plus a
//! display-only timer chip: recording state is visible on the window
//! itself rather than in a floating panel.

mod grab;
mod mark;
mod pick;

pub use mark::RecordingMark;
pub use pick::pick_window;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as CompositeExt, Redirect};
use x11rb::protocol::xproto::{ConnectionExt as XprotoExt, EventMask, Window};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use super::encoder::{Encoder, EncoderConfig, PixFmt};
use super::{unique_recording_path, RecordingSpec};

use grab::{grab_pixmap, grab_root_rect, resolve_pix_fmt, DamageWatch, NamedPixmap, ShmGrab};
use mark::connect;

/// Picked target: window id plus its geometry.
pub struct PickedWindow {
    pub id: Window,
    pub width: u32,
    pub height: u32,
}

/// Outer geometry of a window in root coordinates.
#[derive(Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
}

/// Frame geometry for one grab: pixel dimensions plus the negotiated
/// depth and bytes-per-pixel. Grouping them stops a width/height or
/// depth/bpp transposition at a call site from compiling silently.
#[derive(Clone, Copy)]
pub(super) struct FrameGeom {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) depth: u8,
    pub(super) bpp: usize,
}

impl FrameGeom {
    /// Bytes in one packed frame at this geometry.
    pub(super) fn bytes(&self) -> usize {
        self.width as usize * self.height as usize * self.bpp
    }
}

/// Follow target for the recording chip: the border thread reports the
/// target window's rect, and the chip's owner repositions (and at the
/// end removes) the chip window. The GPUI daemon implements this over
/// its native chip window.
pub trait ChipFollow: Send + Sync {
    fn place(&self, rect: Rect);
    fn hide(&self);
}

/// Record the picked window until `spec.stop` fires, with the chip owned
/// by the caller through `chip`. This is the toolkit-independent entry
/// point used by the GPUI daemon.
pub fn record_window_follow(
    spec: RecordingSpec,
    chip: Arc<dyn ChipFollow>,
) -> Result<PathBuf, String> {
    let chip_inner = chip.clone();
    let result = (move || {
        let picked = pick_window(&spec.stop)?;
        let _mark = RecordingMark::show(chip_inner, picked.id);
        record_target(&picked, &spec)
    })();
    // On every exit (cancel, error, or clean stop), the chip goes away.
    chip.hide();
    result
}

fn record_target(picked: &PickedWindow, spec: &RecordingSpec) -> Result<PathBuf, String> {
    let (conn, screen_num) = connect()?;

    // Composite 0.2+ for window redirection + named pixmaps.
    let version = conn
        .composite_query_version(0, 4)
        .map_err(|e| format!("composite extension missing: {e}"))?
        .reply()
        .map_err(|e| format!("composite_query_version: {e}"))?;
    if version.major_version == 0 && version.minor_version < 2 {
        return Err(format!(
            "composite extension too old: {}.{}",
            version.major_version, version.minor_version
        ));
    }

    conn.composite_redirect_window(picked.id, Redirect::AUTOMATIC)
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("redirect window: {e}"))?;

    let result = record_loop(&conn, screen_num, picked, spec);

    let _ = conn.composite_unredirect_window(picked.id, Redirect::AUTOMATIC);
    result
}

fn record_loop(
    conn: &RustConnection,
    screen_num: usize,
    picked: &PickedWindow,
    spec: &RecordingSpec,
) -> Result<PathBuf, String> {
    let (pix_fmt, depth) = resolve_pix_fmt(conn, screen_num, picked.id)?;
    let bpp = pix_fmt.bytes_per_pixel();
    // Persistent SHM segment + named pixmap for per-frame grabs; the
    // guard frees both on every exit path, including early returns.
    struct GrabGuard<'a> {
        conn: &'a RustConnection,
        shm: Option<ShmGrab>,
        named: Option<NamedPixmap>,
    }
    impl Drop for GrabGuard<'_> {
        fn drop(&mut self) {
            if let Some(s) = self.shm.take() {
                s.detach(self.conn);
            }
            if let Some(n) = self.named.take() {
                n.free(self.conn);
            }
        }
    }
    let mut guard = GrabGuard {
        conn,
        shm: None,
        named: None,
    };

    // Resize/close tracking is event-driven: StructureNotify delivers
    // ConfigureNotify and DestroyNotify, so the per-frame probe drains
    // the event queue instead of paying a get_geometry round trip on
    // every frame of the recording.
    let _ = conn.change_window_attributes(
        picked.id,
        &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
            .event_mask(EventMask::STRUCTURE_NOTIFY),
    );
    let _ = conn.flush();
    let mut dims = conn
        .get_geometry(picked.id)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|g| (u32::from(g.width), u32::from(g.height)));
    // Damage subscription on the window: an idle window repeats the
    // writer's last buffer instead of paying a grab + frame copy for
    // identical output.
    let mut watch = DamageWatch::arm(conn, picked.id, None);
    struct WatchGuard<'a> {
        conn: &'a RustConnection,
        watch: Option<DamageWatch>,
    }
    impl Drop for WatchGuard<'_> {
        fn drop(&mut self) {
            if let Some(w) = self.watch.take() {
                w.release(self.conn);
            }
        }
    }
    let mut watch_guard = WatchGuard {
        conn,
        watch: watch.take(),
    };
    let probe_events = move || {
        loop {
            match conn.poll_for_event() {
                Ok(Some(Event::ConfigureNotify(ev))) if ev.window == picked.id => {
                    dims = Some((u32::from(ev.width), u32::from(ev.height)));
                }
                Ok(Some(Event::DestroyNotify(ev))) if ev.window == picked.id => {
                    return None;
                }
                Ok(Some(Event::DamageNotify(ev))) => {
                    if let Some(w) = watch_guard.watch.as_mut() {
                        w.note(&ev);
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => return None, // connection died
            }
        }
        let dirty = watch_guard
            .watch
            .as_mut()
            .map(|w| w.take_dirty(conn))
            .unwrap_or(true);
        dims.map(|d| (d, dirty))
    };
    let grab = |w: u32, h: u32, rgba: &mut Vec<u8>| {
        let geom = FrameGeom {
            width: w,
            height: h,
            depth,
            bpp,
        };
        grab_pixmap(
            conn,
            &mut guard.shm,
            &mut guard.named,
            picked.id,
            geom,
            rgba,
        )
    };
    record_loop_inner(spec, pix_fmt, probe_events, grab)
}

/// Record a fixed screen region until `spec.stop` fires. The overlay
/// picks the rect; this grabs it straight off the root window through
/// the same SHM path, with a static border and the chip parked at the
/// rect's top-right corner.
pub fn record_region(
    spec: RecordingSpec,
    chip: Arc<dyn ChipFollow>,
    rect: Rect,
) -> Result<PathBuf, String> {
    let chip_inner = chip.clone();
    let result = (move || {
        let (conn, screen_num) = connect()?;
        let root = conn.setup().roots[screen_num].root;
        let (pix_fmt, depth) = resolve_pix_fmt(&conn, screen_num, root)?;
        let bpp = pix_fmt.bytes_per_pixel();
        let _mark = RecordingMark::show_static(chip_inner, root, rect);
        struct ShmGuard<'a> {
            conn: &'a RustConnection,
            shm: Option<ShmGrab>,
        }
        impl Drop for ShmGuard<'_> {
            fn drop(&mut self) {
                if let Some(s) = self.shm.take() {
                    s.detach(self.conn);
                }
            }
        }
        let mut guard = ShmGuard {
            conn: &conn,
            shm: None,
        };
        // Damage on the root window, filtered to the record rect: a
        // quiet region repeats the writer's last buffer.
        let mut watch = DamageWatch::arm(&conn, root, Some(rect));
        struct WatchGuard<'a> {
            conn: &'a RustConnection,
            watch: Option<DamageWatch>,
        }
        impl Drop for WatchGuard<'_> {
            fn drop(&mut self) {
                if let Some(w) = self.watch.take() {
                    w.release(self.conn);
                }
            }
        }
        let mut watch_guard = WatchGuard {
            conn: &conn,
            watch: watch.take(),
        };
        let probe = || {
            loop {
                match conn.poll_for_event() {
                    Ok(Some(Event::DamageNotify(ev))) => {
                        if let Some(w) = watch_guard.watch.as_mut() {
                            w.note(&ev);
                        }
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => return None,
                }
            }
            let dirty = watch_guard
                .watch
                .as_mut()
                .map(|w| w.take_dirty(&conn))
                .unwrap_or(true);
            Some(((rect.w as u32, rect.h as u32), dirty))
        };
        let grab = |w: u32, h: u32, rgba: &mut Vec<u8>| {
            let geom = FrameGeom {
                width: w,
                height: h,
                depth,
                bpp,
            };
            grab_root_rect(&conn, &mut guard.shm, root, rect, geom, rgba)
        };
        record_loop_inner(&spec, pix_fmt, probe, grab)
    })();
    chip.hide();
    result
}

/// The shared recording loop: chip controls, the absolute frame
/// schedule, encoder splits on resize, and the zero-copy frame queue.
/// `events` drains the source's event state and reports its current
/// dimensions plus whether its pixels changed since the last call
/// (None = source gone, a clean end); `grab` fills `rgba` with one
/// frame. An unchanged source repeats the writer's last buffer: the
/// grab and the frame copy both skip. Returns the path of the LAST
/// segment written: splits rename the output, and the caller must
/// report the file that actually holds the tail.
fn record_loop_inner(
    spec: &RecordingSpec,
    pix_fmt: PixFmt,
    mut events: impl FnMut() -> Option<((u32, u32), bool)>,
    mut grab: impl FnMut(u32, u32, &mut Vec<u8>) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let Some(((mut width, mut height), _)) = events() else {
        return Err("recording source gone before first frame".to_string());
    };
    let mut output: PathBuf = spec.output.clone();
    let mut encoder = Encoder::start(&EncoderConfig {
        output: output.clone(),
        width,
        height,
        fps: spec.fps,
        mic: spec.mic,
        format: spec.format,
        encoder: spec.encoder,
        pix_fmt,
    })?;
    let frame_interval = Duration::from_secs_f64(1.0 / f64::from(spec.fps));
    let mut start = Instant::now();
    let mut mic = spec.mic;
    let mut frame_no: u64 = 0;
    // Reused RGBA scratch: recording would otherwise allocate a fresh
    // w*h*4 buffer on every frame.
    let mut rgba: Vec<u8> = Vec::new();

    // A stop is a send OR a disconnect: a dropped ActiveRecording must
    // still end the loop, or the recording runs forever detached.
    let stopped = |stop: &std::sync::mpsc::Receiver<()>| -> bool {
        !matches!(stop.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty))
    };
    // Split the file at the current dimensions/format and start a new
    // segment under a fresh unique name.
    macro_rules! split_encoder {
        () => {{
            encoder.finish()?;
            let dir = output
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            output = unique_recording_path(&dir, super::ext_of(&output));
            encoder = Encoder::start(&EncoderConfig {
                output: output.clone(),
                width,
                height,
                fps: spec.fps,
                mic,
                format: spec.format,
                encoder: spec.encoder,
                pix_fmt,
            })?;
        }};
    }

    loop {
        if stopped(&spec.stop) {
            encoder.finish()?;
            return Ok(output);
        }

        // Chip controls: pause blocks the schedule (the mp4 simply has
        // no frames for the paused span), mic toggle splits the file the
        // same way a resize does. Any split must grab the next frame:
        // the new writer's `last` is empty, so a Repeat writes nothing.
        let mut force_grab = false;
        while let Ok(ctl) = spec.control.try_recv() {
            match ctl {
                super::RecControl::Pause => {
                    let paused_at = Instant::now();
                    loop {
                        if stopped(&spec.stop) {
                            encoder.finish()?;
                            return Ok(output);
                        }
                        match spec.control.recv_timeout(Duration::from_millis(100)) {
                            Ok(super::RecControl::Resume) => break,
                            Ok(super::RecControl::ToggleMic) => {
                                mic = !mic;
                                split_encoder!();
                                force_grab = true;
                            }
                            Ok(super::RecControl::Pause) => {}
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    // Rebase the absolute schedule so the paused span
                    // does not arrive as a burst of dropped frames.
                    start += paused_at.elapsed();
                }
                super::RecControl::Resume => {}
                super::RecControl::ToggleMic => {
                    mic = !mic;
                    split_encoder!();
                    force_grab = true;
                }
            }
        }

        // Detect resize / close each frame. A vanished source is a clean
        // end of the recording, not an error; so is a zero-size probe
        // (a minimized window reports 0x0 and would kill the encoder).
        let Some(((w, h), dirty_early)) = events() else {
            encoder.finish()?;
            return Ok(output);
        };
        if w == 0 || h == 0 {
            encoder.finish()?;
            return Ok(output);
        }
        // A resize split must grab the first frame of the new segment:
        // the writer's `last` belongs to the old encoder's queue.
        if w != width || h != height {
            width = w;
            height = h;
            split_encoder!();
            force_grab = true;
        }

        // Absolute schedule: frame n is due at start + n/fps. On overrun,
        // skip the counter forward (drop) instead of bursting.
        let due = start + frame_interval.mul_f64(frame_no as f64);
        let now = Instant::now();
        if now < due {
            thread::sleep(due - now);
        } else if now - due > frame_interval * 2 {
            frame_no = ((now - start).as_secs_f64() * f64::from(spec.fps)) as u64;
        }

        // Sample the dirty flag after the sleep so damage that landed
        // while waiting is caught; a source that never changed repeats
        // the writer's last buffer and skips the grab entirely.
        let Some((_, dirty_late)) = events() else {
            encoder.finish()?;
            return Ok(output);
        };
        if !dirty_early && !dirty_late && !force_grab {
            if let Err(e) = encoder.repeat_frame() {
                let _ = encoder.finish();
                return Err(e);
            }
            frame_no += 1;
            continue;
        }

        match grab(width, height, &mut rgba) {
            Ok(()) => {}
            Err(_) => {
                // Source closed mid-grab: keep what we have.
                encoder.finish()?;
                return Ok(output);
            }
        };
        // The frame buffer moves to the writer thread; the next frame
        // fills a recycled one.
        let frame = std::mem::replace(&mut rgba, encoder.take_buf());
        if let Err(e) = encoder.write_frame(frame) {
            // Salvage the buffered frames before reporting: a write
            // failure must not also lose the minutes already encoded.
            let _ = encoder.finish();
            return Err(e);
        }
        frame_no += 1;
    }
}
