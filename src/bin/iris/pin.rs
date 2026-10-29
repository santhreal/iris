//! Pin to screen: a Snipaste-style always-on-top reference image.
//!
//! A borderless window holding the capture at native scale, pinned
//! above every other window via _NET_WM_STATE_ABOVE. It opens centered
//! on the screen rect its pixels came from, so a pin at native scale
//! covers them exactly. Drag anywhere to move it; double-click, Escape,
//! Ctrl+W, or the close button that shows in the top-left corner while
//! the pointer is over the window closes it. Ctrl+C copies the image.
//! The image is the frozen capture's own pixels, not a live feed.

use std::{path::PathBuf, sync::Arc};

use gpui::*;

use crate::theme;

/// A pinned reference window: the image and the capture it shows.
pub struct PinStage {
    img: Arc<RenderImage>,
    /// The capture file, which Ctrl+C copies.
    path: PathBuf,
    /// The window's focus: without it no key event reaches this window,
    /// so Escape would be dead code on a plain div.
    focus: FocusHandle,
    /// The result of the last copy, shown for `widgets::STATUS_FOR`.
    status: Option<SharedString>,
    /// Copies run so far: a copy clears the pill only when no later
    /// copy has run.
    copies: u64,
}

/// A rect in screen coordinates, logical pixels: x, y, width, height.
pub type Over = (f32, f32, f32, f32);

/// The origin of a pin with no rect to open over.
const UNPLACED: (f32, f32) = (80.0, 80.0);

/// How far a pin that would open on another pin's origin steps down
/// and right.
const CASCADE: f32 = 24.0;

/// Open a pinned window showing `path`'s image at native scale,
/// centered on `over`. The PNG read + decode run on the background
/// executor: a 4K decode on the UI thread stalls every other surface
/// for ~100ms. A pin that cannot open reports why in a notice.
pub fn open(cx: &mut App, path: &std::path::Path, over: Option<Over>) {
    let path = path.to_path_buf();
    cx.spawn(async move |cx| {
        let file = path.clone();
        let decoded = cx
            .background_executor()
            .spawn(async move {
                let path = file;
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
        cx.update(|cx| {
            if let Err(e) = decoded.and_then(|img| open_with_image(cx, img, path, over)) {
                iris_lib::ilog!("pin: {e}");
                crate::notice::failed(cx, "Pin failed", &e);
            }
        });
    })
    .detach();
}

/// The origin of a `size` pin: centered on `over` and on a whole pixel,
/// or at UNPLACED with no rect. A pin whose origin another pin's
/// origin in `taken` already has steps CASCADE down and right until no
/// other pin is there. A pin no larger than an on-screen `over` stays
/// inside it, so it stays on that screen.
pub(crate) fn place(size: (f32, f32), over: Option<Over>, taken: &[(f32, f32)]) -> (f32, f32) {
    let (mut x, mut y) = match over {
        Some((ox, oy, ow, oh)) => (
            (ox + (ow - size.0) / 2.0).round(),
            (oy + (oh - size.1) / 2.0).round(),
        ),
        None => UNPLACED,
    };
    // Each step leaves the one pin it met: `taken.len()` steps reach a
    // free origin.
    for _ in 0..taken.len() {
        let met = taken
            .iter()
            .any(|&(tx, ty)| (tx - x).abs() < 1.0 && (ty - y).abs() < 1.0);
        if !met {
            break;
        }
        x += CASCADE;
        y += CASCADE;
    }
    (x, y)
}

/// The origins of the open pins.
fn pin_origins(cx: &mut App) -> Vec<(f32, f32)> {
    cx.windows()
        .into_iter()
        .filter_map(|window| window.downcast::<PinStage>())
        .filter_map(|pin| {
            pin.update(cx, |_, window, _| {
                let origin = window.bounds().origin;
                (f32::from(origin.x), f32::from(origin.y))
            })
            .ok()
        })
        .collect()
}

fn open_with_image(
    cx: &mut App,
    img: Arc<RenderImage>,
    path: PathBuf,
    over: Option<Over>,
) -> Result<(), String> {
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
    let (x, y) = place((vw, vh), over, &pin_origins(cx));
    crate::widgets::open_window(
        cx,
        "Pin - iris",
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(x), px(y)),
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
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        },
        move |window, cx| {
            crate::sys::window::keep_above(window);
            cx.new(|cx| {
                let focus = cx.focus_handle();
                // The window takes OS focus at open; key events reach
                // the root only once its dispatch path is the focused
                // one, so the handle takes GPUI focus here and on a
                // click inside (track_focus).
                window.focus(&focus, cx);
                PinStage {
                    img,
                    path,
                    focus,
                    status: None,
                    copies: 0,
                }
            })
        },
    )
    .map_err(|e| format!("open pin window: {e}"))?;
    Ok(())
}

impl PinStage {
    /// Copy the capture's image to the clipboard off the UI thread, and
    /// state the result in the status pill.
    fn copy(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        self.copies += 1;
        let copy = self.copies;
        let task = cx
            .background_executor()
            .spawn(async move { crate::pipeline::copy_image_file(&path) });
        cx.spawn(async move |this, cx| {
            let text = SharedString::from(match task.await {
                Ok(()) => "Copied".to_owned(),
                Err(e) => e,
            });
            let _ = this.update(cx, |this, cx| {
                this.status = Some(text);
                cx.notify();
            });
            cx.background_executor()
                .timer(crate::widgets::STATUS_FOR)
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.copies == copy {
                    this.status = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for PinStage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // No shadow: the window is the image's own size, so a shadow
        // would show only in the transparent rounded corners, as dark
        // wedges that square them off.
        div()
            .group("pin")
            .size_full()
            .font_family(theme::FONT)
            .rounded(crate::widgets::window_corner(window, 8.))
            .overflow_hidden()
            .bg(theme::BG_ELEV)
            .cursor_pointer()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let ks = &ev.keystroke;
                let meta = ks.modifiers.control || ks.modifiers.platform;
                match ks.key.as_str() {
                    "escape" => window.remove_window(),
                    "w" if meta => window.remove_window(),
                    "c" if meta => this.copy(cx),
                    _ => {}
                }
            }))
            .child(crate::widgets::move_handle(crate::widgets::Double::Close))
            .child(
                img(ImageSource::Render(self.img.clone()))
                    .size_full()
                    .object_fit(ObjectFit::Contain),
            )
            // Over the move handle and occluding it, so a press on the
            // button closes instead of starting a window drag.
            .child(
                div()
                    .id("pin-close")
                    .absolute()
                    .top(px(8.))
                    .left(px(8.))
                    .size(px(22.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme::alpha(theme::BG_ELEV, 0.9))
                    .border_1()
                    .border_color(theme::HAIRLINE)
                    .shadow(theme::shadow_float())
                    .opacity(0.)
                    .group_hover("pin", |s| s.opacity(1.))
                    .hover(|s| s.bg(theme::BG_ELEV))
                    .occlude()
                    .cursor_pointer()
                    .tooltip(crate::widgets::tip("Close", Some("Esc".into())))
                    .on_click(|_, window, _| window.remove_window())
                    .child(crate::icons::icon(
                        crate::icons::Icon::Close,
                        theme::FG,
                        12.0,
                    )),
            )
            .children(
                self.status
                    .as_ref()
                    .map(|text| crate::widgets::status_pill(text, 10.0)),
            )
    }
}

#[cfg(test)]
mod tests;
