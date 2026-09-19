//! X11 per-window recording: click-to-pick a window, then record ONLY that
//! window via its XComposite backing pixmap. Occluding windows, the
//! desktop, and other applications never enter the frame — the pixmap holds
//! the window's own pixels.
//!
//! While recording, the target window wears a thin red override-redirect
//! border with an empty input shape (clicks pass straight through) plus a
//! display-only timer chip — recording state is visible on the window
//! itself rather than in a floating panel.

use std::path::{Path, PathBuf};
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
    let geom = conn
        .get_geometry(win)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("window gone: {e}"))?;
    let translated = conn
        .translate_coordinates(win, root, 0, 0)
        .map_err(|e| e.to_string())?
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
    conn.open_font(font, b"cursor")
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("open cursor font: {e}"))?;
    let cursor = conn.generate_id().map_err(|e| e.to_string())?;
    conn.create_glyph_cursor(
        cursor, font, font, XC_CROSSHAIR, XC_CROSSHAIR + 1,
        0, 0, 0, u16::MAX, u16::MAX, u16::MAX,
    )
    .map_err(|e| e.to_string())?
    .check()
    .map_err(|e| format!("create crosshair cursor: {e}"))?;
    conn.close_font(font).map_err(|e| e.to_string())?;
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
/// Escape (cancel). Returns the top-level window under the click.
pub fn pick_window() -> Result<PickedWindow, String> {
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

    let picked = loop {
        let event = conn.wait_for_event().map_err(|e| format!("wait_for_event: {e}"))?;
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
fn bgrx_to_rgba(data: &[u8], pixels: usize, out: &mut Vec<u8>) -> Result<(), String> {
    let bpp = data.len() / pixels.max(1);
    // resize without clear(): once sized, this is a no-op and does not
    // re-zero a buffer every byte of which the swizzle overwrites.
    out.resize(pixels * 4, 0);
    match bpp {
        4 => {
            // Parallel word-level swizzle: at 4K a single-threaded
            // per-pixel loop is a visible slice of the frame budget and
            // drops frames. B,G,R,_ -> R,G,B,255 swaps bytes 0<->2.
            const PARALLEL_MIN: usize = 1 << 20; // ~1 MP
            if pixels < PARALLEL_MIN {
                for (o, px) in out.chunks_exact_mut(4).zip(data.chunks_exact(4)) {
                    let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                    let rgb = (v & 0xFF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF);
                    o.copy_from_slice(&(rgb | 0xFF00_0000).to_le_bytes());
                }
            } else {
                let threads = std::thread::available_parallelism()
                    .map(|n| n.get().min(8))
                    .unwrap_or(4);
                let chunk_px = pixels.div_ceil(threads);
                std::thread::scope(|scope| {
                    let mut out_rest = out.as_mut_slice();
                    let mut in_rest = data;
                    for _ in 0..threads {
                        let take_px = chunk_px.min(in_rest.len() / 4);
                        if take_px == 0 {
                            break;
                        }
                        let (o_chunk, o_rest) = out_rest.split_at_mut(take_px * 4);
                        let (i_chunk, i_rest) = in_rest.split_at(take_px * 4);
                        out_rest = o_rest;
                        in_rest = i_rest;
                        scope.spawn(move || {
                            for (o, px) in o_chunk.chunks_exact_mut(4).zip(i_chunk.chunks_exact(4)) {
                                let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                                let rgb = (v & 0xFF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF);
                                o.copy_from_slice(&(rgb | 0xFF00_0000).to_le_bytes());
                            }
                        });
                    }
                });
            }
        }
        3 => {
            for (o, px) in out.chunks_exact_mut(4).zip(data.chunks_exact(3)) {
                o.copy_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        other => return Err(format!("unsupported bytes-per-pixel {other}")),
    }
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
    for strip in &mut strips {
        let win = conn.generate_id().ok()?;
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
            return None;
        }
        // Empty input region: clicks pass through to whatever is beneath.
        let _ = conn
            .xfixes_set_window_shape_region(win, ShapeKind::INPUT, 0, 0, x11rb::NONE)
            .map(|c| c.check());
        let _ = conn.map_window(win);
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        *strip = win;
    }
    let _ = conn.flush();
    Some(strips)
}

fn destroy_strips(conn: &RustConnection, strips: &[Window; 4]) {
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
        if stop.try_recv().is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(200));
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

    let mut last: Option<Rect> = None;
    loop {
        if stop.try_recv().is_ok() {
            break;
        }
        match root_rect(&conn, root, target) {
            Ok(rect) => {
                if last != Some(rect) {
                    place_strips(&conn, &strips, rect);
                    last = Some(rect);
                }
                // Every poll: the WM may place the chip on first map,
                // after the initial rect report.
                chip.place(rect);
            }
            Err(_) => break, // target closed; recording ends on its own
        }
        thread::sleep(Duration::from_millis(200));
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
pub fn record_window_follow(spec: RecordingSpec, chip: Arc<dyn ChipFollow>) -> Result<(), String> {
    let chip_inner = chip.clone();
    let result = (move || {
        let picked = pick_window()?;
        let _mark = RecordingMark::show(chip_inner, picked.id);
        record_target(&picked, &spec)
    })();
    // On every exit — cancel, error, or clean stop — the chip goes away.
    chip.hide();
    result
}

fn record_target(picked: &PickedWindow, spec: &RecordingSpec) -> Result<(), String> {
    let (conn, _) = connect()?;

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

    let result = record_loop(&conn, picked, spec);

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
    fn new(conn: &RustConnection, width: u32, height: u32) -> Option<Self> {
        use x11rb::protocol::shm::ConnectionExt as ShmExt;
        let version = ShmExt::shm_query_version(conn).ok()?.reply().ok()?;
        if version.major_version < 1 {
            return None;
        }
        let size = width as usize * height as usize * 4;
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
            let seg = conn.generate_id().ok()?;
            ShmExt::shm_attach(conn, seg, shmid as u32, false).ok()?.check().ok()?;
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

fn grab_pixmap(
    conn: &RustConnection,
    shm: &mut Option<ShmGrab>,
    win: Window,
    width: u32,
    height: u32,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let pixmap = conn.generate_id().map_err(|e| e.to_string())?;
    conn.composite_name_window_pixmap(win, pixmap)
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("name_window_pixmap (window closed?): {e}"))?;

    // SHM path: the server writes the frame into the mapped segment and
    // the swizzle reads it in place. A 1080p frame over the socket is
    // ~8MB of protocol traffic per frame; this is none.
    let need = width as usize * height as usize * 4;
    if shm.as_ref().map(|s| s.size) != Some(need) {
        if let Some(old) = shm.take() {
            old.detach(conn);
        }
        *shm = ShmGrab::new(conn, width, height);
    }
    if let Some(s) = shm.as_ref() {
        use x11rb::protocol::shm::ConnectionExt as ShmExt;
        let grabbed = ShmExt::shm_get_image(
            conn, pixmap, 0, 0, width as u16, height as u16, !0u32,
            ImageFormat::Z_PIXMAP.into(), s.seg, 0,
        )
        .ok()
        .and_then(|c| c.reply().ok());
        if let Some(_reply) = grabbed {
            let src = unsafe { std::slice::from_raw_parts(s.addr as *const u8, s.size) };
            let r = bgrx_to_rgba(src, width as usize * height as usize, out);
            conn.free_pixmap(pixmap).map_err(|e| e.to_string())?;
            return r;
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
    conn.free_pixmap(pixmap).map_err(|e| e.to_string())?;
    bgrx_to_rgba(&image.data, width as usize * height as usize, out)
}


/// The container extension of the current output path, so a mid-recording
/// split (resize, mic toggle) keeps the same format.
fn ext_of(path: &Path) -> &str {
    path.extension().and_then(|e| e.to_str()).unwrap_or("mp4")
}

fn record_loop(
    conn: &RustConnection,
    picked: &PickedWindow,
    spec: &RecordingSpec,
) -> Result<(), String> {
    // Persistent SHM segment for per-frame grabs; None when the server
    // lacks MIT-SHM, in which case grab_pixmap uses the socket. The
    // guard detaches on every exit path, including early returns.
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
    let mut guard = ShmGuard { conn, shm: None };

    let probe = || {
        conn.get_geometry(picked.id)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|g| (u32::from(g.width), u32::from(g.height)))
    };
    let grab = |w: u32, h: u32, rgba: &mut Vec<u8>| {
        grab_pixmap(conn, &mut guard.shm, picked.id, w, h, rgba)
    };
    record_loop_inner(spec, probe, grab)
}

/// Record a fixed screen region until `spec.stop` fires. The overlay
/// picks the rect; this grabs it straight off the root window through
/// the same SHM path, with a static border and the chip parked at the
/// region's top-right corner.
pub fn record_region(
    spec: RecordingSpec,
    chip: Arc<dyn ChipFollow>,
    rect: Rect,
) -> Result<(), String> {
    let chip_inner = chip.clone();
    let result = (move || {
        let (conn, screen_num) = connect()?;
        let root = conn.setup().roots[screen_num].root;
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
            grab_root_rect(&conn, &mut guard.shm, root, rect, w, h, rgba)
        };
        record_loop_inner(&spec, probe, grab)
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
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let need = width as usize * height as usize * 4;
    if shm.as_ref().map(|s| s.size) != Some(need) {
        if let Some(old) = shm.take() {
            old.detach(conn);
        }
        *shm = ShmGrab::new(conn, width, height);
    }
    if let Some(s) = shm.as_ref() {
        use x11rb::protocol::shm::ConnectionExt as ShmExt;
        let grabbed = ShmExt::shm_get_image(
            conn, root, rect.x, rect.y, width as u16, height as u16, !0u32,
            ImageFormat::Z_PIXMAP.into(), s.seg, 0,
        )
        .ok()
        .and_then(|c| c.reply().ok());
        if grabbed.is_some() {
            let src = unsafe { std::slice::from_raw_parts(s.addr as *const u8, s.size) };
            return bgrx_to_rgba(src, width as usize * height as usize, out);
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
    bgrx_to_rgba(&image.data, width as usize * height as usize, out)
}

/// The shared recording loop: chip controls, the absolute frame
/// schedule, encoder splits on resize, and the zero-copy frame queue.
/// `probe` reports the source's current dimensions (None = source
/// gone, a clean end); `grab` fills `rgba` with one frame.
fn record_loop_inner(
    spec: &RecordingSpec,
    mut probe: impl FnMut() -> Option<(u32, u32)>,
    mut grab: impl FnMut(u32, u32, &mut Vec<u8>) -> Result<(), String>,
) -> Result<(), String> {
    let Some((mut width, mut height)) = probe() else {
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
    })?;

    let frame_interval = Duration::from_secs_f64(1.0 / f64::from(spec.fps));
    let mut start = Instant::now();
    let mut mic = spec.mic;
    let mut frame_no: u64 = 0;
    // Reused RGBA scratch: recording would otherwise allocate a fresh
    // w*h*4 buffer on every frame.
    let mut rgba: Vec<u8> = Vec::new();

    loop {
        if spec.stop.try_recv().is_ok() {
            encoder.finish()?;
            return Ok(());
        }

        // Chip controls: pause blocks the schedule (the mp4 simply has
        // no frames for the paused span), mic toggle splits the file the
        // same way a resize does.
        while let Ok(ctl) = spec.control.try_recv() {
            match ctl {
                super::RecControl::Pause => {
                    let paused_at = Instant::now();
                    loop {
                        if spec.stop.try_recv().is_ok() {
                            encoder.finish()?;
                            return Ok(());
                        }
                        match spec.control.recv_timeout(Duration::from_millis(100)) {
                            Ok(super::RecControl::Resume) => break,
                            Ok(super::RecControl::ToggleMic) => {
                                mic = !mic;
                                encoder.finish()?;
                                let dir = output.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
                                output = unique_recording_path(&dir, ext_of(&output));
                                encoder = Encoder::start(&EncoderConfig { output: output.clone(), width, height, fps: spec.fps, mic, format: spec.format, encoder: spec.encoder })?;
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
                    encoder.finish()?;
                    let dir = output.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
                    output = unique_recording_path(&dir, ext_of(&output));
                    encoder = Encoder::start(&EncoderConfig { output: output.clone(), width, height, fps: spec.fps, mic, format: spec.format, encoder: spec.encoder })?;
                }
            }
        }


        // Detect resize / close each frame. A vanished source is a clean
        // end of the recording, not an error.
        let Some((w, h)) = probe() else {
            encoder.finish()?;
            return Ok(());
        };
        if w != width || h != height {
            encoder.finish()?;
            width = w;
            height = h;
            let dir = output
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            output = unique_recording_path(&dir, ext_of(&output));
            encoder = Encoder::start(&EncoderConfig {
                output: output.clone(),
                width,
                height,
                fps: spec.fps,
                mic: spec.mic,
                format: spec.format,
                encoder: spec.encoder,
            })?;
        }

        // Absolute schedule: frame n is due at start + n/fps. On overrun,
        // skip the counter forward (drop) instead of bursting.
        let due = start + frame_interval * frame_no as u32;
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
                return Ok(());
            }
        };
        // The frame buffer moves to the writer thread; the next frame
        // fills a recycled one.
        let frame = std::mem::replace(&mut rgba, encoder.take_buf());
        encoder.write_frame(frame)?;
        frame_no += 1;
    }
}
