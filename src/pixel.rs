//! Channel-order conversions on packed 8-bit pixels. Every capture,
//! thumbnail, and GPU-bound image crosses between RGBA and BGRA here.
//!
//! Each pixel is a `[u8; 4]` chunk handled as one little-endian u32
//! with masks and shifts. LLVM vectorizes that form and leaves byte
//! indexing (`px[3] = 255`, `px.swap(0, 2)`) and `chunks_exact(4)`
//! slices scalar: on a 3840x2160 frame the u32 form measured 1.5x to
//! 3.6x the byte forms it replaced.

/// R and B swapped: RGBA <-> BGRA. G and A are unchanged.
#[inline]
pub fn swap_rb(px: [u8; 4]) -> [u8; 4] {
    let v = u32::from_le_bytes(px);
    ((v & 0xFF00_FF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF)).to_le_bytes()
}

/// R and B swapped, alpha set to 255: X11 BGRX and GDI `BI_RGB`
/// pixels leave the fourth byte undefined.
#[inline]
pub fn swap_rb_opaque(px: [u8; 4]) -> [u8; 4] {
    let v = u32::from_le_bytes(px);
    ((v & 0xFF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF) | 0xFF00_0000).to_le_bytes()
}

/// Channels unchanged, alpha set to 255.
#[inline]
pub fn opaque(px: [u8; 4]) -> [u8; 4] {
    (u32::from_le_bytes(px) | 0xFF00_0000).to_le_bytes()
}

/// `swap_rb` over every whole pixel of `buf`, in place.
pub fn swap_rb_in_place(buf: &mut [u8]) {
    for px in buf.as_chunks_mut::<4>().0 {
        *px = swap_rb(*px);
    }
}

/// `f` over `src` into `dst`, pixel for pixel, for the whole pixels
/// both hold. One read and one write per pixel: a copy followed by an
/// in-place conversion measured 1.8x slower.
pub fn map_into(src: &[u8], dst: &mut [u8], f: impl Fn([u8; 4]) -> [u8; 4]) {
    let dst = dst.as_chunks_mut::<4>().0.iter_mut();
    for (d, s) in dst.zip(src.as_chunks::<4>().0) {
        *d = f(*s);
    }
}

/// `f` over every whole pixel of `src`, into a new buffer: one pass,
/// with no zero fill of the destination.
pub fn map_to_vec(src: &[u8], f: impl Fn([u8; 4]) -> [u8; 4]) -> Vec<u8> {
    let pixels: Vec<[u8; 4]> = src.as_chunks::<4>().0.iter().map(|&px| f(px)).collect();
    pixels.into_flattened()
}

// WHY: the class closed here is "a conversion scrambles channels":
// every capture, thumbnail, and RenderImage passes through this module,
// so a wrong lane recolors or drops alpha across the whole app, and a
// slip in a buffer walk skips or smears pixels. Each pixel function is
// pinned byte for byte; each walk is checked against a per-pixel
// reference over a buffer long enough for a vectorized body and its
// scalar tail. swap_rb is an involution because the same function
// runs RGBA->BGRA on the way in and BGRA->RGBA on the way out. Not
// covered: the banding that calls the walks, which par.rs tests own.
#[cfg(test)]
mod tests {
    use super::*;

    type Convert = fn([u8; 4]) -> [u8; 4];

    const CONVERTERS: [(&str, Convert); 3] = [
        ("swap_rb", swap_rb),
        ("swap_rb_opaque", swap_rb_opaque),
        ("opaque", opaque),
    ];

    /// A buffer whose neighbouring bytes all differ, so any lane
    /// mix-up shows.
    fn frame(pixels: usize) -> Vec<u8> {
        (0..pixels * 4).map(|i| (i * 37 + 11) as u8).collect()
    }

    #[test]
    fn pixel_functions_move_exact_bytes() {
        let px = [0x11, 0x22, 0x33, 0x44];
        assert_eq!(swap_rb(px), [0x33, 0x22, 0x11, 0x44]);
        assert_eq!(swap_rb_opaque(px), [0x33, 0x22, 0x11, 0xFF]);
        assert_eq!(opaque(px), [0x11, 0x22, 0x33, 0xFF]);
        assert_eq!(swap_rb_opaque([1, 2, 3, 0]), [3, 2, 1, 0xFF]);
        assert_eq!(opaque([1, 2, 3, 0]), [1, 2, 3, 0xFF]);
    }

    #[test]
    fn swap_rb_is_an_involution() {
        let original = frame(16);
        let mut buf = original.clone();
        swap_rb_in_place(&mut buf);
        assert_ne!(buf, original);
        swap_rb_in_place(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn walks_convert_every_pixel() {
        let src = frame(67);
        let reference = |f: Convert| -> Vec<u8> {
            let mut out = Vec::with_capacity(src.len());
            for i in 0..src.len() / 4 {
                let p = 4 * i;
                out.extend_from_slice(&f([src[p], src[p + 1], src[p + 2], src[p + 3]]));
            }
            out
        };
        for (name, f) in CONVERTERS {
            let want = reference(f);
            assert_eq!(map_to_vec(&src, f), want, "{name} map_to_vec");
            let mut dst = vec![0u8; src.len()];
            map_into(&src, &mut dst, f);
            assert_eq!(dst, want, "{name} map_into");
        }
        let mut buf = src.clone();
        swap_rb_in_place(&mut buf);
        assert_eq!(buf, reference(swap_rb), "swap_rb_in_place");
    }

    /// A trailing partial pixel is not a pixel: the walks stop before
    /// it, leave its bytes alone, and do not panic.
    #[test]
    fn partial_trailing_pixel_is_left_alone() {
        let mut buf = frame(3);
        buf.truncate(11);
        let tail = buf[8..].to_vec();
        swap_rb_in_place(&mut buf);
        assert_eq!(buf[8..], tail[..]);
        assert_eq!(map_to_vec(&buf, opaque).len(), 8);
        let mut dst = vec![7u8; 11];
        map_into(&buf, &mut dst, opaque);
        assert_eq!(dst[8..], [7, 7, 7]);
    }
}
