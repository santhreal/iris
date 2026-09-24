//! Pin to screen: a Snipaste-style always-on-top reference image.
//!
//! A borderless window holding the capture at native scale, pinned
//! above every other window via _NET_WM_STATE_ABOVE. Drag anywhere to
//! move it; double-click or Escape closes. The image is the frozen
//! capture's own pixels, not a live feed.

use std::sync::Arc;

use gpui::*;

use crate::theme;

/// A pinned reference window: the image.
pub struct PinStage {
    img: Arc<RenderImage>,
}

/// Open a pinned window showing `path`'s image at native scale. The
/// PNG read + decode run on the background executor: a 4K decode on
/// the UI thread stalls every other surface for ~100ms. A pin that
/// cannot open reports why in a notice.
pub fn open(cx: &mut App, path: &std::path::Path) {
    let path = path.to_path_buf();
    cx.spawn(async move |cx| {
        let decoded = cx
            .background_executor()
            .spawn(async move {
                // A just-captured image is already decoded in the
                // pipeline stash: swizzle it to BGRA (~10ms) instead of
                // re-decoding the PNG (~100ms). peek leaves the slot for
                // the editor's annotate path.
                if let Some(img) = crate::pipeline::peek_decoded(&path) {
                    return Ok(crate::widgets::render_image_from_rgba(
                        img.width(),
                        img.height(),
                        img.as_raw(),
                    ));
                }
                let bytes =
                    std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
                crate::widgets::render_image_from_png(&bytes)
                    .ok_or_else(|| format!("decode {}: not a supported image", path.display()))
            })
            .await;
        let _ = cx.update(|cx| {
            if let Err(e) = decoded.and_then(|img| open_with_image(cx, img)) {
                iris_lib::ilog!("pin: {e}");
                crate::notice::failed(cx, "Pin failed", &e);
            }
        });
    })
    .detach();
}

fn open_with_image(cx: &mut App, img: Arc<RenderImage>) -> Result<(), String> {
    // One image pixel per device pixel: a pinned capture shows at the
    // size it had on screen.
    let root = crate::sys::window::root_scale(cx);
    let (w, h) = {
        let s = img.size(0);
        (s.width.0 as f32 / root, s.height.0 as f32 / root)
    };
    // Cap the pinned window at a working size; the image scales down
    let (max_w, max_h) = (720.0f32, 540.0f32);
    let scale = (max_w / w).min(max_h / h).min(1.0);
    let (vw, vh) = (w * scale, h * scale);

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(80.), px(80.)),
                size: size(px(vw), px(vh)),
            })),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: None,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some("dev.iris.pin".to_string()),
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
        },
        move |window, cx| {
            crate::sys::window::keep_above(window);
            cx.new(|_| PinStage { img })
        },
    )
    .map_err(|e| format!("open pin window: {e}"))?;
    Ok(())
}

impl Render for PinStage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // No shadow: the window is the image's own size, so a shadow
        // would show only in the transparent rounded corners, as dark
        // wedges that square them off.
        div()
            .size_full()
            .font_family(theme::FONT)
            .rounded(px(8.))
            .overflow_hidden()
            .bg(theme::BG_ELEV)
            .cursor_pointer()
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, window, _| {
                if ev.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .child(crate::widgets::move_handle(crate::widgets::Double::Close))
            .child(
                img(ImageSource::Render(self.img.clone()))
                    .size_full()
                    .object_fit(ObjectFit::Contain),
            )
    }
}
