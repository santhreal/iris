//! Editor keyboard: text entry, tool hotkeys, history, zoom and pan.

use std::time::Instant;

use gpui::*;
use iris_lib::history::Edit;

use super::action::{step_color, tool_for_key};
use super::Editor;
use crate::widgets;
use crate::zoom::KEY_STEP;

/// Image pixels an arrow key moves the selection with `Shift`; one
/// without.
const NUDGE_SHIFT: f32 = 10.0;

impl Editor {
    pub(super) fn handle_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = ev.keystroke.key.as_str();
        if self.copy_menu || self.more_menu {
            self.toolbar_menu_key(key, window, cx);
            cx.notify();
            return;
        }
        let meta = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
        // The shortcut sheet is modal: no key acts on the image under
        // it.
        if self.help {
            match key {
                "w" if meta => self.finish(window, cx),
                "escape" | "?" | "/" => self.help = false,
                _ => return,
            }
            cx.notify();
            return;
        }
        // Undo, Redo, Copy, Save, and Close run with a label open, as
        // their toolbar buttons do: each settles the label first.
        let command = meta && matches!(key, "z" | "y" | "c" | "s" | "w");
        if self.text_entry.is_some() && !command {
            match key {
                "enter" => {
                    self.commit_text(true);
                }
                "escape" => {
                    self.commit_text(false);
                }
                _ => {
                    if let Some(entry) = &mut self.text_entry {
                        let paste = || cx.read_from_clipboard().and_then(|item| item.text());
                        let key = widgets::edit_line(&mut entry.line, &ev.keystroke, paste);
                        if key == widgets::LineKey::Edited {
                            entry.buffer_str = SharedString::from(entry.line.text().to_string());
                        }
                        if key != widgets::LineKey::Ignored {
                            self.caret_started = Instant::now();
                        }
                    }
                }
            }
            cx.notify();
            return;
        }
        match key {
            "escape" => {
                if self.crop_rect.is_some() {
                    self.crop_rect = None;
                } else if self.selected.is_some() {
                    self.selected = None;
                } else {
                    self.escape_discard(window, cx);
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
            "left" | "right" | "up" | "down" if !meta => {
                let step = if ev.keystroke.modifiers.shift {
                    NUDGE_SHIFT
                } else {
                    1.0
                };
                self.nudge(match key {
                    "left" => (-step, 0.0),
                    "right" => (step, 0.0),
                    "up" => (0.0, -step),
                    _ => (0.0, step),
                });
            }
            "c" if meta => {
                self.copy_variant("image", cx);
            }
            "z" if meta && ev.keystroke.modifiers.shift => {
                self.redo(cx);
            }
            "y" if meta => {
                self.redo(cx);
            }
            "z" if meta => {
                self.undo(cx);
            }
            "s" if meta => {
                self.finish(window, cx);
            }
            "?" | "/" => {
                self.help = true;
            }
            "w" if meta => {
                self.finish(window, cx);
            }
            "f" => self.toggle_fill(),
            "1" | "2" | "3" => self.set_stroke(key.as_bytes()[0] - b'1'),
            "[" | "]" if !meta => self.set_color(step_color(self.color, key == "]")),
            // Zoom: 0 fits, +/- step, space+drag pans.
            "0" => {
                self.zoom = 1.0;
                self.pan = (0.0, 0.0);
            }
            "=" | "+" | "-" => {
                // A key step zooms about the stage's center, so the
                // part of the image in view stays in view.
                let size = window.bounds().size;
                let top = super::render_chrome::STAGE_TOP;
                let at = (
                    f32::from(size.width) / 2.0,
                    top + (f32::from(size.height) - top) / 2.0,
                );
                let factor = if key == "-" { 1.0 / KEY_STEP } else { KEY_STEP };
                self.zoom_at(at, factor, window);
            }
            "space" => {
                self.space_pan = true;
            }
            // The editor's actions menu: More, with Copy beside it.
            "menu" => self.open_toolbar_menu(false, true),
            "f10" if ev.keystroke.modifiers.shift => self.open_toolbar_menu(false, true),
            _ if !meta => {
                if let Some(tool) = tool_for_key(key) {
                    self.set_tool(tool, cx);
                }
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
}
