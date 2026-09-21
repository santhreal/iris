//! Pin to screen: a Snipaste-style always-on-top reference image.
//!
//! A borderless window holding the capture at native scale, pinned
//! above every other window via _NET_WM_STATE_ABOVE. Drag anywhere to
//! move it; double-click or Escape closes. The image is the frozen
//! capture's own pixels, not a live feed.

use std::sync::Arc;

use gpui::*;

use crate::theme;

/// A pinned reference window: the image plus its drag state.
pub struct PinStage {
    img: Arc<RenderImage>,
    /// Window-space point where the current move drag grabbed the image.
    drag: Option<(f32, f32)>,
    class: SharedString,
}

/// Open a pinned window showing `path`'s image at native scale. The
/// PNG read + decode run on the background executor: a 4K decode on
/// the UI thread stalls every other surface for ~100ms.
pub fn open(cx: &mut App, path: &std::path::Path) -> Result<(), String> {
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
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("read {}: {e}", path.display()))?;
                crate::widgets::render_image_from_png(&bytes)
                    .ok_or_else(|| format!("decode {}: not a supported image", path.display()))
            })
            .await;
        let img = match decoded {
            Ok(img) => img,
            Err(e) => {
                iris_lib::ilog!("pin: {e}");
                return;
            }
        };
        let _ = cx.update(|cx| {
            if let Err(e) = open_with_image(cx, img) {
                iris_lib::ilog!("pin: {e}");
            }
        });
    })
    .detach();
    Ok(())
}

fn open_with_image(cx: &mut App, img: Arc<RenderImage>) -> Result<(), String> {
    let (w, h) = {
        let s = img.size(0);
        (s.width.0 as f32, s.height.0 as f32)
    };
    // Cap the pinned window at a working size; the image scales down
    let (max_w, max_h) = (720.0f32, 540.0f32);
    let scale = (max_w / w).min(max_h / h).min(1.0);
    let (vw, vh) = (w * scale, h * scale);

    let win_id = crate::sys::window::unique_id("dev.iris.pin");
    let class = SharedString::from(win_id.clone());
    let class2 = win_id.clone();
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
            app_id: Some(win_id),
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
        },
        move |_, cx| {
            cx.new(|_| PinStage {
                img,
                drag: None,
                class,
            })
        },
    )
    .map_err(|e| format!("open pin window: {e}"))?;
    crate::sys::window::always_on_top_after_map(class2);
    Ok(())
}

impl Render for PinStage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let class = self.class.clone();
        div()
            .size_full()
            .font_family(theme::FONT)
            .rounded(px(8.))
            .overflow_hidden()
            .bg(theme::BG_ELEV)
            .shadow(theme::shadow_float())
            .cursor_pointer()
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, window, _| {
                if ev.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    if ev.click_count == 2 {
                        window.remove_window();
                        return;
                    }
                    this.drag = Some((ev.position.x.into(), ev.position.y.into()));
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, window, _cx| {
                if this.drag.is_some() && ev.pressed_button == Some(MouseButton::Left) {
                    // Hand the move to the WM: the root pointer position
                    // is window origin + local position.
                    let origin = window.bounds().origin;
                    let (mx, my): (f32, f32) =
                        (ev.position.x.into(), ev.position.y.into());
                    crate::sys::window::begin_wm_move(
                        class.to_string(),
                        (f32::from(origin.x) + mx) as i32,
                        (f32::from(origin.y) + my) as i32,
                    );
                    this.drag = None;
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .child(
                img(ImageSource::Render(self.img.clone()))
                    .size_full()
                    .object_fit(ObjectFit::Contain),
            )
    }
}

