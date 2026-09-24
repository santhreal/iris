//! Toast stage actions and UI action helpers.

use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

use super::*;
use crate::{motion, pipeline};

impl ToastStage {
    /// Build the stage from an already-scaled thumbnail. The scaling
    /// itself runs on the background executor.
    pub(super) fn from_parts(path: &Path, thumb: Thumb) -> Self {
        Self {
            path: path.to_path_buf(),
            thumb,
            rescaling: false,
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
            status: None,
            busy: false,
            bar: Bar::Shown,
        }
    }

    /// Fade in the action bar a landed toast opened without.
    pub(super) fn reveal_bar(&mut self, cx: &mut Context<Self>) {
        if matches!(self.bar, Bar::Hidden) {
            self.bar = Bar::FadingIn(Instant::now());
            cx.notify();
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
                if stage.hover_paused || stage.menu_at.is_some() || stage.busy {
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
                crate::widgets::release_render(&self.thumb.render, cx);
                window.remove_window();
                crate::notice::failed(cx, "Editor did not open", &e);
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

    /// Show `text` over the card and give it a full dismiss duration
    /// from now, so a result that lands late is still read.
    pub(super) fn show_status(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = Some(text.into());
        self.arm_dismiss(cx);
        cx.notify();
    }

    /// Run `job` off the UI thread and show its message, or its error,
    /// on the card.
    fn report(
        &mut self,
        job: impl FnOnce() -> Result<String, String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let task = cx.background_executor().spawn(async move { job() });
        cx.spawn(async move |this, cx| {
            let msg = task.await.unwrap_or_else(|e| e);
            this.update(cx, |stage, cx| {
                stage.busy = false;
                stage.show_status(msg, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn copy_image(&mut self, cx: &mut Context<Self>) {
        // PNG decode of a large capture is too slow for the UI thread.
        let path = self.path.clone();
        self.report(
            move || crate::pipeline::copy_image_file(&path).map(|()| "Copied image".into()),
            cx,
        );
    }

    pub(super) fn copy_file(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        self.report(
            move || crate::pipeline::copy_file(&path).map(|()| "Copied file".into()),
            cx,
        );
    }

    /// OCR the capture to the clipboard. tesseract takes hundreds of
    /// ms on a large capture; the card holds until it finishes.
    pub(super) fn copy_text(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        self.show_status("Reading text…", cx);
        let path = self.path.clone();
        self.report(
            move || {
                pipeline::copy_ocr_text(&path)
                    .map(|text| format!("Copied {} characters", text.chars().count()))
            },
            cx,
        );
    }

    pub(super) fn open_folder(&self) {
        crate::sys::reveal::reveal(&self.path);
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
                Err(e) => stage.show_status(format!("Delete failed: {e}"), cx),
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
                self.open_folder();
            }
            iris_lib::config::ToastClickAction::None => {}
        }
    }

    /// Right-click context menu, macOS style: Markup, Copy, Copy text,
    /// Pin, Delete, Close. Fades in over 120ms rising 3px; shares the
    /// exit opacity so it never outlives the card it belongs to.
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
                    stage.copy_text(cx);
                },
            )),
        );
        menu = menu.child(
            crate::widgets::menu_row("toast-pin", "Pin to screen").on_click(cx.listener(
                |stage, _, _window, cx| {
                    cx.stop_propagation();
                    stage.menu_at = None;
                    crate::pin::open(cx, &stage.path);
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
                    stage.open_folder();
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

/// The last action's result over the card's lower edge. The card is
/// the toast's only surface, so a failure shows here, not only in the
/// log. Text wraps to the card width; the card clips the overflow.
pub(super) fn status_band(text: SharedString) -> Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .px(px(8.))
        .py(px(6.))
        .bg(theme::alpha(theme::BG, 0.86))
        .text_size(px(theme::TEXT_SMALL))
        .text_color(theme::FG)
        .child(text)
}
