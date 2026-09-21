//! Toast stage actions, thumbnail preparation, and UI action helpers.

use std::{
    path::Path,
    sync::{
        atomic::AtomicBool,
        Arc,
    },
    time::{Duration, Instant},
};


use super::*;
use crate::{motion, pipeline};

/// Decode and pre-scale the thumbnail to its exact display size with a
/// high-quality filter: no resampling happens per frame afterwards.
pub(crate) fn prepare_thumb(
    path: &Path,
) -> Result<(Arc<RenderImage>, Arc<Vec<u8>>, (f32, f32)), String> {
    // Reuse the decoded pixels finalize stashed: a fresh capture's toast
    // skips the ~100ms 4K PNG re-decode entirely. peek (not take) leaves
    // the slot for the editor's annotate path, which reads it later. The
    // borrow feeds thumbnail() directly, so no 33MB clone either.
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
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let scale = (MAX_W / iw).min(MAX_H / ih).min(1.0);
    let (w, h) = ((iw * scale).round().max(1.0), (ih * scale).round().max(1.0));
    // thumbnail() pre-shrinks with a cheap nearest pass before the
    // convolution resize: on a 4K capture a straight Lanczos3 resize
    // convolves 33MB to produce a ~216px card.
    let rgba = if scale < 1.0 {
        image::imageops::thumbnail(img, w as u32, h as u32)
    } else {
        img.clone()
    };
    // thumbnail() can land a pixel off the computed (w, h) on a
    // rounding boundary; report the real dims so the card layout
    // matches the pixels.
    let (w, h) = (rgba.width() as f32, rgba.height() as f32);
    // RenderImage directly from the resized pixels: no PNG
    // re-encode, and the atlas tile stays freeable on dismiss. The
    // copy is inherent: the RenderImage wants BGRA while thumb_rgba
    // keeps RGBA for the drag icon's own swizzle.
    let render = crate::widgets::render_image_from_rgba(rgba.width(), rgba.height(), rgba.as_raw());
    Ok((render, Arc::new(rgba.into_raw()), (w, h)))
}

impl ToastStage {
    /// Build the stage from an already-decoded thumbnail. The decode
    /// itself runs on the background executor in show_toast_kind.
    pub(super) fn from_parts(
        path: &Path,
        thumb: Arc<RenderImage>,
        thumb_rgba: Arc<Vec<u8>>,
        dims: (f32, f32),
    ) -> Self {
        Self {
            path: path.to_path_buf(),
            thumb,
            thumb_rgba,
            dims,
            card_screen: (0.0, 0.0, 0.0, 0.0),
            opened: None,
            hover_paused: false,
            pinned: false,
            dismiss_gen: 0,
            closing_at: None,
            exit_from: 0.0,
            drag_start: None,
            gesture: Gesture::Undecided,
            swipe: None,
            swipe_return: None,
            menu_at: None,
            menu_opened: None,
            swallow_click: false,
            pending_morph: None,
            morph_ready_at: None,
            closing_dur: EXIT,
            cfg: iris_lib::config::Config::load(),
        }
    }

    /// Start the auto-dismiss countdown. When it fires, hovering pushes
    /// the deadline out instead of dismissing under the pointer.
    pub(super) fn arm_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.pinned || self.closing_at.is_some() {
            return;
        }
        self.dismiss_gen += 1;
        let gen = self.dismiss_gen;
        let sit = Duration::from_millis(u64::from(self.cfg.toast_duration_ms));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(sit).await;
            this.update(cx, |stage, cx| {
                if stage.dismiss_gen != gen || stage.pinned {
                    return;
                }
                if stage.hover_paused || stage.menu_at.is_some() {
                    stage.arm_dismiss(cx);
                } else {
                    stage.begin_close(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        self.pinned = !self.pinned;
        if self.pinned {
            self.closing_at = None;
        } else {
            self.arm_dismiss(cx);
        }
        cx.notify();
    }

    pub(super) fn begin_close(&mut self, cx: &mut Context<Self>) {
        if self.closing_at.is_some() || self.pending_morph.is_some() {
            return;
        }
        self.closing_at = Some(Instant::now());
        cx.notify();
    }

    /// Replaced by a newer capture: fade out fast from wherever the
    /// card sits. The full fly-off would read as two competing exits.
    pub(super) fn fade_out_quick(&mut self, cx: &mut Context<Self>) {
        self.closing_dur = REPLACE_EXIT;
        self.begin_close(cx);
    }

    /// Open the markup editor over this toast and wait for its first
    /// frame before leaving: the editor paints the same image over the
    /// exact same screen pixels, so the morph has no blank blink.
    pub(super) fn hand_off_to_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ready = Arc::new(AtomicBool::new(false));
        let result =
            crate::editor::open(cx, &self.path, Some(self.card_screen), Some(ready.clone()));
        match result {
            Ok(()) => {
                self.pending_morph = Some((ready, Instant::now()));
                cx.notify();
            }
            Err(e) => {
                iris_lib::ilog!("annotate: {e}");
                crate::widgets::release_render(&self.thumb, cx);
                window.remove_window();
            }
        }
    }

    /// Resolve a released dismiss swipe: fast or far enough flies the
    /// card off from its carried offset; otherwise it springs back to
    /// the corner. No-op once the swipe state is consumed.
    pub(super) fn release_swipe(&mut self, cx: &mut Context<Self>) {
        let Some(mut s) = self.swipe.take() else {
            return;
        };
        // A finger that stopped moving before release is not a flick:
        // decay the smoothed velocity by the sample's age.
        if s.last_at.elapsed() > SWIPE_STALE {
            s.vel = 0.0;
        }
        if s.vel > SWIPE_VELOCITY || s.dx > SWIPE_TRAVEL {
            self.exit_from = s.dx;
            self.begin_close(cx);
        } else {
            self.swipe_return = Some((Instant::now(), s.dx));
        }
        cx.notify();
    }

    pub(super) fn copy_image(&mut self, cx: &mut Context<Self>) {
        // PNG decode of a large capture is too slow for the UI thread.
        let path = self.path.clone();
        cx.background_executor()
            .spawn(async move {
                if let Err(e) = crate::pipeline::copy_image_file(&path) {
                    iris_lib::ilog!("copy: {e}");
                }
            })
            .detach();
    }

    pub(super) fn copy_file(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = iris_lib::dragcopy::copy_file_path(&self.path) {
            iris_lib::ilog!("copy file: {e}");
        }
        cx.notify();
    }

    pub(super) fn open_folder(&mut self, cx: &mut Context<Self>) {
        reveal_in_folder(&self.path);
        cx.notify();
    }

    pub(super) fn delete_capture(&mut self, cx: &mut Context<Self>) {
        // File unlink + library.json rewrite off the UI thread: a slow
        // shots dir would freeze the toast mid-swipe.
        let path = self.path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { iris_lib::library::delete(&path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |stage, cx| match result {
                Ok(()) => stage.begin_close(cx),
                Err(e) => iris_lib::ilog!("delete: {e}"),
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn perform_click_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.cfg.toast_click_action {
            iris_lib::config::ToastClickAction::Markup => {
                self.hand_off_to_editor(window, cx);
            }
            iris_lib::config::ToastClickAction::Copy => {
                self.copy_image(cx);
            }
            iris_lib::config::ToastClickAction::OpenFolder => {
                self.open_folder(cx);
            }
            iris_lib::config::ToastClickAction::None => {}
        }
    }

    /// Right-click context menu, macOS style: Markup, Copy, Delete,
    /// Close. Fades in over 120ms rising 3px; shares the exit
    /// opacity so it never outlives the card it belongs to.
    pub(super) fn render_menu(
        &mut self,
        opacity: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let (menu_x, menu_y) = self.menu_at?;
        let opened = *self.menu_opened.get_or_insert_with(Instant::now);
        let mt = (opened.elapsed().as_secs_f32() / motion::tempo(MENU_FADE).as_secs_f32()).min(1.0);
        if mt < 1.0 {
            window.request_animation_frame();
        }
        let mut menu = crate::widgets::menu()
            .absolute()
            .left(px(menu_x))
            .top(px(menu_y + 3.0 * (1.0 - motion::ease_out_cubic(mt))))
            .opacity(mt * opacity);
        menu = menu.child(crate::widgets::menu_row("toast-markup", "Markup").on_click(
            cx.listener(|stage, _, window, cx| {
                cx.stop_propagation();
                stage.menu_at = None;
                stage.hand_off_to_editor(window, cx);
            }),
        ));
        menu = menu.child(
            crate::widgets::menu_row("toast-copy", "Copy").on_click(cx.listener(
                |stage, _, _, cx| {
                    cx.stop_propagation();
                    stage.menu_at = None;
                    stage.copy_image(cx);
                },
            )),
        );
        menu = menu.child(
            crate::widgets::menu_row("toast-ocr", "Copy text (OCR)").on_click(cx.listener(
                |stage, _, _window, cx| {
                    cx.stop_propagation();
                    stage.menu_at = None;
                    let path = stage.path.clone();
                    // tesseract is a subprocess; never the UI thread.
                    cx.background_executor()
                        .spawn(async move {
                            if let Err(e) = pipeline::copy_ocr_text(&path) {
                                iris_lib::ilog!("iris: ocr: {e}");
                            }
                        })
                        .detach();
                    stage.begin_close(cx);
                },
            )),
        );
        menu = menu.child(
            crate::widgets::menu_row("toast-pin", "Pin to screen").on_click(cx.listener(
                |stage, _, _window, cx| {
                    cx.stop_propagation();
                    stage.menu_at = None;
                    if let Err(e) = crate::pin::open(cx, &stage.path) {
                        iris_lib::ilog!("pin: {e}");
                    }
                    stage.begin_close(cx);
                },
            )),
        );
        menu = menu.child(crate::widgets::menu_row("toast-delete", "Delete").on_click(
            cx.listener(|stage, _, _, cx| {
                cx.stop_propagation();
                stage.menu_at = None;
                stage.delete_capture(cx);
            }),
        ));
        menu = menu.child(
            crate::widgets::menu_row("toast-close", "Close").on_click(cx.listener(
                |stage, _, _, cx| {
                    cx.stop_propagation();
                    stage.menu_at = None;
                    stage.begin_close(cx);
                },
            )),
        );
        Some(menu)
    }

    pub(super) fn render_action_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.cfg.toast_show_actions {
            return None;
        }
        let mut actions = div()
            .absolute()
            .bottom(px(6.))
            .left(px(6.))
            .right(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(3.));

        if self.cfg.toast_pin_enabled {
            actions = actions.child(
                crate::widgets::overlay_icon_button_active(
                    ElementId::Name("toast-action-pin".into()),
                    crate::icons::Icon::Pin,
                    self.pinned,
                )
                .on_click(cx.listener(|stage, _, _, cx| {
                    cx.stop_propagation();
                    stage.toggle_pin(cx);
                })),
            );
        }

        actions = actions
            .child(
                crate::widgets::overlay_icon_button(
                    ElementId::Name("toast-action-copy-img".into()),
                    crate::icons::Icon::Copy,
                )
                .on_click(cx.listener(|stage, _, _, cx| {
                    cx.stop_propagation();
                    stage.copy_image(cx);
                })),
            )
            .child(
                crate::widgets::overlay_icon_button(
                    ElementId::Name("toast-action-copy-file".into()),
                    crate::icons::Icon::Grid,
                )
                .on_click(cx.listener(|stage, _, _, cx| {
                    cx.stop_propagation();
                    stage.copy_file(cx);
                })),
            )
            .child(
                crate::widgets::overlay_icon_button(
                    ElementId::Name("toast-action-open-folder".into()),
                    crate::icons::Icon::Viewfinder,
                )
                .on_click(cx.listener(|stage, _, _, cx| {
                    cx.stop_propagation();
                    stage.open_folder(cx);
                })),
            )
            .child(
                crate::widgets::overlay_icon_button(
                    ElementId::Name("toast-action-markup".into()),
                    crate::icons::Icon::Pen,
                )
                .on_click(cx.listener(|stage, _, window, cx| {
                    cx.stop_propagation();
                    stage.hand_off_to_editor(window, cx);
                })),
            )
            .child(
                crate::widgets::overlay_icon_button(
                    ElementId::Name("toast-action-delete".into()),
                    crate::icons::Icon::Trash,
                )
                .on_click(cx.listener(|stage, _, _, cx| {
                    cx.stop_propagation();
                    stage.delete_capture(cx);
                })),
            );
        Some(actions)
    }
}

pub(super) fn reveal_in_folder(path: &Path) {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let uri = format!(
        "file://{}",
        iris_lib::dragcopy::uri_encode_path(&abs.to_string_lossy())
    );
    let parent = abs.parent().unwrap_or(path).to_path_buf();
    std::thread::spawn(move || {
        let dbus_status = std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ])
            .status();
        if dbus_status.map(|s| s.success()).unwrap_or(false) {
            return;
        }
        let _ = std::process::Command::new("xdg-open").arg(&parent).spawn();
    });
}

/// One soft, deep shadow: the only thing separating the thumbnail
/// from the desktop. No contact layer, no hairline. During the
/// entrance the shadow fades in with the card's visible fraction, so
/// the blur never arrives ahead of the pixels casting it.
pub(crate) fn card_shadow(visibility: f32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.36 * visibility),
            offset: point(px(0.), px(12.)),
            blur_radius: px(32.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.20 * visibility),
            offset: point(px(0.), px(2.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
        },
    ]
}
