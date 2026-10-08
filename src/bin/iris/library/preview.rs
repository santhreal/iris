//! The Space preview: one capture at full resolution over the grid,
//! decoded off the UI thread, with the card's thumbnail standing in
//! until the decode lands.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::*;

use super::Library;

pub(super) struct Preview {
    /// The listing index shown.
    pub index: usize,
    pub path: PathBuf,
    /// The decoded capture, once the background decode lands.
    pub image: Option<Arc<RenderImage>>,
    /// Why the capture could not be read, when it could not.
    pub error: Option<String>,
}

impl Library {
    /// Show entry `index` in the preview, replacing the one shown.
    pub(super) fn open_preview(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.listing.entries().get(index) else {
            return;
        };
        let path = entry.path.clone();
        self.release_preview(cx);
        self.preview = Some(Preview {
            index,
            path: path.clone(),
            image: None,
            error: None,
        });
        let task = cx.background_executor().spawn({
            let path = path.clone();
            async move {
                ::image::open(&path)
                    .map(|img| crate::widgets::render_image_from_rgba_owned(img.into_rgba8()))
                    .map_err(|e| format!("Could not read {}: {e}", path.display()))
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match this.preview.as_mut() {
                    Some(p) if p.path == path && p.image.is_none() => match result {
                        Ok(img) => p.image = Some(img),
                        Err(e) => p.error = Some(e),
                    },
                    // The preview moved on or closed while this decoded.
                    _ => {
                        if let Ok(img) = result {
                            crate::widgets::release_render(&img, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn close_preview(&mut self, cx: &mut Context<Self>) {
        self.release_preview(cx);
        cx.notify();
    }

    fn release_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(img) = self.preview.take().and_then(|p| p.image) {
            crate::widgets::release_render(&img, cx);
        }
    }
}

/// Space kept around the preview image inside the window's content.
const MARGIN: f32 = 32.0;
/// The caption line under the preview image.
const FOOTER_H: f32 = 40.0;

/// The logical size a `w` x `h` pixel capture shows at in a `avail_w` x
/// `avail_h` area on a `scale` display: its own size, scaled down to fit
/// and never up, so a small capture is not shown blurred.
pub(super) fn fit(w: u32, h: u32, scale: f32, avail_w: f32, avail_h: f32) -> (f32, f32) {
    let (nw, nh) = (w.max(1) as f32 / scale, h.max(1) as f32 / scale);
    // Constant bounds 0 <= 1: clamp cannot panic here.
    let s = (avail_w / nw).min(avail_h / nh).clamp(0.0, 1.0);
    ((nw * s).floor(), (nh * s).floor())
}

impl Library {
    /// The preview over the grid: the capture fitted to the content area,
    /// its file name and size under it. A click anywhere closes it.
    pub(super) fn preview_view(
        &self,
        preview: &Preview,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entry = self.listing.entries().get(preview.index);
        let caption = self.listing.captions().get(preview.index);
        let win = window.bounds().size;
        let avail_w = f32::from(win.width) - 2.0 * MARGIN;
        let avail_h = f32::from(win.height) - crate::theme::TOOLBAR_H - 2.0 * MARGIN - FOOTER_H;
        // The thumbnail stands in until the full decode lands.
        let image = preview
            .image
            .clone()
            .or_else(|| self.thumb_cache.get(&preview.path).cloned());
        let body = match (&preview.error, entry, image) {
            (Some(e), ..) => div()
                .text_size(px(crate::theme::TEXT_BODY))
                .text_color(crate::theme::FG_DIM)
                .child(e.clone())
                .into_any_element(),
            (None, Some(entry), Some(image)) => {
                let (w, h) = fit(
                    entry.width,
                    entry.height,
                    window.scale_factor(),
                    avail_w,
                    avail_h,
                );
                div()
                    .relative()
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(8.))
                    .overflow_hidden()
                    .shadow(crate::theme::shadow_float())
                    .child(img(ImageSource::Render(image)).size_full())
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(crate::theme::HAIRLINE),
                    )
                    .into_any_element()
            }
            _ => div().into_any_element(),
        };
        div()
            .id("preview")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .top(px(crate::theme::TOOLBAR_H))
            .bg(crate::theme::alpha(crate::theme::BG, 0.96))
            .occlude()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p(px(MARGIN))
            .on_click(cx.listener(|this, _, _, cx| this.close_preview(cx)))
            .child(body)
            .children(caption.map(|c| {
                div()
                    .h(px(FOOTER_H))
                    .flex()
                    .items_end()
                    .gap(px(8.))
                    .whitespace_nowrap()
                    .text_size(px(crate::theme::TEXT_BODY))
                    .font_features(crate::theme::tabular())
                    .child(div().text_color(crate::theme::FG).child(c.name.clone()))
                    .child(div().text_color(crate::theme::FG_DIM).child(c.dims.clone()))
            }))
    }
}
