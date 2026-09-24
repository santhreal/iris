//! Editor keyboard: text entry, tool hotkeys, history, zoom and pan.

use std::time::Instant;

use gpui::*;
use iris_lib::history::Edit;

use super::action::Tool;
use super::Editor;

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
}
