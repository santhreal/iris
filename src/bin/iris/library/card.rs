use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::*;
use iris_lib::library::{self, CaptureEntry};

use crate::{pipeline, theme};

use super::{Library, CARD_W, THUMB_H};

impl Library {
    pub(super) fn card(
        &self,
        index: usize,
        entry: &Rc<CaptureEntry>,
        hover_amt: f32,
        enter: f32,
        sel_set: &std::collections::HashSet<PathBuf>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = sel_set.contains(&entry.path);
        let hovered = self.hovered == Some(index);
        let sel_amt = self.sel_springs.get(&index).map(|s| s.value).unwrap_or(0.0);
        let squish = if self.pressed == Some(index) {
            self.press_spring.value
        } else {
            0.0
        };
        // One Rc bump feeds every closure below; the alternative is a
        // PathBuf clone per closure per render.
        let entry = entry.clone();

        let mut thumb = div()
            .w_full()
            .h(px(THUMB_H))
            .rounded(px(10.))
            .overflow_hidden()
            .bg(theme::SURFACE)
            .shadow(vec![{
                let mut s = theme::shadow_rest();
                s.color.a *= 0.6 + 0.9 * hover_amt;
                s.blur_radius = px((10. + 8.0 * hover_amt) * (1.0 - 0.3 * squish));
                s.offset.y = px(2. + 4.0 * hover_amt + 1.0 * squish);
                s
            }])
            .border_1()
            .border_color(if selected {
                theme::FG
            } else {
                theme::alpha(theme::FG, 0.0)
            });
        thumb = thumb.child(
            div().absolute().top_0().left_0().size_full().child(
                img(ImageSource::Render(
                    self.thumb_cache
                        .get(&entry.path)
                        .cloned()
                        .unwrap_or_else(|| {
                            // One shared transparent tile: minting a
                            // RenderImage per missing thumb per frame
                            // allocates an atlas slot on every paint.
                            static BLANK: std::sync::LazyLock<Arc<RenderImage>> =
                                std::sync::LazyLock::new(|| {
                                    crate::widgets::render_image_from_rgba(1, 1, &[0, 0, 0, 0])
                                });
                            BLANK.clone()
                        }),
                ))
                .size_full()
                .object_fit(ObjectFit::Cover),
            ),
        );

        // Selection circle, Photos-style: solid with a check when
        // selected. Its pop is spring-driven; otherwise it rides
        // the hover lift (or stays faintly up while selecting).
        let selecting = !self.selected.is_empty();
        let circle_vis =
            sel_amt
                .max(hover_amt)
                .max(if selecting && !selected { 0.85 } else { 0.0 });
        let circle = div()
            .id(ElementId::NamedInteger("sel".into(), index as u64))
            .absolute()
            .top(px(6.))
            .left(px(6.))
            .w(px(18.))
            .h(px(18.))
            .rounded_full()
            .border_1()
            .opacity(circle_vis)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer();
        let circle = if selected {
            let p = entry.clone();
            circle
                .bg(theme::ACCENT)
                .border_color(theme::ACCENT)
                .child(crate::icons::icon(
                    crate::icons::Icon::Check,
                    theme::ACCENT_INK,
                    11.0,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_select(p.path.clone());
                    this.anchor = Some(index);
                    cx.notify();
                }))
        } else {
            let p = entry.clone();
            let c = circle
                .bg(theme::alpha(theme::BG, 0.35))
                .border_color(theme::alpha(theme::FG, 0.45 + 0.55 * hover_amt));
            // An invisible circle must not eat clicks meant for the card.
            if circle_vis > 0.0 {
                c.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_select(p.path.clone());
                    this.anchor = Some(index);
                    cx.notify();
                }))
            } else {
                c
            }
        };
        thumb = thumb.child(circle);

        // Hover actions: annotate / copy / delete, bottom-right.
        // They exist only while hovered, fading in with the lift.
        if hovered {
            let copy_path = entry.clone();
            let folder_path = entry.clone();
            let delete_path = entry.clone();
            thumb = thumb.child(
                div()
                    .absolute()
                    .bottom(px(6.))
                    .right(px(6.))
                    .flex()
                    .gap(px(4.))
                    .opacity(hover_amt)
                    .child(
                        crate::widgets::overlay_icon_button(
                            ElementId::NamedInteger("cpy".into(), index as u64),
                            crate::icons::Icon::Copy,
                        )
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            cx.stop_propagation();
                            let path = copy_path.path.clone();
                            let task = cx
                                .background_executor()
                                .spawn(async move { pipeline::copy_image_file(&path) });
                            cx.spawn(async move |this, cx| {
                                let result = task.await;
                                let _ = this.update(cx, |this, cx| {
                                    this.status = Some(
                                        result.map(|_| "copied".to_string()).unwrap_or_else(|e| e),
                                    );
                                    cx.notify();
                                });
                            })
                            .detach();
                        })),
                    )
                    .child(
                        crate::widgets::overlay_icon_button(
                            ElementId::NamedInteger("fld".into(), index as u64),
                            crate::icons::Icon::Folder,
                        )
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            Self::open_containing_folder(&folder_path.path);
                        })),
                    )
                    .child(
                        crate::widgets::overlay_icon_button(
                            ElementId::NamedInteger("del".into(), index as u64),
                            crate::icons::Icon::Close,
                        )
                        .on_click(cx.listener(move |_this, _, _, cx| {
                            cx.stop_propagation();
                            let path = delete_path.path.clone();
                            let task = cx.background_executor().spawn(async move {
                                let err = library::delete(&path).err();
                                (err, library::list())
                            });
                            cx.spawn(async move |this, cx| {
                                let (err, entries) = task.await;
                                let _ = this.update(cx, |this, cx| {
                                    if let Some(e) = err {
                                        this.status = Some(e);
                                    }
                                    this.entries = entries.into_iter().map(Rc::new).collect();
                                    this.entries_dirty = true;
                                    this.entry_names =
                                        this.entries.iter().map(|e| Self::entry_name(e)).collect();
                                    cx.notify();
                                });
                            })
                            .detach();
                        })),
                    ),
            );
        }

        let card_path = entry.clone();

        div()
            .id(ElementId::NamedInteger("card".into(), index as u64))
            .w(px(CARD_W))
            .flex()
            .flex_col()
            .gap(px(8.))
            .opacity(enter)
            .mt(px(
                (1.0 - crate::motion::ease_out_cubic(enter)) * 10.0 + 1.0 * squish
            ))
            .cursor_pointer()
            .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                if *hovering {
                    this.hovered = Some(index);
                } else if this.hovered == Some(index) {
                    this.hovered = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.click_card(index, ev, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |_, _, _, _| {
                    Self::open_containing_folder(&card_path.path);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.drag_start = Some((index, ev.position.x.into(), ev.position.y.into()));
                    this.drag_fired = false;
                    this.pressed = Some(index);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.pressed.is_some() {
                        this.pressed = None;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                if ev.pressed_button != Some(MouseButton::Left) && this.pressed.is_some() {
                    this.pressed = None;
                    cx.notify();
                }
                if this.drag_fired || ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let Some((card, sx, sy)) = this.drag_start else {
                    return;
                };
                let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                if (mx - sx).hypot(my - sy) < 8.0 {
                    return;
                }
                this.drag_fired = true;
                // The index was captured at mousedown; a refresh that
                // shrank entries since then must not panic the drag.
                let Some(entry) = this.entries.get(card) else {
                    this.drag_start = None;
                    return;
                };
                let paths: Vec<PathBuf> = if this.selected.contains(&entry.path) {
                    this.selected.clone()
                } else {
                    vec![entry.path.clone()]
                };
                // The cached RenderImage already holds the pixels
                // (BGRA; the swizzle is symmetric): no disk read or
                // PNG decode on the drag-start path.
                let icon = this.thumb_cache.get(&entry.path).and_then(|img| {
                    let size = img.size(0);
                    // Fused copy+swizzle into the Arc the drag owns:
                    // the cache's BGRA bytes become the icon's RGBA in
                    // one pass.
                    let src = img.as_bytes(0)?;
                    let mut rgba = Vec::with_capacity(src.len());
                    #[allow(clippy::uninit_vec)] // copy_from_slice fills it next
                    unsafe {
                        rgba.set_len(src.len())
                    };
                    rgba.copy_from_slice(src);
                    crate::widgets::swizzle_rgba_bgra(&mut rgba);
                    Some(iris_lib::dragcopy::DragIcon {
                        width: size.width.0 as u32,
                        height: size.height.0 as u32,
                        rgba: std::sync::Arc::new(rgba),
                    })
                });
                if let Err(e) = iris_lib::dragcopy::start_file_drag_at_cursor(paths, icon) {
                    this.status = Some(e);
                }
            }))
            .child(thumb)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(6.))
                    .px(px(2.))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(theme::TEXT_SMALL))
                            .text_color(theme::FG)
                            .child(
                                self.entry_names
                                    .get(index)
                                    .map(|n| n.0.clone())
                                    .unwrap_or_default(),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(theme::TEXT_SMALL))
                            .text_color(theme::FG_FAINT)
                            .child(
                                self.entry_names
                                    .get(index)
                                    .map(|n| n.1.clone())
                                    .unwrap_or_default(),
                            ),
                    ),
            )
    }
}
