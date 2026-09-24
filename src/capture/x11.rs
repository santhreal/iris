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
        let full = GrabRect {
            x: 0,
            y: 0,
            width: geom.width,
            height: geom.height,
        };
        let depth = grab_pixels_into(&conn, root, full, &mut rgba, false)?;

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
    let full = GrabRect {
        x: 0,
        y: 0,
        width: geom.width,
        height: geom.height,
    };
    let depth = grab_pixels_into(&conn, root, full, &mut bgra, true)?;
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
    let rect = GrabRect {
        x: x0 as i16,
        y: y0 as i16,
        width: w as u16,
        height: h as u16,
    };
    let depth = grab_pixels_into(conn, root, rect, &mut rgba, false)?;
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
                crate::pixel::map_into(in_chunk, out_chunk, crate::pixel::opaque);
            });
        }
        // BGRX little-endian -> RGBA: swap R and B, stamp alpha.
        (4, false) => {
            // Banded across threads: at 12M pixels a scalar per-byte
            // loop is a visible slice of the latency.
            crate::par::par_bands_mut(out, 4096, |out_chunk, start| {
                let in_chunk = &src[start..start + out_chunk.len()];
                crate::pixel::map_into(in_chunk, out_chunk, crate::pixel::swap_rb_opaque);
            });
        }
        // BGR triplets -> BGRA: bytes already land B,G,R; stamp alpha.
        // Banded like 4bpp: a 24bpp root at 12M pixels is the same
        // per-byte loop cost.
        (3, true) => {
            crate::par::par_bands_mut(out, 4096, |out_chunk, start| {
                let in_start = start / 4 * 3;
                expand24::<0>(
                    &src[in_start..in_start + out_chunk.len() / 4 * 3],
                    out_chunk,
                );
            });
        }
        // BGR triplets -> RGBA: swap R and B, stamp alpha.
        (3, false) => {
            crate::par::par_bands_mut(out, 4096, |out_chunk, start| {
                let in_start = start / 4 * 3;
                expand24::<2>(
                    &src[in_start..in_start + out_chunk.len() / 4 * 3],
                    out_chunk,
                );
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

/// Packed 24-bit pixels to opaque 32-bit. `FIRST` is the source byte
/// written first: 0 keeps B,G,R (BGRA out), 2 swaps R and B (RGBA out).
/// Four pixels per step, 12 bytes in and 16 out: on a 3840x2160 frame
/// the four-pixel literal measured 1.2x (BGRA) and 1.6x (RGBA) the
/// throughput of a per-pixel loop, which LLVM leaves scalar.
fn expand24<const FIRST: usize>(src: &[u8], out: &mut [u8]) {
    let (a, c) = (FIRST, 2 - FIRST);
    let (blocks, tail) = out.as_chunks_mut::<16>();
    let (ins, in_tail) = src.as_chunks::<12>();
    for (o, i) in blocks.iter_mut().zip(ins) {
        #[rustfmt::skip]
        let block = [
            i[a], i[1], i[c], 255,
            i[3 + a], i[4], i[3 + c], 255,
            i[6 + a], i[7], i[6 + c], 255,
            i[9 + a], i[10], i[9 + c], 255,
        ];
        *o = block;
    }
    let tail = tail.as_chunks_mut::<4>().0.iter_mut();
    for (o, i) in tail.zip(in_tail.as_chunks::<3>().0) {
        *o = [i[a], i[1], i[c], 255];
    }
}

/// A grab rectangle inside the root window, in root coordinates.
#[derive(Clone, Copy)]
pub(super) struct GrabRect {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
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
    rect: GrabRect,
    out: &mut [u8],
    bgra: bool,
) -> Result<u8, String>
where
    C: Connection + x11rb::protocol::xproto::ConnectionExt,
{
    if let Some(depth) = try_shm_grab_into(conn, root, rect, out, bgra) {
        return Ok(depth);
    }
    let GrabRect {
        x,
        y,
        width,
        height,
    } = rect;
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

    /// Every pixel of a frame, not one: the 24bpp path converts four
    /// pixels per step plus a per-pixel tail, and bands split the frame
    /// on 1024-pixel edges. A tail or band-offset slip corrupts pixels
    /// the single-pixel test above never reaches. Counts straddle the
    /// four-pixel step and the band edge; the last one is past the 1 MiB
    /// size gate, so it bands across threads with a ragged final band.
    /// Not covered: the SHM and GetImage transports that feed `src`.
    #[test]
    fn convert_frame_matches_per_pixel_reference_across_bands() {
        for n in [1usize, 2, 3, 4, 5, 7, 1023, 1024, 1025, 4097, (1 << 18) + 3] {
            for bpp in [3usize, 4] {
                let src: Vec<u8> = (0..n * bpp).map(|i| (i * 131 % 251) as u8).collect();
                for bgra in [false, true] {
                    let mut out = vec![0u8; n * 4];
                    convert_frame(&src, n, bpp, &mut out, n as u32, 1, bgra).unwrap();
                    let pixels = out.as_chunks::<4>().0.iter().zip(src.chunks(bpp));
                    for (p, (o, s)) in pixels.enumerate() {
                        let want = if bgra {
                            [s[0], s[1], s[2], 255]
                        } else {
                            [s[2], s[1], s[0], 255]
                        };
                        assert_eq!(*o, want, "bpp {bpp} bgra {bgra} n {n} pixel {p}");
                    }
                }
            }
        }
    }
}
