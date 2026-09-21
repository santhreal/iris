use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

use super::{CaptureBackend, Frame, WinRect};

mod query;
mod shm;

pub use query::{active_window_rect, layout, list_top_level_windows, monitors, shared_conn};
pub(crate) use shm::shm_supported;
use shm::try_shm_grab_into;

/// X11 full-screen capture via GetImage on the root window.
///
/// The root window spans the whole virtual screen, so one grab covers every
/// monitor; the frozen frame's coordinates match root coordinates, which is
/// what the overlay selection reports back.
pub struct X11Backend;

impl X11Backend {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }
}

impl CaptureBackend for X11Backend {
    fn grab_screen(&self) -> Result<Frame, String> {
        let (conn, screen_num) = shared_conn()?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

        let geom = conn
            .get_geometry(root)
            .map_err(|e| format!("X11 get_geometry: {e}"))?
            .reply()
            .map_err(|e| format!("X11 get_geometry reply: {e}"))?;
        let width = u32::from(geom.width);
        let height = u32::from(geom.height);

        let pixels = width as usize * height as usize;
        // Uninit capacity, not a zeroed vec: the grab writes every byte
        // and a 33MB memset before a 33MB fill is a wasted pass. On
        // failure the buffer drops without ever being read.
        let mut rgba: Vec<u8> = Vec::with_capacity(pixels * 4);
        // The grab writes every byte before any read.
        #[allow(clippy::uninit_vec)]
        unsafe {
            rgba.set_len(pixels * 4)
        };
        let depth = grab_pixels_into(&conn, root, 0, 0, geom.width, geom.height, &mut rgba, false)?;

        if depth != 24 {
            return Err(format!(
                "unsupported root depth {}; only 24-bit TrueColor is implemented",
                depth
            ));
        }

        Ok(Frame {
            width,
            height,
            rgba,
        })
    }
}

/// Grab the whole root into a fresh BGRA buffer: the overlay's
/// GPU-bound frame consumes BGRA, so this skips the RGBA intermediate
/// and the second swizzle `slice_frame` would otherwise run. Same
/// SHM/GetImage path as `grab_screen`, only the output order differs.
pub fn grab_screen_bgra() -> Result<(u32, u32, Vec<u8>), String> {
    let (conn, screen_num) = shared_conn()?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let geom = conn
        .get_geometry(root)
        .map_err(|e| format!("X11 get_geometry: {e}"))?
        .reply()
        .map_err(|e| format!("X11 get_geometry reply: {e}"))?;
    let width = u32::from(geom.width);
    let height = u32::from(geom.height);
    let pixels = width as usize * height as usize;
    let mut bgra: Vec<u8> = Vec::with_capacity(pixels * 4);
    #[allow(clippy::uninit_vec)] // the grab writes every byte
    unsafe {
        bgra.set_len(pixels * 4)
    };
    let depth = grab_pixels_into(&conn, root, 0, 0, geom.width, geom.height, &mut bgra, true)?;
    if depth != 24 {
        return Err(format!(
            "unsupported root depth {}; only 24-bit TrueColor is implemented",
            depth
        ));
    }
    Ok((width, height, bgra))
}

/// Grab one rect of the root window into a fresh RGBA buffer: the
/// window-capture path reads only the target's pixels (decorations
/// included, since the rect comes off the root) instead of grabbing
/// the whole screen and cropping. Returns None when the rect is
/// empty or the grab fails at the protocol level.
pub fn grab_root_rect(rect: WinRect) -> Result<Frame, String> {
    let (conn, screen_num) = shared_conn()?;
    let root = conn.setup().roots[screen_num].root;
    // Intersect with the root: a window hanging off the screen edge
    // must not ask GetImage for pixels outside the drawable, and the
    // i16 protocol fields must not wrap on a wide virtual screen.
    let geom = conn
        .get_geometry(root)
        .map_err(|e| format!("X11 get_geometry: {e}"))?
        .reply()
        .map_err(|e| format!("X11 get_geometry reply: {e}"))?;
    let (rw, rh) = (i32::from(geom.width), i32::from(geom.height));
    let x0 = rect.x.clamp(0, rw);
    let y0 = rect.y.clamp(0, rh);
    let x1 = (rect.x + rect.width as i32).clamp(0, rw);
    let y1 = (rect.y + rect.height as i32).clamp(0, rh);
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0 || h <= 0 {
        return Err("capture rect is outside the screen".to_string());
    }
    let (w, h) = (w as u32, h as u32);
    let pixels = w as usize * h as usize;
    let mut rgba: Vec<u8> = Vec::with_capacity(pixels * 4);
    #[allow(clippy::uninit_vec)] // the grab writes every byte
    unsafe {
        rgba.set_len(pixels * 4)
    };
    let depth = grab_pixels_into(
        conn, root, x0 as i16, y0 as i16, w as u16, h as u16, &mut rgba, false,
    )?;
    if depth != 24 {
        return Err(format!(
            "unsupported root depth {}; only 24-bit TrueColor is implemented",
            depth
        ));
    }
    Ok(Frame {
        width: w,
        height: h,
        rgba,
    })
}

/// BGRX/BGR/raw-24 to opaque RGBA or BGRA, banded across threads at 4K
/// sizes. `src` may be a socket reply or a mapped SHM segment; either
/// way the conversion writes `out` exactly once. `bgra` selects the
/// output order: the overlay's GPU-bound frame wants BGRA, which for a
/// BGRX source is a plain alpha stamp with no R/B swap.
pub(super) fn convert_frame(
    src: &[u8],
    _pixels: usize,
    bpp: usize,
    out: &mut [u8],
    width: u32,
    height: u32,
    bgra: bool,
) -> Result<(), String> {
    match (bpp, bgra) {
        // BGRX little-endian -> BGRA: same byte order, stamp alpha.
        (4, true) => {
            crate::par::par_bands_mut(out, 4096, |out_chunk, start| {
                let in_chunk = &src[start..start + out_chunk.len()];
                for (o, i) in out_chunk.chunks_exact_mut(4).zip(in_chunk.chunks_exact(4)) {
                    let v = u32::from_le_bytes([i[0], i[1], i[2], i[3]]);
                    o.copy_from_slice(&(v | 0xFF00_0000).to_le_bytes());
                }
            });
        }
        // BGRX little-endian -> RGBA: swap R and B, stamp alpha.
        (4, false) => {
            // Banded across threads: at 12M pixels a scalar per-byte
            // loop is a visible slice of the latency.
            crate::par::par_bands_mut(out, 4096, |out_chunk, start| {
                let in_chunk = &src[start..start + out_chunk.len()];
                for (o, i) in out_chunk.chunks_exact_mut(4).zip(in_chunk.chunks_exact(4)) {
                    let v = u32::from_le_bytes([i[0], i[1], i[2], i[3]]);
                    let rgb = (v & 0xFF00_FF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF);
                    o.copy_from_slice(&(rgb | 0xFF00_0000).to_le_bytes());
                }
            });
        }
        // BGR triplets -> BGRA: bytes already land B,G,R; stamp alpha.
        (3, true) => {
            crate::par::par_bands_mut(out, 4096, |out_chunk, start| {
                let in_start = start / 4 * 3;
                let in_chunk = &src[in_start..in_start + out_chunk.len() / 4 * 3];
                for (o, px) in out_chunk.chunks_exact_mut(4).zip(in_chunk.chunks_exact(3)) {
                    o.copy_from_slice(&[px[0], px[1], px[2], 255]);
                }
            });
        }
        // BGR triplets -> RGBA: swap R and B, stamp alpha.
        (3, false) => {
            // Same banding as 4bpp: a 24bpp root at 12M pixels is the
            // same per-byte loop cost.
            crate::par::par_bands_mut(out, 4096, |out_chunk, start| {
                let in_start = start / 4 * 3;
                let in_chunk = &src[in_start..in_start + out_chunk.len() / 4 * 3];
                for (o, px) in out_chunk.chunks_exact_mut(4).zip(in_chunk.chunks_exact(3)) {
                    o.copy_from_slice(&[px[2], px[1], px[0], 255]);
                }
            });
        }
        (other, _) => {
            return Err(format!(
                "unsupported bytes-per-pixel {other} for {}x{} grab",
                width, height
            ));
        }
    }
    Ok(())
}

/// The pixel transfer, converted into `out`. A 4K-and-change root is
/// ~200MB: over the X socket that is seconds, over MIT-SHM it is a
/// page-faulted read. The SHM path converts straight out of the mapped
/// segment: no intermediate copy of the frame ever exists. Falls back
/// to plain GetImage when the server lacks SHM or the segment cannot
/// be set up. Returns the image depth. `x`/`y` offset the grab inside
/// `root`: a window capture reads only its rect, not the whole screen.
/// `bgra` selects the output channel order (BGRA for the GPU-bound
/// overlay frame, RGBA for PNG-bound captures).
fn grab_pixels_into<C>(
    conn: &C,
    root: x11rb::protocol::xproto::Window,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    out: &mut [u8],
    bgra: bool,
) -> Result<u8, String>
where
    C: Connection + x11rb::protocol::xproto::ConnectionExt,
{
    if let Some(depth) = try_shm_grab_into(conn, root, x, y, width, height, out, bgra) {
        return Ok(depth);
    }
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, root, x, y, width, height, !0u32)
        .map_err(|e| format!("X11 get_image: {e}"))?
        .reply()
        .map_err(|e| format!("X11 get_image reply: {e}"))?;
    if reply.depth == 24 {
        let pixels = width as usize * height as usize;
        let bpp = reply.data.len() / pixels.max(1);
        convert_frame(
            &reply.data,
            pixels,
            bpp,
            out,
            u32::from(width),
            u32::from(height),
            bgra,
        )?;
    }
    Ok(reply.depth)
}

#[cfg(test)]
mod tests {
    use super::convert_frame;

    /// Pin the channel-order math for every (bpp, bgra) combination.
    /// A swapped R/B is invisible to a dimension check, so assert the
    /// exact output bytes for a known input pixel.
    #[test]
    fn convert_frame_channel_orders() {
        // 4bpp BGRX source: bytes B=0x11, G=0x22, R=0x33, X=0x00.
        let src4 = [0x11, 0x22, 0x33, 0x00];
        let mut out = [0u8; 4];
        // RGBA out: R,G,B,A.
        convert_frame(&src4, 1, 4, &mut out, 1, 1, false).unwrap();
        assert_eq!(out, [0x33, 0x22, 0x11, 0xFF]);
        // BGRA out: B,G,R,A (alpha stamped, no swap).
        convert_frame(&src4, 1, 4, &mut out, 1, 1, true).unwrap();
        assert_eq!(out, [0x11, 0x22, 0x33, 0xFF]);

        // 3bpp BGR source: bytes B=0x11, G=0x22, R=0x33.
        let src3 = [0x11, 0x22, 0x33];
        let mut out = [0u8; 4];
        convert_frame(&src3, 1, 3, &mut out, 1, 1, false).unwrap();
        assert_eq!(out, [0x33, 0x22, 0x11, 0xFF]);
        convert_frame(&src3, 1, 3, &mut out, 1, 1, true).unwrap();
        assert_eq!(out, [0x11, 0x22, 0x33, 0xFF]);
    }
}
