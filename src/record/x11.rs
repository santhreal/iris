//! X11 per-window recording: click-to-pick a window, then record ONLY that
//! window via its XComposite backing pixmap. Occluding windows, the
//! desktop, and other applications never enter the frame; the pixmap holds
//! the window's own pixels.
//!
//! While recording, the target window wears a thin red override-redirect
//! border with an empty input shape (clicks pass straight through), and
//! the chip follows it: recording state is visible on the window itself
//! rather than in a floating panel.

mod damage;
mod grab;
mod mark;
mod pick;
#[cfg(test)]
mod tests;

pub use mark::RecordingMark;

use std::os::fd::{AsFd, BorrowedFd};
use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as CompositeExt, Redirect};
use x11rb::protocol::xproto::{ConnectionExt as XprotoExt, EventMask, Window};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use super::mkv::PixFmt;
use super::recorder::{Recorder, Shape};
use super::wake::Wake;
use super::{RecControl, RecordingSpec};

use damage::DamageWatch;
use grab::{grab_pixmap, grab_root_rect, resolve_pix_fmt, NamedPixmap, ShmGrab};
use mark::connect;
use pick::pick_window;

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
/// target window's rect, and the chip's owner repositions the chip
/// window. The GPUI daemon implements this over its native chip window
/// and removes the chip when the recording ends.
pub trait ChipFollow: Send + Sync {
    fn place(&self, rect: Rect);
}

/// Record the picked window until `spec.stop` fires, with the chip owned
/// by the caller through `chip`. This is the toolkit-independent entry
/// point used by the GPUI daemon.
pub fn record_window_follow(
    spec: RecordingSpec,
    chip: Arc<dyn ChipFollow>,
) -> Result<PathBuf, String> {
    // Installed first: the pick and the recording both sleep until X
    // input or a ring.
    let wake = Wake::install(&spec.bell)?;
    let picked = pick_window(&spec.stop, &wake)?;
    let _mark = RecordingMark::show(chip, picked.id)?;
    record_target(&picked, &spec, &wake)
}

fn record_target(
    picked: &PickedWindow,
    spec: &RecordingSpec,
    wake: &Wake,
) -> Result<PathBuf, String> {
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

    let result = record_loop(&conn, screen_num, picked, spec, wake);

    let _ = conn.composite_unredirect_window(picked.id, Redirect::AUTOMATIC);
    result
}

fn record_loop(
    conn: &RustConnection,
    screen_num: usize,
    picked: &PickedWindow,
    spec: &RecordingSpec,
    wake: &Wake,
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
    let mut watch = DamageWatch::new(conn, picked.id, None);
    let probe_events = move |paused: bool| {
        loop {
            match conn.poll_for_event() {
                Ok(Some(Event::ConfigureNotify(ev))) if ev.window == picked.id => {
                    dims = Some((u32::from(ev.width), u32::from(ev.height)));
                }
                Ok(Some(Event::DestroyNotify(ev))) if ev.window == picked.id => {
                    return None;
                }
                Ok(Some(Event::DamageNotify(ev))) => watch.note(&ev),
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => return None, // connection died
            }
        }
        dims.map(|d| (d, watch.take_dirty(paused)))
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
    record_loop_inner(
        spec,
        conn.stream().as_fd(),
        wake,
        pix_fmt,
        probe_events,
        grab,
    )
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
    let wake = Wake::install(&spec.bell)?;
    let (conn, screen_num) = connect()?;
    let root = conn.setup().roots[screen_num].root;
    let (pix_fmt, depth) = resolve_pix_fmt(&conn, screen_num, root)?;
    let bpp = pix_fmt.bytes_per_pixel();
    let _mark = RecordingMark::show_static(chip, root, rect);
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
    let mut watch = DamageWatch::new(&conn, root, Some(rect));
    let probe = |paused: bool| {
        loop {
            match conn.poll_for_event() {
                Ok(Some(Event::DamageNotify(ev))) => watch.note(&ev),
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => return None,
            }
        }
        Some(((rect.w as u32, rect.h as u32), watch.take_dirty(paused)))
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
    record_loop_inner(&spec, conn.stream().as_fd(), &wake, pix_fmt, probe, grab)
}

/// The shared recording loop. `events` drains the source's events and
/// reports its size and whether its pixels changed since the last call
/// (None: the source is gone, a clean end); paused, it may stop
/// watching for changes. `grab` fills a buffer with one frame. A frame
/// is grabbed only when the source changed, at most `spec.fps` times a
/// second. A still or paused source costs no grab, no encode, and no
/// wakeup: the loop sleeps without a timeout until `input` (the X
/// connection) is readable or `wake` rings.
fn record_loop_inner(
    spec: &RecordingSpec,
    input: BorrowedFd<'_>,
    wake: &Wake,
    pix: PixFmt,
    mut events: impl FnMut(bool) -> Option<((u32, u32), bool)>,
    mut grab: impl FnMut(u32, u32, &mut Vec<u8>) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let mut rec = Recorder::new(spec);
    let interval = Duration::from_secs_f64(1.0 / f64::from(spec.fps));
    let mut due = Instant::now();
    // Set by damage, a resize, a resume (damage during a pause is
    // drained unrecorded), and at the start; cleared by a grab.
    let mut changed = true;
    let mut size = (0, 0);
    loop {
        // Drained before the channels are read: a send after the read
        // rings again, and the next wait ends at once.
        wake.drain();
        // A stop is a send OR a disconnect: a dropped ActiveRecording
        // must still end the loop, or the recording runs forever.
        if !matches!(spec.stop.try_recv(), Err(TryRecvError::Empty)) {
            return rec.finish(Instant::now());
        }
        loop {
            match spec.control.try_recv() {
                Ok(ctl) => {
                    changed |= ctl == RecControl::Resume;
                    if let Err(e) = rec.apply(ctl, Instant::now()) {
                        return Err(rec.fail(Instant::now(), e));
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return rec.finish(Instant::now()),
            }
        }
        // Every queued event goes before a wait: input already read
        // into the connection's buffer does not wake the poll. Drained
        // while paused too. A vanished source ends the recording
        // cleanly, and so does a minimized window, which reports 0x0.
        let paused = rec.paused();
        let Some(((w, h), dirty)) = events(paused).filter(|((w, h), _)| *w > 0 && *h > 0) else {
            return rec.finish(Instant::now());
        };
        changed |= !paused && (dirty || (w, h) != size);
        if paused || !changed {
            wake.wait(Some(input), None);
            continue;
        }
        let now = Instant::now();
        if now < due {
            // Only a ring cuts the rate limit short: the frame is taken
            // at `due` whatever X input arrives meanwhile.
            wake.wait(None, Some(due - now));
            continue;
        }
        let mut buf = rec.take_buf();
        if grab(w, h, &mut buf).is_err() {
            // The source closed mid-grab: keep what was recorded.
            rec.give_back(buf);
            return rec.finish(now);
        }
        let shape = Shape {
            width: w,
            height: h,
            pix,
        };
        if let Err(e) = rec.frame(buf, shape, now) {
            return Err(rec.fail(Instant::now(), e));
        }
        (changed, size) = (false, (w, h));
        // The next frame is due one interval on; a loop that fell a
        // whole interval behind starts over from this frame.
        due = if due + interval < now {
            now + interval
        } else {
            due + interval
        };
    }
}
