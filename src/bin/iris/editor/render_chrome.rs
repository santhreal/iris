//! Editor chrome rendering: backdrop, topbar, sidebar, menus, help sheet.

use std::time::Instant;

use gpui::*;

use super::action::{hex_rgba, Transform, COLORS, TOOLS};
use super::Editor;
use crate::icons::Icon;
use crate::{motion, theme};

impl Editor {
    pub(super) fn render_backdrop(&self, chrome: f32, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .left(px(64.))
            .top(px(52.))
            .right_0()
            .bottom_0()
            .id("backdrop")
            .opacity(chrome)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev: &MouseDownEvent, _, _| {
                    // Clicking outside the image only settles an
                    // open text entry; work is never thrown away
                    // by a stray click. Discard is explicit.
                    this.commit_text(true);
                }),
            )
    }

    /// Undo, redo, and clear. The group stops presses from reaching the
    /// title bar's move.
    fn history_buttons(&self, cx: &mut Context<Self>) -> Div {
        let button = |id: &'static str, glyph: Icon, run: fn(&mut Self)| {
            crate::widgets::icon_button(id, glyph, false, theme::CONTROL_H).on_click(cx.listener(
                move |this, _, _, cx| {
                    run(this);
                    cx.notify();
                },
            ))
        };
        div()
            .flex()
            .items_center()
            .gap(px(4.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(button("action-undo", Icon::Undo, Self::undo))
            .child(button("action-redo", Icon::Redo, Self::redo))
            .child(button("action-clear", Icon::Trash, Self::clear))
    }

    pub(super) fn render_topbar(
        &self,
        topbar: f32,
        title: SharedString,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The topbar is the window's title bar, opaque and over the
        // stage: a zoomed image runs under it and never takes its
        // presses, hover, or wheel.
        div()
            .absolute()
            .top(px(12.))
            .left(px(12.))
            .right(px(12.))
            .h(px(48.))
            .opacity(topbar)
            .flex()
            .items_center()
            .justify_between()
            .px(px(14.))
            .rounded(px(12.))
            .bg(theme::BG_ELEV)
            .shadow(theme::shadow_float())
            .occlude()
            .child(crate::widgets::move_handle(
                crate::widgets::Double::TitleBar,
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(theme::TEXT_BODY))
                            .text_color(theme::FG_DIM)
                            .child(title),
                    )
                    .child(self.history_buttons(cx)),
            )
            .child({
                let mut buttons = div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    // Buttons handle their own presses; the bar's move
                    // must not take them.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
                buttons = buttons
                    .child(
                        crate::widgets::button("btn-discard", "Discard", false)
                            .on_click(cx.listener(|this, _, window, cx| this.discard(window, cx))),
                    )
                    .child(
                        crate::widgets::button("btn-copy-text", "Copy text", false)
                            .on_click(cx.listener(|this, _, _, cx| this.copy_text_ocr(cx))),
                    )
                    .child(
                        div().relative().child(
                            crate::widgets::button_with_icon(
                                "btn-copy",
                                "Copy",
                                Icon::ChevronDown,
                                false,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.copy_menu = !this.copy_menu;
                                this.copy_menu_opened = None;
                                cx.notify();
                            })),
                        ),
                    )
                    .child(
                        crate::widgets::button("btn-rotate", "Rotate", false).on_click(
                            cx.listener(|this, _, _, cx| this.transform(Transform::Rot90, cx)),
                        ),
                    )
                    .child(crate::widgets::button("btn-flip", "Flip", false).on_click(
                        cx.listener(|this, _, _, cx| this.transform(Transform::FlipH, cx)),
                    ))
                    .child(
                        crate::widgets::button("btn-flipv", "Flip V", false).on_click(
                            cx.listener(|this, _, _, cx| this.transform(Transform::FlipV, cx)),
                        ),
                    )
                    .child(
                        crate::widgets::button("btn-done", "Done", true)
                            .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx))),
                    )
                    .child(
                        crate::widgets::button("btn-help", "?", false).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.help = !this.help;
                                cx.notify();
                            },
                        )),
                    );
                buttons
            })
    }

    pub(super) fn render_sidebar(&self, sidebar: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .id("sidebar")
            .absolute()
            .left(px(12.))
            .top(px(72.))
            .bottom(px(12.))
            .w(px(48.))
            .opacity(sidebar)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .py(px(10.))
            .overflow_y_scroll()
            .rounded(px(12.))
            .bg(theme::BG_ELEV)
            .shadow(theme::shadow_float())
            // Over the stage, as the topbar is.
            .occlude();
        for (tool, glyph, label) in TOOLS {
            let active = self.tool == tool;
            bar = bar.child(
                crate::widgets::icon_button(
                    ElementId::Name(label.into()),
                    glyph,
                    active,
                    theme::CONTROL_H,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.commit_text(true);
                    this.tool = tool;
                    cx.notify();
                })),
            );
        }
        bar = bar.child(
            div()
                .h(px(1.))
                .w(px(28.))
                .my(px(4.))
                .flex_shrink_0()
                .bg(theme::HAIRLINE),
        );
        // Stroke width stops for new vector actions.
        for (i, glyph) in [Icon::Stroke1, Icon::Stroke2, Icon::Stroke3]
            .into_iter()
            .enumerate()
        {
            let active = self.stroke == i as u8;
            bar = bar.child(
                crate::widgets::icon_button(
                    ElementId::NamedInteger("stroke".into(), i as u64),
                    glyph,
                    active,
                    theme::CONTROL_H,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.stroke = i as u8;
                    cx.notify();
                })),
            );
        }
        // Fill toggle for Rect/Ellipse: paints the interior
        // instead of just the outline.
        bar = bar.child(
            crate::widgets::icon_button(
                ElementId::Name("fill-toggle".into()),
                Icon::Rect,
                self.fill,
                theme::CONTROL_H,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.fill = !this.fill;
                cx.notify();
            })),
        );
        bar = bar.child(
            div()
                .h(px(1.))
                .w(px(28.))
                .my(px(4.))
                .flex_shrink_0()
                .bg(theme::HAIRLINE),
        );
        // Swatches in a two-column grid; the active color gets a
        // ring, like Markup's swatch selection.
        let mut swatches = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(4.))
            .w(px(40.));
        for c in COLORS {
            let active = c == self.color;
            swatches = swatches.child(
                div()
                    .id(ElementId::Name(c.into()))
                    .w(px(16.))
                    .h(px(16.))
                    .flex_shrink_0()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_color(if active {
                        theme::FG
                    } else {
                        theme::alpha(theme::FG, 0.0)
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .w(px(12.))
                            .h(px(12.))
                            .rounded_full()
                            .bg(hex_rgba(c))
                            .border_1()
                            .border_color(theme::HAIRLINE),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.color = c;
                        cx.notify();
                    })),
            );
        }
        bar.child(swatches)
    }

    pub(super) fn render_menus_and_overlays(
        &mut self,
        mut root: Stateful<Div>,
        topbar: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        // Copy dropdown menu, fading in over 120ms. Painted after the
        // stage so its rows are never under the canvas.
        if self.copy_menu {
            let opened = *self.copy_menu_opened.get_or_insert_with(Instant::now);
            let mt = (opened.elapsed().as_secs_f32() / motion::tempo(motion::FADE).as_secs_f32())
                .min(1.0);
            if mt < 1.0 {
                window.request_animation_frame();
            }
            let mut menu = crate::widgets::menu()
                .absolute()
                .top(px(48. + 3.0 * (1.0 - motion::ease_out_cubic(mt))))
                .right(px(86.))
                .opacity(mt);
            for (id, label) in [
                ("copy-image", "Copy Image"),
                ("copy-file", "Copy File"),
                ("copy-path", "Copy Path"),
            ] {
                let variant = id.strip_prefix("copy-").unwrap_or(id);
                menu = menu.child(crate::widgets::menu_row(id, label).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.copy_variant(variant, cx);
                    },
                )));
            }
            root = root.child(menu);
        }

        // Transient status line (copy results, OCR counts, save errors).
        if let Some(status) = &self.status {
            root = root.child(crate::widgets::status_pill(status, 76.0));
        }

        if self.help {
            root = root.child(
                crate::widgets::shortcuts_sheet(vec![
                    ("Tools", "V P L A E R T H B C N".into()),
                    ("Fill shapes", "F".into()),
                    ("Stroke width", "1 / 2 / 3".into()),
                    ("Zoom fit / in / out", "0 / + / -".into()),
                    ("Pan", "Space + drag".into()),
                    ("Save and close", "Ctrl+S / Enter".into()),
                    ("Discard", "Esc".into()),
                    ("Undo / Redo", "Ctrl+Z / Ctrl+Shift+Z".into()),
                    ("Delete selected action", "Delete".into()),
                    ("Apply crop", "Enter".into()),
                    ("Copy image / file / path", "Copy menu".into()),
                    ("This sheet", "?".into()),
                ])
                .opacity(topbar)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.help = false;
                    cx.notify();
                })),
            );
        }

        root
    }
}
