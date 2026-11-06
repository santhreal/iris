//! The Space preview: one capture at full resolution over the grid,
//! decoded off the UI thread, with the card's thumbnail standing in
//! until the decode lands. It fades in as it opens and out as it
//! closes; a step to another capture swaps it in place.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui::*;

use super::Library;
use crate::motion;

pub(super) struct Preview {
    /// The listing index shown.
    pub index: usize,
    pub path: PathBuf,
    /// The decoded capture, once the background decode lands.
    pub image: Option<Arc<RenderImage>>,
    /// Why the capture could not be read, when it could not.
    pub error: Option<String>,
    /// When the preview opened: its fade-in runs from here, and a step
    /// to another capture keeps it.
    pub shown: Instant,
}

/// A closed preview while it fades out over the grid. It takes no
/// input: the grid under it has the keyboard and the pointer again.
pub(super) struct Leaving {
    pub preview: Preview,
    pub since: Instant,
}

/// How long the preview fades in, and out.
const FADE: std::time::Duration = motion::FADE;
/// The capture's scale where its fade-in starts: it grows to its size
/// as it appears.
const OPEN_SCALE: f32 = 0.96;
/// The capture's scale where its fade-out ends.
const CLOSE_SCALE: f32 = 0.98;

/// The preview's opacity and the capture's scale at `t` (0..=1) of its
/// fade: in while it opens, out while it is `leaving`. The capture is
/// at its own size, exactly, while the preview is fully shown; under
/// reduced motion only the opacity changes.
pub(super) fn pose(t: f32, leaving: bool, reduced: bool) -> (f32, f32) {
    let (opacity, from) = if leaving {
        (1.0 - motion::ease_out(t), CLOSE_SCALE)
    } else {
        (motion::ease_out_cubic(t), OPEN_SCALE)
    };
    let scale = if reduced {
        1.0
    } else {
        1.0 - (1.0 - from) * (1.0 - opacity)
    };
    (opacity, scale)
}

impl Library {
    /// Show entry `index` in the preview, replacing the one shown.
    pub(super) fn open_preview(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.listing.entries().get(index) else {
            return;
        };
        let path = entry.path.clone();
        let shown = self.preview.as_ref().map_or_else(Instant::now, |p| p.shown);
        self.release_preview(cx);
        self.release_leaving(cx);
        self.preview = Some(Preview {
            index,
            path: path.clone(),
            image: None,
            error: None,
            shown,
        });
        let task = cx.background_executor().spawn({
            let path = path.clone();
            async move {
                ::image::open(&path)
                    .map(|img| crate::widgets::render_image_from_rgba_owned(img.into_rgba8()))
                    .map_err(|e| iris_lib::reason::not_an_image(&path, &e))
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

    /// Close the preview: it fades out over the grid, which takes input
    /// at once.
    pub(super) fn close_preview(&mut self, cx: &mut Context<Self>) {
        self.release_leaving(cx);
        self.preview_out = self.preview.take().map(|preview| Leaving {
            preview,
            since: Instant::now(),
        });
        cx.notify();
    }

    /// Close the preview with no fade, as the library page leaves view.
    pub(super) fn drop_preview(&mut self, cx: &mut Context<Self>) {
        self.release_preview(cx);
        self.release_leaving(cx);
        cx.notify();
    }

    /// Whether the listing still shows `preview`'s capture at its index.
    pub(super) fn lists(&self, preview: &Preview) -> bool {
        self.listing.entries().get(preview.index).map(|e| &e.path) == Some(&preview.path)
    }

    /// The preview layers over the library page at `now`: a closed one
    /// fading out, then the open one fading in. Drops a fade-out that
    /// has ended or whose capture left its place in the listing, and
    /// asks for the next frame while either fades.
    pub(super) fn preview_layers(
        &mut self,
        page: Div,
        now: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let over = motion::tempo(FADE).as_secs_f32();
        let t = |from: Instant| (now.saturating_duration_since(from).as_secs_f32() / over).min(1.0);
        if self
            .preview_out
            .as_ref()
            .is_some_and(|l| t(l.since) >= 1.0 || !self.lists(&l.preview))
        {
            self.release_leaving(cx);
        }
        let reduced = cx.reduce_motion();
        let mut page = page;
        let mut fading = false;
        if let Some(leaving) = &self.preview_out {
            fading = true;
            let pose = pose(t(leaving.since), true, reduced);
            page = page.child(self.preview_view(&leaving.preview, pose, false, window, cx));
        }
        if let Some(preview) = &self.preview {
            let shown = t(preview.shown);
            fading |= shown < 1.0;
            let pose = pose(shown, false, reduced);
            page = page.child(self.preview_view(preview, pose, true, window, cx));
        }
        if fading {
            window.request_animation_frame();
        }
        page
    }

    fn release_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(img) = self.preview.take().and_then(|p| p.image) {
            crate::widgets::release_render(&img, cx);
        }
    }

    fn release_leaving(&mut self, cx: &mut Context<Self>) {
        if let Some(img) = self.preview_out.take().and_then(|l| l.preview.image) {
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
    /// The preview over the library page: the capture fitted to the
    /// grid's area, its file name and size under it, at `opacity` and
    /// with the capture at `scale` of its size (`pose`). A `live` one
    /// takes the pointer, and a click anywhere closes it; a fading-out
    /// one passes it to the grid.
    pub(super) fn preview_view(
        &self,
        preview: &Preview,
        (opacity, scale): (f32, f32),
        live: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entry = self.listing.entries().get(preview.index);
        let caption = self.listing.captions().get(preview.index);
        let win = window.client_area().size;
        let avail_w = f32::from(win.width) - crate::theme::SIDEBAR_W - 2.0 * MARGIN;
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
                // The capture scales inside a box of its full size: the
                // caption under it holds still.
                div()
                    .w(px(w))
                    .h(px(h))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .relative()
                            .w(px(w * scale))
                            .h(px(h * scale))
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
                            ),
                    )
                    .into_any_element()
            }
            _ => div().into_any_element(),
        };
        let layer = div()
            .id(if live { "preview" } else { "preview-leaving" })
            .absolute()
            .inset_0()
            .opacity(opacity)
            .bg(crate::theme::alpha(crate::theme::BG, 0.96));
        let layer = if live {
            layer
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| this.close_preview(cx)))
        } else {
            layer
        };
        layer
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p(px(MARGIN))
            .child(body)
            .children(caption.map(|c| {
                div()
                    .h(px(FOOTER_H))
                    .flex()
                    .items_end()
                    .gap(px(8.))
                    .whitespace_nowrap()
                    .text_size(px(crate::theme::TEXT_BODY))
                    // The name reads as a word, in proportional figures;
                    // the dimensions are a number, as the card's caption.
                    .child(div().text_color(crate::theme::FG).child(c.name.clone()))
                    .child(
                        div()
                            .text_color(crate::theme::FG_DIM)
                            .font_features(crate::theme::tabular())
                            .child(c.dims.clone()),
                    )
            }))
    }
}
