//! X11 per-window recording: click-to-pick a window, then record ONLY that
//! window via its XComposite backing pixmap. Occluding windows, the
//! desktop, and other applications never enter the frame — the pixmap holds
//! the window's own pixels.
//!
//! While recording, the target window wears a thin red override-redirect
//! border with an empty input shape (clicks pass straight through) plus a
//! display-only timer chip — recording state is visible on the window
//! itself rather than in a floating panel.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as CompositeExt, Redirect};
use x11rb::protocol::shape::SK as ShapeKind;
use x11rb::protocol::xfixes::ConnectionExt as XfixesExt;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt as XprotoExt, CreateWindowAux, Cursor, EventMask, Font,
    GrabMode, GrabStatus, ImageFormat, StackMode, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

use super::encoder::{Encoder, EncoderConfig};
use super::{unique_recording_path, RecordingSpec, CANCELLED_PREFIX};

const XC_CROSSHAIR: u16 = 34;
const KEYSYM_ESCAPE: u32 = 0xff1b;
const BORDER_COLOR: u32 = 0x00f7768e;
const BORDER_THICKNESS: i16 = 3;

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

/// Follow target for the recording chip: the border thread reports the
/// target window's rect, and the chip's owner repositions (and at the
/// end removes) the chip window. The GPUI daemon implements this over
/// its native chip window.
pub trait ChipFollow: Send + Sync {
    fn place(&self, rect: Rect);
    fn hide(&self);
}

fn connect() -> Result<(RustConnection, usize), String> {
    x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))
}

fn root_rect(conn: &RustConnection, root: Window, win: Window) -> Result<Rect, String> {
    // The two requests are independent: send both before awaiting
    // either reply, halving the round trips on every border wake.
    let geom_cookie = conn.get_geometry(win).map_err(|e| e.to_string())?;
    let trans_cookie = conn
        .translate_coordinates(win, root, 0, 0)
        .map_err(|e| e.to_string())?;
    let geom = geom_cookie
        .reply()
        .map_err(|e| format!("window gone: {e}"))?;
    let translated = trans_cookie
        .reply()
        .map_err(|e| format!("translate_coordinates: {e}"))?;
    Ok(Rect {
        x: translated.dst_x,
        y: translated.dst_y,
        w: geom.width,
        h: geom.height,
    })
}

fn make_crosshair(conn: &RustConnection) -> Result<Cursor, String> {
    let font: Font = conn.generate_id().map_err(|e| e.to_string())?;
    let cursor = conn.generate_id().map_err(|e| e.to_string())?;
    // X11 requests run in order, so the font open, cursor create and
    // font close can all be in flight together; checking each reply
    // serially was three round trips for one cursor.
    let open = conn.open_font(font, b"cursor").map_err(|e| e.to_string())?;
    let create = conn
        .create_glyph_cursor(
            cursor, font, font, XC_CROSSHAIR, XC_CROSSHAIR + 1,
            0, 0, 0, u16::MAX, u16::MAX, u16::MAX,
        )
        .map_err(|e| e.to_string())?;
    let close = conn.close_font(font).map_err(|e| e.to_string())?;
    open.check().map_err(|e| format!("open cursor font: {e}"))?;
    create
        .check()
        .map_err(|e| format!("create crosshair cursor: {e}"))?;
    close.check().map_err(|e| format!("close cursor font: {e}"))?;
    Ok(cursor)
}

/// Keycodes that map to Escape on this server.
fn escape_keycodes(conn: &RustConnection) -> Result<Vec<u8>, String> {
    let setup = conn.setup();
    let (lo, hi) = (setup.min_keycode, setup.max_keycode);
    let reply = conn
        .get_keyboard_mapping(lo, hi - lo + 1)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("get_keyboard_mapping: {e}"))?;
    let per = reply.keysyms_per_keycode as usize;
    let mut codes = Vec::new();
    for (i, syms) in reply.keysyms.chunks(per).enumerate() {
        if syms.contains(&KEYSYM_ESCAPE) {
            codes.push(lo + i as u8);
        }
    }
    Ok(codes)
}

/// Click-to-pick: grab pointer and keyboard, wait for a click (target) or
/// Escape (cancel). Returns the top-level window under the click. The
/// stop channel is polled between events: a stop during the pick must
/// end the wait, or the recording thread never joins and the daemon
/// wedges on the next command.
pub fn pick_window(stop: &std::sync::mpsc::Receiver<()>) -> Result<PickedWindow, String> {
    let (conn, screen_num) = connect()?;
    let root = conn.setup().roots[screen_num].root;
    let cursor = make_crosshair(&conn)?;
    let escapes = escape_keycodes(&conn)?;

    let status = conn
        .grab_pointer(
            false,
            root,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            root,
            cursor,
            CURRENT_TIME,
        )
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("grab_pointer: {e}"))?;
    if status.status != GrabStatus::SUCCESS {
        return Err(format!("pointer grab failed: {:?}", status.status));
    }
    let _kb = conn
        .grab_keyboard(false, root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)
        .map_err(|e| e.to_string())?
        .reply();

    use std::os::unix::io::AsRawFd;
    let x_fd = conn.stream().as_raw_fd();
    let picked = loop {
        match stop.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                break Err(format!("{CANCELLED_PREFIX} pick stopped"));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        // Sleep on the connection fd: an X event wakes the poll
        // instantly, and the 50ms timeout re-checks the stop channel.
        // A bare sleep would burn a wake every 10ms for nothing.
        let mut pfd = libc::pollfd {
            fd: x_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe {
            libc::poll(&mut pfd, 1, 50);
        }
        let Some(event) = conn
            .poll_for_event()
            .map_err(|e| format!("poll_for_event: {e}"))?
        else {
            continue;
        };
        match event {
            Event::ButtonPress(_) => {
                let ptr = conn
                    .query_pointer(root)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| format!("query_pointer: {e}"))?;
                let child = ptr.child;
                if child == x11rb::NONE || child == root {
                    break Err(format!(
                        "{CANCELLED_PREFIX} clicked the desktop, not a window"
                    ));
                }
                // Walk up to the top-level ancestor (direct child of root).
                let mut win = child;
                loop {
                    let tree = conn
                        .query_tree(win)
                        .map_err(|e| e.to_string())?
                        .reply()
                        .map_err(|e| format!("query_tree: {e}"))?;
                    if tree.parent == root || tree.parent == x11rb::NONE {
                        break;
                    }
                    win = tree.parent;
                }
                let geom = conn
                    .get_geometry(win)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| format!("get_geometry: {e}"))?;
                break Ok(PickedWindow {
                    id: win,
                    width: u32::from(geom.width),
                    height: u32::from(geom.height),
                });
            }
            Event::KeyPress(ev) if escapes.contains(&ev.detail) => {
                break Err(format!("{CANCELLED_PREFIX} pick aborted"));
            }
            _ => {}
        }
    };

    let _ = conn.ungrab_pointer(CURRENT_TIME);
    let _ = conn.ungrab_keyboard(CURRENT_TIME);
    let _ = conn.free_cursor(cursor);
    picked
}

/// Swizzle a BGRX/XRGB grab into `out` (resized to pixels*4), forcing
/// alpha to opaque. `out` is reused across frames so recording does not
/// allocate a fresh buffer per frame.
/// The window's native pixel format as ffmpeg's -pix_fmt, resolved
/// once at record start: the grab then memcpy's server pixels into
/// the frame buffer instead of swizzling per pixel. Returns the
/// format and the drawable depth the grabs must see.
fn resolve_pix_fmt(
    conn: &RustConnection,
    screen_num: usize,
    win: Window,
) -> Result<(super::encoder::PixFmt, u8), String> {
    use super::encoder::PixFmt;
    let screen = &conn.setup().roots[screen_num];
    if conn.setup().image_byte_order != x11rb::protocol::xproto::ImageOrder::LSB_FIRST {
        return Err("MSB-first X11 image byte order is unsupported".to_string());
    }
    let attrs = conn
        .get_window_attributes(win)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("get_window_attributes: {e}"))?;
    let geom = conn
        .get_geometry(win)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("get_geometry: {e}"))?;
    let depth = geom.depth;
    // The window's visual decides channel order: red in the low byte
    // is BGRX/BGR in memory, red in the high byte is XRGB/RGB.
    let visual = screen
        .allowed_depths
        .iter()
        .flat_map(|d| d.visuals.iter())
        .find(|v| v.visual_id == attrs.visual)
        .ok_or_else(|| format!("visual {:#x} not in screen list", attrs.visual))?;
    let bpp = conn
        .setup()
        .pixmap_formats
        .iter()
        .find(|f| f.depth == depth)
        .map(|f| f.bits_per_pixel)
        .unwrap_or(32);
    // The window's visual decides channel order: red in the high
    // bits of the pixel value lands in byte 2 of the little-endian
    // word, so memory is B,G,R,X (the common TrueColor case); red in
    // the low byte is R,G,B,X.
    let fmt = match (bpp, visual.red_mask) {
        (32, 0x00FF_0000) => PixFmt::Bgra,
        (32, 0x0000_00FF) => PixFmt::Rgba,
        (24, 0x00FF_0000) => PixFmt::Bgr24,
        (24, 0x0000_00FF) => PixFmt::Rgb24,
        (b, m) => {
            return Err(format!(
                "unsupported pixel layout: {b}bpp red_mask {m:#x} at depth {depth}"
            ))
        }
    };
    Ok((fmt, depth))
}

/// Copy `need` native-format bytes out of `src` into `out` (resized).
/// `out` is reused across frames so recording does not allocate a
/// fresh buffer per frame.
fn copy_frame_bytes(src: &[u8], need: usize, out: &mut Vec<u8>) -> Result<(), String> {
    if src.len() < need {
        return Err(format!(
            "short frame buffer: {} bytes for {need}",
            src.len()
        ));
    }
    out.clear();
    out.reserve(need);
    // Uninit capacity, not a zeroed vec: the banded copy writes every
    // byte, and a resize's memset before the copy is a wasted pass
    // per frame.
    #[allow(clippy::uninit_vec)]
    unsafe { out.set_len(need) };
    crate::par::par_bands_mut(out, 4096, |dst, start| {
        dst.copy_from_slice(&src[start..start + dst.len()]);
    });
    Ok(())
}

// ---------- recording mark: border strips + timer chip ----------

/// Visual "this window is being recorded" state. Dropping it removes the
/// border; the chip window is closed separately via hide_chip().
pub struct RecordingMark {
    stop: Sender<()>,
    join: Option<JoinHandle<()>>,
}

impl RecordingMark {
    /// Draw the border around `target` and keep it following the window
    /// (moves, resizes) until dropped or the window closes.
    pub fn show(chip: Arc<dyn ChipFollow>, target: Window) -> Result<Self, String> {
        let (tx, rx) = channel::<()>();
        let join = thread::spawn(move || border_thread(chip, target, rx));
        Ok(Self {
            stop: tx,
            join: Some(join),
        })
    }

    /// Draw the border around a fixed rect once; no follow thread, so
    /// the mark lives until dropped. The chip is placed once at the
    /// rect's top-right corner.
    pub fn show_static(chip: Arc<dyn ChipFollow>, root: Window, rect: Rect) -> Result<Self, String> {
        let (tx, rx) = channel::<()>();
        let join = thread::spawn(move || static_border_thread(chip, root, rect, rx));
        Ok(Self {
            stop: tx,
            join: Some(join),
        })
    }
}

impl Drop for RecordingMark {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Create the four override-redirect border strips on `root`. Returns
/// the strip windows; the caller destroys them.
fn make_strips(conn: &RustConnection, root: Window) -> Option<[Window; 4]> {
    let mut strips = [0u32; 4];
    let mut made = 0usize;
    for strip in &mut strips {
        let Some(win) = conn.generate_id().ok() else {
            destroy_strips(conn, &strips[..made]);
            return None;
        };
        let aux = CreateWindowAux::new()
            .background_pixel(BORDER_COLOR)
            .override_redirect(1);
        if conn
            .create_window(
                x11rb::COPY_FROM_PARENT as u8,
                win,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &aux,
            )
            .is_err()
        {
            // A failed create leaves the earlier strips mapped:
            // destroy them rather than leaking four border windows.
            destroy_strips(conn, &strips[..made]);
            return None;
        }
        // Empty input region: clicks pass through to whatever is beneath.
        let _ = conn
            .xfixes_set_window_shape_region(win, ShapeKind::INPUT, 0, 0, x11rb::NONE)
            .map(|c| c.check());
        let _ = conn.map_window(win);
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        *strip = win;
        made += 1;
    }
    let _ = conn.flush();
    Some(strips)
}

fn destroy_strips(conn: &RustConnection, strips: &[Window]) {
    for strip in strips {
        let _ = conn.destroy_window(*strip);
    }
    let _ = conn.flush();
}

/// Static variant: strips around a fixed rect, chip placed once, then
/// the thread only waits for stop.
fn static_border_thread(
    chip: Arc<dyn ChipFollow>,
    root: Window,
    rect: Rect,
    stop: std::sync::mpsc::Receiver<()>,
) {
    let Ok((conn, _)) = connect() else {
        return;
    };
    let Some(strips) = make_strips(&conn, root) else {
        return;
    };
    place_strips(&conn, &strips, rect);
    chip.place(rect);
    loop {
        // recv_timeout wakes the instant stop fires; a bare sleep
        // would leave the strips up for up to 200ms past it.
        if stop.recv_timeout(Duration::from_millis(200)).is_ok() {
            break;
        }
    }
    destroy_strips(&conn, &strips);
}

fn border_thread(chip: Arc<dyn ChipFollow>, target: Window, stop: std::sync::mpsc::Receiver<()>) {
    let Ok((conn, screen_num)) = connect() else {
        return;
    };
    let root = conn.setup().roots[screen_num].root;

    let Some(strips) = make_strips(&conn, root) else {
        return;
    };

    // Follow the target by event, not by polling: StructureNotify on
    // the target delivers ConfigureNotify on every move/resize, so the
    // 200ms get_geometry+translate round trips become a poll on the
    // connection's fd that wakes only when the window actually moves.
    let _ = conn.change_window_attributes(
        target,
        &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
            .event_mask(EventMask::STRUCTURE_NOTIFY),
    );
    let _ = conn.flush();
    use std::os::unix::io::AsRawFd;
    let x_fd = conn.stream().as_raw_fd();

    let mut last: Option<Rect> = None;
    let mut settle = 0u32;
    loop {
        if stop.try_recv().is_ok() {
            break;
        }
        let mut gone = false;
        let mut woke = false;
        while let Ok(Some(event)) = conn.poll_for_event() {
            woke = true;
            if let Event::DestroyNotify(ev) = event {
                if ev.window == target {
                    gone = true;
                }
            }
        }
        if gone {
            break;
        }
        // The rect query is two round trips. It is needed on the first
        // pass (no rect yet), on every event wake (a ConfigureNotify
        // means the window moved), and through the ~2s settle window
        // (the WM's map-time re-framing lands without a notify we can
        // trust). After that a timeout wake means nothing moved, so
        // the query is pure waste for the recording's life.
        let need_rect = last.is_none() || woke || settle < 10;
        if need_rect {
            match root_rect(&conn, root, target) {
                Ok(rect) => {
                    let moved = last != Some(rect);
                    if moved {
                        place_strips(&conn, &strips, rect);
                        last = Some(rect);
                    }
                    // Re-assert the chip for the first ~2s: the WM can
                    // re-place it on map-time re-framing after the
                    // initial rect report. After that, only a real
                    // move re-places.
                    if moved || settle < 10 {
                        chip.place(rect);
                    }
                }
                Err(_) => break, // target closed; recording ends on its own
            }
            settle += 1;
        }
        // Sleep until the next X event or the 200ms re-check: a
        // ConfigureNotify wakes the poll instantly, so a moved window
        // re-borders in the same frame instead of up to 200ms late.
        let mut pfd = libc::pollfd {
            fd: x_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe {
            libc::poll(&mut pfd, 1, 200);
        }
    }

    destroy_strips(&conn, &strips);
}

fn place_strips(conn: &RustConnection, strips: &[Window; 4], r: Rect) {
    let t = BORDER_THICKNESS;
    // top, bottom: full width incl. corners; left, right: between them.
    let geoms = [
        (r.x - t, r.y - t, r.w as i16 + 2 * t, t),
        (r.x - t, r.y + r.h as i16, r.w as i16 + 2 * t, t),
        (r.x - t, r.y, t, r.h as i16),
        (r.x + r.w as i16, r.y, t, r.h as i16),
    ];
    for (win, (x, y, w, h)) in strips.iter().zip(geoms) {
        let _ = conn.configure_window(
            *win,
            &ConfigureWindowAux::new()
                .x(i32::from(x))
                .y(i32::from(y))
                .width(w.max(1) as u32)
                .height(h.max(1) as u32)
                .stack_mode(StackMode::ABOVE),
        );
    }
    let _ = conn.flush();
}

// ---------- recording ----------

/// Record the picked window until `spec.stop` fires, with the chip owned
/// by the caller through `chip`. This is the toolkit-independent entry
/// point used by the GPUI daemon.
pub fn record_window_follow(spec: RecordingSpec, chip: Arc<dyn ChipFollow>) -> Result<PathBuf, String> {
    let chip_inner = chip.clone();
    let result = (move || {
        let picked = pick_window(&spec.stop)?;
        let _mark = RecordingMark::show(chip_inner, picked.id);
        record_target(&picked, &spec)
    })();
    // On every exit — cancel, error, or clean stop — the chip goes away.
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

/// A persistent MIT-SHM segment for per-frame grabs. Recreated when the
/// target resizes; falls back to socket GetImage when SHM is absent.
struct ShmGrab {
    shmid: i32,
    addr: *mut u8,
    seg: u32,
    size: usize,
}

impl ShmGrab {
    fn new(conn: &RustConnection, width: u32, height: u32, bpp: usize) -> Option<Self> {
        use x11rb::protocol::shm::ConnectionExt as ShmExt;
        let version = ShmExt::shm_query_version(conn).ok()?.reply().ok()?;
        if version.major_version < 1 {
            return None;
        }
        let size = width as usize * height as usize * bpp;
        unsafe {
            let shmid = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
            if shmid < 0 {
                return None;
            }
            libc::shmctl(shmid, libc::IPC_RMID, std::ptr::null_mut());
            let addr = libc::shmat(shmid, std::ptr::null(), 0);
            if addr as isize == -1 {
                return None;
            }
            let Some(seg) = conn.generate_id().ok() else {
                libc::shmdt(addr);
                return None;
            };
            if ShmExt::shm_attach(conn, seg, shmid as u32, false)
                .ok()
                .and_then(|c| c.check().ok())
                .is_none()
            {
                // The segment is already marked IPC_RMID, but the
                // mapping must still be detached or it leaks.
                libc::shmdt(addr);
                return None;
            }
            Some(Self { shmid, addr: addr as *mut u8, seg, size })
        }
    }


    fn detach(&self, conn: &RustConnection) {
        use x11rb::protocol::shm::ConnectionExt as ShmExt;
        unsafe {
            libc::shmdt(self.addr as *const _);
        }
        let _ = ShmExt::shm_detach(conn, self.seg);
        let _ = self.shmid;
    }
}

/// A named composite pixmap for the recording target, cached across
/// frames: name_window_pixmap + free_pixmap is two round trips per
/// frame otherwise. Re-keyed on resize; the server frees the storage
/// when the window is unredirected or dies.
struct NamedPixmap {
    win: Window,
    width: u32,
    height: u32,
    depth: u8,
    pixmap: x11rb::protocol::xproto::Pixmap,
}

impl NamedPixmap {
    fn for_window(
        conn: &RustConnection,
        win: Window,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let pixmap = conn.generate_id().map_err(|e| e.to_string())?;
        conn.composite_name_window_pixmap(win, pixmap)
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| format!("name_window_pixmap (window closed?): {e}"))?;
        let depth = conn
            .get_geometry(pixmap)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|g| g.depth)
            .unwrap_or(24);
        Ok(Self { win, width, height, depth, pixmap })
    }

    fn free(&self, conn: &RustConnection) {
        let _ = conn.free_pixmap(self.pixmap);
    }
}

fn grab_pixmap(
    conn: &RustConnection,
    shm: &mut Option<ShmGrab>,
    named: &mut Option<NamedPixmap>,
    win: Window,
    width: u32,
    height: u32,
    depth: u8,
    bpp: usize,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    // The named pixmap is cached per (window, size): naming and freeing
    // per frame is two round trips the schedule cannot spare.
    let stale = named
        .as_ref()
        .map(|n| n.win != win || n.width != width || n.height != height)
        .unwrap_or(true);
    if stale {
        if let Some(old) = named.take() {
            old.free(conn);
        }
        *named = Some(NamedPixmap::for_window(conn, win, width, height)?);
    }
    let named = named.as_mut().unwrap();
    if named.depth != depth {
        return Err(format!(
            "window pixmap depth {} differs from window depth {depth}",
            named.depth
        ));
    }
    let pixmap = named.pixmap;

    // SHM path: the server writes the frame into the mapped segment and
    // the copy reads it in place. A 1080p frame over the socket is
    // ~8MB of protocol traffic per frame; this is none.
    let need = width as usize * height as usize * bpp;
    if shm.as_ref().map(|s| s.size) != Some(need) {
        if let Some(old) = shm.take() {
            old.detach(conn);
        }
        *shm = ShmGrab::new(conn, width, height, bpp);
    }
    if let Some(s) = shm.as_ref() {
        use x11rb::protocol::shm::ConnectionExt as ShmExt;
        let grabbed = ShmExt::shm_get_image(
            conn, pixmap, 0, 0, width as u16, height as u16, !0u32,
            ImageFormat::Z_PIXMAP.into(), s.seg, 0,
        )
        .ok()
        .and_then(|c| c.reply().ok());
        if let Some(reply) = grabbed {
            if reply.depth != depth {
                return Err(format!(
                    "shm grab returned depth {} (expected {depth})",
                    reply.depth
                ));
            }
            let src = unsafe { std::slice::from_raw_parts(s.addr as *const u8, s.size) };
            return copy_frame_bytes(src, need, out);
        }
        // SHM failed mid-session: drop it and fall through to the socket.
        if let Some(old) = shm.take() {
            old.detach(conn);
        }
    }

    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            pixmap,
            0,
            0,
            width as u16,
            height as u16,
            !0u32,
        )
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("get_image on window pixmap: {e}"))?;
    if image.depth != depth {
        return Err(format!(
            "get_image returned depth {} (expected {depth})",
            image.depth
        ));
    }
    copy_frame_bytes(&image.data, need, out)
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
    let mut guard = GrabGuard { conn, shm: None, named: None };

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
    let probe = || {
        loop {
            match conn.poll_for_event() {
                Ok(Some(Event::ConfigureNotify(ev))) if ev.window == picked.id => {
                    dims = Some((u32::from(ev.width), u32::from(ev.height)));
                }
                Ok(Some(Event::DestroyNotify(ev))) if ev.window == picked.id => {
                    return None;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => return None, // connection died
            }
        }
        dims
    };
    let grab = |w: u32, h: u32, rgba: &mut Vec<u8>| {
        grab_pixmap(conn, &mut guard.shm, &mut guard.named, picked.id, w, h, depth, bpp, rgba)
    };
    record_loop_inner(spec, pix_fmt, probe, grab)
}

/// Record a fixed screen region until `spec.stop` fires. The overlay
/// picks the rect; this grabs it straight off the root window through
/// the same SHM path, with a static border and the chip parked at the
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
        let mut guard = ShmGuard { conn: &conn, shm: None };
        let probe = || Some((rect.w as u32, rect.h as u32));
        let grab = |w: u32, h: u32, rgba: &mut Vec<u8>| {
            grab_root_rect(&conn, &mut guard.shm, root, rect, w, h, depth, bpp, rgba)
        };
        record_loop_inner(&spec, pix_fmt, probe, grab)
    })();
    chip.hide();
    result
}

/// Grab a rect of the root window through SHM (or the socket when SHM
/// is absent). No composite pixmap: the region is read live, so other
/// windows moving through it appear in the recording.
fn grab_root_rect(
    conn: &RustConnection,
    shm: &mut Option<ShmGrab>,
    root: Window,
    rect: Rect,
    width: u32,
    height: u32,
    depth: u8,
    bpp: usize,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let need = width as usize * height as usize * bpp;
    if shm.as_ref().map(|s| s.size) != Some(need) {
        if let Some(old) = shm.take() {
            old.detach(conn);
        }
        *shm = ShmGrab::new(conn, width, height, bpp);
    }
    if let Some(s) = shm.as_ref() {
        use x11rb::protocol::shm::ConnectionExt as ShmExt;
        let grabbed = ShmExt::shm_get_image(
            conn, root, rect.x, rect.y, width as u16, height as u16, !0u32,
            ImageFormat::Z_PIXMAP.into(), s.seg, 0,
        )
        .ok()
        .and_then(|c| c.reply().ok());
        if let Some(reply) = grabbed {
            if reply.depth != depth {
                return Err(format!(
                    "shm grab returned depth {} (expected {depth})",
                    reply.depth
                ));
            }
            let src = unsafe { std::slice::from_raw_parts(s.addr as *const u8, s.size) };
            return copy_frame_bytes(src, need, out);
        }
        if let Some(old) = shm.take() {
            old.detach(conn);
        }
    }
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            rect.x,
            rect.y,
            width as u16,
            height as u16,
            !0u32,
        )
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("get_image on root region: {e}"))?;
    if image.depth != depth {
        return Err(format!(
            "get_image returned depth {} (expected {depth})",
            image.depth
        ));
    }
    copy_frame_bytes(&image.data, need, out)
}

/// The shared recording loop: chip controls, the absolute frame
/// schedule, encoder splits on resize, and the zero-copy frame queue.
/// `probe` reports the source's current dimensions (None = source
/// gone, a clean end); `grab` fills `rgba` with one frame. Returns the
/// path of the LAST segment written: splits rename the output, and the
/// caller must report the file that actually holds the tail.
fn record_loop_inner(
    spec: &RecordingSpec,
    pix_fmt: super::encoder::PixFmt,
    mut probe: impl FnMut() -> Option<(u32, u32)>,
    mut grab: impl FnMut(u32, u32, &mut Vec<u8>) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let Some((mut width, mut height)) = probe() else {
        return Err("recording source gone before first frame".to_string());
    };
    if width == 0 || height == 0 {
        return Err("recording source has zero size".to_string());
    }
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
        // same way a resize does.
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
                }
            }
        }

        // Detect resize / close each frame. A vanished source is a clean
        // end of the recording, not an error; so is a zero-size probe
        // (a minimized window reports 0x0 and would kill the encoder).
        let Some((w, h)) = probe() else {
            encoder.finish()?;
            return Ok(output);
        };
        if w == 0 || h == 0 {
            encoder.finish()?;
            return Ok(output);
        }
        if w != width || h != height {
            width = w;
            height = h;
            split_encoder!();
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
