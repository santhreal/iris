//! Editor keyboard: text entry, tool hotkeys, history, zoom and pan.

use std::time::Instant;

use gpui::*;
use iris_lib::history::Edit;

use super::action::tool_for_key;
use super::Editor;
use crate::widgets;

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
        if self.text_entry.is_some() {
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
                        if widgets::edit_line(&mut entry.buffer, &ev.keystroke, paste) {
                            entry.buffer_str = SharedString::from(entry.buffer.clone());
                            self.caret_started = Instant::now();
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
                self.help = !self.help;
            }
            "w" if meta => {
                self.finish(window, cx);
            }
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
