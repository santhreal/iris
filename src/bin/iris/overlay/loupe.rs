use std::sync::Arc;

use gpui::*;

pub(super) const LOUPE_SRC: u32 = 19; // source pixels across the loupe
pub(super) const LOUPE_ZOOM: u32 = 8;
pub(super) const LOUPE_PX: u32 = LOUPE_SRC * LOUPE_ZOOM;

/// BMP for the monitor slices, written straight from the frame: a
/// 32bpp BI_RGB header plus bottom-up BGRA rows. One pass over the
/// cursor, plus the crosshair-free info line (coords + hex).
pub(super) fn loupe_image(
    img: &Arc<RenderImage>,
    width: u32,
    height: u32,
    fx: i64,
    fy: i64,
    scratch: &mut Vec<u8>,
) -> (Arc<RenderImage>, SharedString) {
    let bgra = img.as_bytes(0).unwrap_or(&[]);
    // Uninit, not zeroed: the row fill writes every byte, so a 92KB
    // memset before the fill is a wasted pass per mousemove. The
    // reserve is load-bearing: mem::take below hands the buffer to
    // the RenderImage, so the next rebuild starts from capacity 0
    // and set_len without it writes into a dangling allocation.
    scratch.clear();
    scratch.reserve((LOUPE_PX * LOUPE_PX * 4) as usize);
    #[allow(clippy::uninit_vec)] // the row fill writes every byte
    unsafe {
        scratch.set_len((LOUPE_PX * LOUPE_PX * 4) as usize)
    };
    // The scratch holds BGRA, the RenderImage's own order: the frame
    // bytes copy straight in with no swizzle pass either way.
    let out = scratch.as_mut_slice();
    let half = LOUPE_SRC as i64 / 2;
    let mut center = [0u8, 0u8, 0u8];
    let row_px = LOUPE_PX as usize;
    for sy in 0..LOUPE_SRC as i64 {
        // Build one zoomed row (each source pixel becomes LOUPE_ZOOM
        // horizontal copies), then memcpy it down LOUPE_ZOOM rows:
        // 19 row builds + 152 row copies instead of 23k pixel writes.
        let row_start = (sy as u32 * LOUPE_ZOOM) as usize * row_px * 4;
        let row = &mut out[row_start..row_start + row_px * 4];
        for sx in 0..LOUPE_SRC as i64 {
            let px_x = fx + sx - half;
            let px_y = fy + sy - half;
            let inside = px_x >= 0 && px_y >= 0 && px_x < width as i64 && px_y < height as i64;
            let src = if inside {
                let i = ((px_y as u32 * width + px_x as u32) * 4) as usize;
                [bgra[i], bgra[i + 1], bgra[i + 2], 255]
            } else {
                [0x14, 0x14, 0x16, 255]
            };
            if sx == half && sy == half {
                center = [src[2], src[1], src[0]];
            }
            let dx = (sx as u32 * LOUPE_ZOOM) as usize * 4;
            for bx in 0..LOUPE_ZOOM as usize {
                row[dx + bx * 4..dx + bx * 4 + 4].copy_from_slice(&src);
            }
        }
        for by in 1..LOUPE_ZOOM as usize {
            let dst = row_start + by * row_px * 4;
            let (head, tail) = out.split_at_mut(dst);
            tail[..row_px * 4].copy_from_slice(&head[row_start..row_start + row_px * 4]);
        }
    }
    // Crosshair on the center pixel.
    let mid = LOUPE_PX / 2;
    let z = LOUPE_ZOOM;
    for i in 0..z {
        for (x, y) in [
            (mid - z / 2 + i, mid - z / 2),
            (mid - z / 2 + i, mid + z / 2 - 1),
            (mid - z / 2, mid - z / 2 + i),
            (mid + z / 2 - 1, mid - z / 2 + i),
        ] {
            let d = ((y * LOUPE_PX + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&[255, 255, 255, 242]);
        }
    }
    // Straight into a RenderImage: this rebuilds on every drag
    // mousemove, so an encode/decode round trip is out of the question.
    // The scratch moves in wholesale; next rebuild allocates fresh.
    let buf = image::RgbaImage::from_raw(LOUPE_PX, LOUPE_PX, std::mem::take(scratch))
        .expect("loupe buffer size");
    let img = Arc::new(RenderImage::new([image::Frame::new(buf)]));
    let info = SharedString::from(format!(
        "{fx}, {fy}  #{:02x}{:02x}{:02x}",
        center[0], center[1], center[2]
    ));
    (img, info)
}

impl super::Overlay {
    /// Rebuild the loupe for frame pixel (fx, fy). The scratch buffer
    /// is reused across rebuilds so a drag does not alloc/free a 92KB
    /// image per mousemove.
    pub(super) fn update_loupe(&mut self, fx: i64, fy: i64, cx: &mut Context<Self>) {
        if self.loupe_at == Some((fx, fy)) {
            return;
        }
        self.loupe_at = Some((fx, fy));
        if let (Some(img), Some((w, h))) = (&self.frame_img, self.frame_size) {
            let img = img.clone();
            let (w, h) = (w, h);
            let built = loupe_image(&img, w, h, fx, fy, &mut self.loupe_scratch);
            // The previous loupe's atlas tile goes with it: a drag
            // mints one per mousemove.
            let old = self.loupe.replace(built);
            if let Some((img, _)) = old {
                crate::widgets::release_render(&img, cx);
            }
        }
    }
}
