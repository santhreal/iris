use gpui::App;

/// GPUI paints an `Arc<Image>` by decoding it into a `RenderImage` and
/// caching both halves forever: `fetch_asset` never evicts the app-wide
/// decode cache, and the sprite-atlas tile (keyed by the RenderImage's
/// unique id) is only removed by an explicit `drop_image`, which the
/// `ImageSource::Image` path never calls. Feeding unique images (every
/// capture's frame, thumb and loupe) therefore leaks GPU memory
/// without bound. The escape hatch is `ImageSource::Render`: the app
/// constructs the `RenderImage` itself, so it can free the atlas tile
/// with `cx.drop_image` when the surface closes. The pixel copy is also
/// cheaper than the old path (BMP encode + decode round trip).
///
/// `rgba` is a tightly packed RGBA8 buffer; GPUI's atlas wants BGRA.
/// RGBA->BGRA in place, one u32 per pixel: the rotate form vectorizes;
/// a byte-wise swap does not.
#[inline]
pub(crate) fn swizzle_rgba_bgra(chunk: &mut [u8]) {
    for px in chunk.chunks_exact_mut(4) {
        let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
        let bgr = (v & 0xFF00_FF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF);
        px.copy_from_slice(&bgr.to_le_bytes());
    }
}

pub fn render_image_from_rgba(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> std::sync::Arc<gpui::RenderImage> {
    // Uninit capacity, not a zeroed vec: the fused copy+swizzle writes
    // every byte, and a multi-MB memset before a multi-MB fill is a
    // wasted pass. On failure the buffer drops without being read.
    let mut data: Vec<u8> = Vec::with_capacity(rgba.len());
    #[allow(clippy::uninit_vec)]
    unsafe {
        data.set_len(rgba.len())
    };
    // Fused copy+swizzle, banded across threads once the buffer is
    // large enough to pay for the spawn (the 152px loupe, rebuilt
    // every mousemove, stays inline).
    let row = width as usize * 4;
    iris_lib::par::par_bands_mut(&mut data, row, |dst, start| {
        dst.copy_from_slice(&rgba[start..start + dst.len()]);
        swizzle_rgba_bgra(dst);
    });
    let buf = ::image::RgbaImage::from_raw(width, height, data).expect("rgba buffer size");
    std::sync::Arc::new(gpui::RenderImage::new([::image::Frame::new(buf)]))
}

/// Wrap an already-BGRA buffer into a `RenderImage` with no swizzle:
/// the X11 capture path can produce BGRA directly, so the frame's
/// only CPU copy moves straight into the GPU-bound image.
pub fn render_image_from_bgra_owned(
    width: u32,
    height: u32,
    bgra: Vec<u8>,
) -> std::sync::Arc<gpui::RenderImage> {
    let buf = ::image::RgbaImage::from_raw(width, height, bgra).expect("bgra buffer size");
    std::sync::Arc::new(gpui::RenderImage::new([::image::Frame::new(buf)]))
}

/// Same construction from encoded PNG bytes (thumbs, editor base).
pub fn render_image_from_png(bytes: &[u8]) -> Option<std::sync::Arc<gpui::RenderImage>> {
    let mut data = ::image::load_from_memory_with_format(bytes, ::image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    // Same banding as the rgba path: a 4K editor base is a 33MB
    // swizzle, too big for one thread.
    let row = data.width() as usize * 4;
    iris_lib::par::par_bands_mut(data.as_mut(), row, |band, _| {
        swizzle_rgba_bgra(band);
    });
    Some(std::sync::Arc::new(gpui::RenderImage::new([
        ::image::Frame::new(data),
    ])))
}

/// Free a `RenderImage`'s sprite-atlas tile across every window.
/// Release between frames (event handlers) or via `cx.defer` from
/// render; dropping mid-paint blanks the frame.
pub fn release_render(image: &std::sync::Arc<gpui::RenderImage>, cx: &mut App) {
    cx.drop_image(image.clone(), None);
}
