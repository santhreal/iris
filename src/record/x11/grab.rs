use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::composite::ConnectionExt as CompositeExt;
use x11rb::protocol::damage::{self, ConnectionExt as DamageExt, ReportLevel};
use x11rb::protocol::shm::ConnectionExt as ShmExt;
use x11rb::protocol::xproto::{
    ConnectionExt as XprotoExt, ImageFormat, ImageOrder, Pixmap, Window,
};
use x11rb::rust_connection::RustConnection;

use crate::record::encoder::PixFmt;

use super::{FrameGeom, Rect};

/// The window's native pixel format as ffmpeg's -pix_fmt, resolved
/// once at record start: the grab then memcpy's server pixels into
/// the frame buffer instead of swizzling per pixel. Returns the
/// format and the drawable depth the grabs must see.
pub(super) fn resolve_pix_fmt(
    conn: &RustConnection,
    screen_num: usize,
    win: Window,
) -> Result<(PixFmt, u8), String> {
    let screen = &conn.setup().roots[screen_num];
    if conn.setup().image_byte_order != ImageOrder::LSB_FIRST {
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
    unsafe {
        out.set_len(need)
    };
    crate::par::par_bands_mut(out, 4096, |dst, start| {
        dst.copy_from_slice(&src[start..start + dst.len()]);
    });
    Ok(())
}

/// A persistent MIT-SHM segment for per-frame grabs. Recreated when the
/// target resizes; falls back to socket GetImage when SHM is absent.
pub(super) struct ShmGrab {
    shmid: i32,
    addr: *mut u8,
    seg: u32,
    size: usize,
}

impl ShmGrab {
    pub(super) fn new(conn: &RustConnection, width: u32, height: u32, bpp: usize) -> Option<Self> {
        if !crate::capture::x11::shm_supported() {
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
            Some(Self {
                shmid,
                addr: addr as *mut u8,
                seg,
                size,
            })
        }
    }

    pub(super) fn detach(&self, conn: &RustConnection) {
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
pub(super) struct NamedPixmap {
    win: Window,
    width: u32,
    height: u32,
    pub(super) depth: u8,
    pixmap: Pixmap,
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
        Ok(Self {
            win,
            width,
            height,
            depth,
            pixmap,
        })
    }

    pub(super) fn free(&self, conn: &RustConnection) {
        let _ = conn.free_pixmap(self.pixmap);
    }
}

pub(super) fn grab_pixmap(
    conn: &RustConnection,
    shm: &mut Option<ShmGrab>,
    named: &mut Option<NamedPixmap>,
    win: Window,
    geom: FrameGeom,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let (width, height, depth, bpp) = (geom.width, geom.height, geom.depth, geom.bpp);
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
    let need = geom.bytes();
    if shm.as_ref().map(|s| s.size) != Some(need) {
        if let Some(old) = shm.take() {
            old.detach(conn);
        }
        *shm = ShmGrab::new(conn, width, height, bpp);
    }
    if let Some(s) = shm.as_ref() {
        let grabbed = ShmExt::shm_get_image(
            conn,
            pixmap,
            0,
            0,
            width as u16,
            height as u16,
            !0u32,
            ImageFormat::Z_PIXMAP.into(),
            s.seg,
            0,
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

/// XDamage subscription for the recording source. RAW_RECTANGLES
/// delivers one Notify per damaged region with its area, so the dirty
/// flag is exact: set by an event intersecting the record rect,
/// cleared after the grab. `None` when the extension is absent, which
/// degrades to grabbing every frame.
pub(super) struct DamageWatch {
    id: damage::Damage,
    /// Damage outside this rect does not dirty the frame; None means
    /// the whole drawable counts (window recording).
    rect: Option<Rect>,
    dirty: bool,
}

impl DamageWatch {
    /// Subscribe to `drawable`. Returns None when the extension or the
    /// create request fails: the caller then grabs every frame.
    pub(super) fn arm(
        conn: &RustConnection,
        drawable: x11rb::protocol::xproto::Drawable,
        rect: Option<Rect>,
    ) -> Option<Self> {
        conn.extension_information(damage::X11_EXTENSION_NAME)
            .ok()??;
        DamageExt::damage_query_version(conn, 1, 1)
            .ok()?
            .reply()
            .ok()?;
        let id = conn.generate_id().ok()?;
        DamageExt::damage_create(conn, id, drawable, ReportLevel::RAW_RECTANGLES)
            .ok()?
            .check()
            .ok()?;
        // First frame always grabs.
        Some(Self {
            id,
            rect,
            dirty: true,
        })
    }

    /// Fold one DamageNotify into the dirty flag. Called from the
    /// same drain that tracks ConfigureNotify, so events are consumed
    /// exactly once.
    pub(super) fn note(&mut self, ev: &damage::NotifyEvent) {
        if ev.damage != self.id {
            return;
        }
        if let Some(r) = self.rect {
            let a = &ev.area;
            let (ax, ay) = (i32::from(a.x), i32::from(a.y));
            let (aw, ah) = (i32::from(a.width), i32::from(a.height));
            let (rx, ry) = (i32::from(r.x), i32::from(r.y));
            let (rw, rh) = (i32::from(r.w), i32::from(r.h));
            let hit = ax < rx + rw && ax + aw > rx && ay < ry + rh && ay + ah > ry;
            if !hit {
                return;
            }
        }
        self.dirty = true;
    }

    /// True when the source changed since the last call. The subtract
    /// keeps the server-side region from growing without bound; it is
    /// hygiene, not correctness, since RAW_RECTANGLES events do not
    /// depend on the accumulated region.
    pub(super) fn take_dirty(&mut self, conn: &RustConnection) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;
        let _ = DamageExt::damage_subtract(conn, self.id, x11rb::NONE, x11rb::NONE);
        let _ = conn.flush();
        true
    }

    pub(super) fn release(&self, conn: &RustConnection) {
        let _ = DamageExt::damage_destroy(conn, self.id);
    }
}

/// Grab a rect of the root window through SHM (or the socket when SHM
/// is absent). No composite pixmap: the region is read live, so other
/// windows moving through it appear in the recording.
pub(super) fn grab_root_rect(
    conn: &RustConnection,
    shm: &mut Option<ShmGrab>,
    root: Window,
    rect: Rect,
    geom: FrameGeom,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let (width, height, depth, bpp) = (geom.width, geom.height, geom.depth, geom.bpp);
    let need = geom.bytes();
    if shm.as_ref().map(|s| s.size) != Some(need) {
        if let Some(old) = shm.take() {
            old.detach(conn);
        }
        *shm = ShmGrab::new(conn, width, height, bpp);
    }
    if let Some(s) = shm.as_ref() {
        let grabbed = ShmExt::shm_get_image(
            conn,
            root,
            rect.x,
            rect.y,
            width as u16,
            height as u16,
            !0u32,
            ImageFormat::Z_PIXMAP.into(),
            s.seg,
            0,
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
