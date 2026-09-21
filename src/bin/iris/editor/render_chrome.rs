//! Editor chrome rendering: backdrop, topbar, sidebar, menus, help sheet, keyboard shortcuts.

use std::time::Instant;

use gpui::*;
use iris_lib::history::Edit;

use super::action::{hex_rgba, Tool, Transform, COLORS, TOOLS};
use super::Editor;
use crate::icons::Icon;
use crate::{motion, theme};

impl Editor {
    pub(super) fn handle_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = ev.keystroke.key.as_str();
        if self.text_entry.is_some() {
            match key {
                "enter" => {
                    self.commit_text(true);
                }
                "escape" => {
                    self.commit_text(false);
                }
                "backspace" => {
                    if let Some(entry) = &mut self.text_entry {
                        entry.buffer.pop();
                        entry.caret = SharedString::from(format!("{}▏", entry.buffer));
                        entry.buffer_str = SharedString::from(entry.buffer.clone());
                    }
                }
                _ => {
                    if !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform {
                        if let Some(ch) = &ev.keystroke.key_char {
                            if let Some(entry) = &mut self.text_entry {
                                entry.buffer.push_str(ch);
                                entry.caret = SharedString::from(format!("{}▏", entry.buffer));
                                entry.buffer_str = SharedString::from(entry.buffer.clone());
                                self.caret_started = Instant::now();
                            }
                        }
                    }
                }
            }
            cx.notify();
            return;
        }
        let meta = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
        match key {
            "escape" => {
                if self.help {
                    self.help = false;
                } else if self.copy_menu {
                    self.copy_menu = false;
                } else if self.crop_rect.is_some() {
                    self.crop_rect = None;
                } else if self.selected.is_some() {
                    self.selected = None;
                } else {
                    self.discard(window, cx);
                }
            }
            "enter" => {
                if self.crop_rect.is_some() {
                    self.apply_crop(cx);
                } else {
                    self.finish(window, cx);
                }
            }
            "delete" | "backspace" => {
                if let Some(i) = self.selected.take() {
                    if i < self.actions.borrow().len() {
                        let removed = self.actions.borrow_mut().remove(i);
                        self.rebuild_for_edit(&Edit::Remove(i, removed.clone()));
                        self.push_edit(Edit::Remove(i, removed));
                    }
                }
            }
            "z" if meta && ev.keystroke.modifiers.shift => {
                self.redo();
            }
            "y" if meta => {
                self.redo();
            }
            "z" if meta => {
                self.undo();
            }
            "s" if meta => {
                self.finish(window, cx);
            }
            "?" | "/" => {
                self.help = !self.help;
            }
            // Tool hotkeys, single letters like Markup/Photoshop.
            // No modifier: the editor owns the window's keys.
            "v" => self.set_tool(Tool::Select, cx),
            "p" => self.set_tool(Tool::Pen, cx),
            "l" => self.set_tool(Tool::Line, cx),
            "a" => self.set_tool(Tool::Arrow, cx),
            "e" => self.set_tool(Tool::Ellipse, cx),
            "r" => self.set_tool(Tool::Rect, cx),
            "t" => self.set_tool(Tool::Text, cx),
            "h" => self.set_tool(Tool::Highlight, cx),
            "b" => self.set_tool(Tool::Blur, cx),
            "c" => self.set_tool(Tool::Crop, cx),
            "n" => self.set_tool(Tool::Counter, cx),
            "f" => {
                self.fill = !self.fill;
            }
            "1" | "2" | "3" => {
                self.stroke = key.as_bytes()[0] - b'1';
            }
            // Zoom: 0 fits, +/- step, space+drag pans.
            "0" => {
                self.zoom = 1.0;
                self.pan = (0.0, 0.0);
            }
            "=" | "+" => {
                self.zoom = (self.zoom * 1.25).min(16.0);
            }
            "-" => {
                self.zoom = (self.zoom / 1.25).max(0.1);
            }
            "space" => {
                self.space_pan = true;
            }
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn handle_key_up(&mut self, ev: &KeyUpEvent, cx: &mut Context<Self>) {
        if ev.keystroke.key == "space" {
            self.space_pan = false;
            cx.notify();
        }
    }

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

    pub(super) fn render_topbar(
        &self,
        topbar: f32,
        title: SharedString,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
            .bg(theme::alpha(theme::BG_ELEV, 0.96))
            .shadow(theme::shadow_float())
            .child(div().text_sm().text_color(theme::FG_DIM).child(title))
            .child({
                let mut buttons = div().flex().items_center().gap(px(6.));
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
            .gap(px(4.))
            .py(px(10.))
            .overflow_y_scroll()
            .rounded(px(12.))
            .bg(theme::alpha(theme::BG_ELEV, 0.96))
            .shadow(theme::shadow_float());
        for (tool, glyph, label) in TOOLS {
            let active = self.tool == tool;
            bar = bar.child(
                crate::widgets::icon_button(ElementId::Name(label.into()), glyph, active, 32.0)
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
                    32.0,
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
                32.0,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.fill = !this.fill;
                cx.notify();
            })),
        );
        bar = bar
            .child(
                div()
                    .h(px(1.))
                    .w(px(28.))
                    .my(px(4.))
                    .flex_shrink_0()
                    .bg(theme::HAIRLINE),
            )
            .child(
                crate::widgets::icon_button(
                    ElementId::Name("action-undo".into()),
                    Icon::Undo,
                    false,
                    32.0,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.undo();
                    cx.notify();
                })),
            )
            .child(
                crate::widgets::icon_button(
                    ElementId::Name("action-redo".into()),
                    Icon::Redo,
                    false,
                    32.0,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.redo();
                    cx.notify();
                })),
            )
            .child(
                crate::widgets::icon_button(
                    ElementId::Name("action-clear".into()),
                    Icon::Trash,
                    false,
                    32.0,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.clear();
                    cx.notify();
                })),
            )
            .child(
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
                    .w(px(18.))
                    .h(px(18.))
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
                    ("Tools", "V P L A E R T H B C".to_string()),
                    ("Stroke width", "1 / 2 / 3".to_string()),
                    ("Save and close", "Ctrl+S / Enter".to_string()),
                    ("Discard", "Esc".to_string()),
                    ("Undo / Redo", "Ctrl+Z / Ctrl+Shift+Z".to_string()),
                    ("Delete selected action", "Delete".to_string()),
                    ("Apply crop", "Enter".to_string()),
                    ("Copy image / file / path", "Copy menu".to_string()),
                    ("This sheet", "?".to_string()),
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
