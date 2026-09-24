//! The toast's thumbnail: the capture box-filtered to the card's size
//! in device pixels, so the GPU maps its texels 1:1 onto the card.

use std::{path::Path, sync::Arc};

use gpui::{Context, RenderImage};

use super::{card_size, ToastStage};

/// A toast thumbnail sized for one display scale.
pub(crate) struct Thumb {
    /// The pixels the card paints.
    pub(crate) render: Arc<RenderImage>,
    /// The same pixels as RGBA: the drag icon's source.
    pub(crate) rgba: Arc<Vec<u8>>,
    /// Pixel size of `render` and `rgba`.
    pub(crate) px: (u32, u32),
    /// The card's size in logical pixels.
    pub(crate) dims: (f32, f32),
    /// Device pixels per logical pixel that `px` was sized for.
    pub(crate) scale: f32,
}

/// Scale the capture at `path` to the card's size in device pixels at
/// `scale` with a box filter. A capture smaller than that box keeps its
/// own pixels: there are no sharper ones.
pub(crate) fn prepare_thumb(path: &Path, scale: f32) -> Result<Thumb, String> {
    // A fresh capture's pixels are in finalize's stash. peek (not take)
    // leaves the slot for the editor's annotate path, and the borrow
    // feeds the scaler directly: no PNG decode and no 33MB clone.
    let stashed = crate::pipeline::peek_decoded(path);
    let owned;
    let img: &image::RgbaImage = if let Some(arc) = &stashed {
        arc
    } else {
        owned = image::open(path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?
            .to_rgba8();
        &owned
    };
    let (iw, ih) = img.dimensions();
    let dims = card_size(iw as f32, ih as f32);
    // The card's own box in device pixels: painting it resamples
    // nothing. Sized off the rounded card, not the capture's aspect, so
    // the texels land on whole device pixels.
    let (pw, ph) = (
        (dims.0 * scale).round().max(1.0) as u32,
        (dims.1 * scale).round().max(1.0) as u32,
    );
    let rgba = if pw <= iw && ph <= ih && (pw, ph) != (iw, ih) {
        // The banded replica of image's box-average thumbnail: a 4K
        // capture is a 33MB scan, split across cores.
        iris_lib::thumb::thumbnail_rgba(img, pw, ph)
    } else {
        img.clone()
    };
    let px = rgba.dimensions();
    // RenderImage straight from the scaled pixels: no PNG re-encode, and
    // the atlas tile stays freeable on dismiss. The copy is inherent:
    // the RenderImage wants BGRA while `rgba` stays RGBA for the drag
    // icon.
    let render = crate::widgets::render_image_from_rgba(px.0, px.1, rgba.as_raw());
    Ok(Thumb {
        render,
        rgba: Arc::new(rgba.into_raw()),
        px,
        dims,
        scale,
    })
}

impl ToastStage {
    /// Size the thumbnail for `scale` off the UI thread and swap it in.
    /// The toast opens with pixels sized for the display it is expected
    /// on; a window that renders at another scale gets them re-sized
    /// once. A failure keeps the current pixels, which still show the
    /// capture.
    pub(super) fn rescale_thumb(&mut self, scale: f32, cx: &mut Context<Self>) {
        self.rescaling = true;
        let path = self.path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { prepare_thumb(&path, scale) });
        cx.spawn(async move |this, cx| {
            let thumb = task.await;
            let _ = this.update(cx, |stage, cx| {
                stage.rescaling = false;
                match thumb {
                    // A leaving card keeps painting the pixels it has;
                    // its close releases them.
                    Ok(_) if stage.closing_at.is_some() || stage.pending_morph.is_some() => {}
                    Ok(thumb) => {
                        crate::widgets::release_render(&stage.thumb.render, cx);
                        stage.thumb = thumb;
                        cx.notify();
                    }
                    Err(e) => {
                        iris_lib::ilog!("toast: rescale: {e}");
                        // Stop the next render from retrying every frame.
                        stage.thumb.scale = scale;
                    }
                }
            });
        })
        .detach();
    }
}
