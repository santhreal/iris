//! The library's keyboard paths: arrow, Home, End, and page
//! selection, open, preview, the card menu, select all, trash, and the
//! window keys.

use gpui::*;

use super::layout::{Layout, Step};
use super::Library;

/// The selection after an arrow key `step`: the new cursor, the new
/// range anchor, and the inclusive index range to select. With no card
/// under the cursor the move lands on the first card, or on the last for
/// End. `extend` (Shift) grows the range from the anchor; a plain move
/// selects the one card it lands on. None for an empty grid.
pub(super) fn arrow(
    layout: &Layout,
    n: usize,
    cursor: Option<usize>,
    anchor: Option<usize>,
    step: Step,
    extend: bool,
) -> Option<(usize, usize, (usize, usize))> {
    if n == 0 {
        return None;
    }
    let from = cursor.filter(|c| *c < n);
    let next = match from {
        Some(c) => layout.step(c, step),
        None if step == Step::Last => n - 1,
        None => 0,
    };
    let anchor = if extend {
        anchor.filter(|a| *a < n).or(from).unwrap_or(next)
    } else {
        next
    };
    Some((next, anchor, (anchor.min(next), anchor.max(next))))
}

/// The grid move for `key`, if any: an arrow, Home, End, Page Up, or
/// Page Down, which moves by `page` pixels.
pub(super) fn arrow_step(key: &str, page: f32) -> Option<Step> {
    match key {
        "left" => Some(Step::Left),
        "right" => Some(Step::Right),
        "up" => Some(Step::Up),
        "down" => Some(Step::Down),
        "home" => Some(Step::First),
        "end" => Some(Step::Last),
        "pageup" => Some(Step::PageUp(page)),
        "pagedown" => Some(Step::PageDown(page)),
        _ => None,
    }
}

impl Library {
    pub(super) fn on_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.leaving {
            return;
        }
        let ks = &ev.keystroke;
        let key = ks.key.as_str();
        let meta = ks.modifiers.control || ks.modifiers.platform;
        // An open menu takes the keyboard: no key acts on the grid
        // under it.
        if self.menu.is_some() {
            self.menu_key(key, window, cx);
            return;
        }
        if meta {
            match key {
                "a" => self.select_all(),
                "c" if !self.selected.is_empty() => self.copy_selection(cx),
                "w" => {
                    window.remove_window();
                    return;
                }
                "," => {
                    if let Err(e) = crate::settings::open(cx) {
                        self.set_status(e, cx);
                    }
                }
                _ => return,
            }
            cx.notify();
            return;
        }
        let page = f32::from(self.scroll.bounds().size.height);
        if let Some(step) = arrow_step(key, page) {
            self.move_cursor(step, ks.modifiers.shift, cx);
            return;
        }
        if self.preview.is_some() {
            match key {
                "escape" | "space" => self.close_preview(cx),
                "enter" => {
                    let index = self.preview.as_ref().map(|p| p.index);
                    self.close_preview(cx);
                    if let Some(i) = index {
                        self.open_entry(i, window, cx);
                    }
                }
                _ => {}
            }
            return;
        }
        match key {
            "escape" if self.help => self.help = false,
            "?" | "/" => self.help = !self.help,
            "escape" if !self.selected.is_empty() => {
                self.selected.clear();
                self.sel_dirty = true;
            }
            "escape" => {
                window.remove_window();
                return;
            }
            "enter" => {
                if let Some(i) = self.focused() {
                    self.open_entry(i, window, cx);
                }
                return;
            }
            "space" => {
                if let Some(i) = self.focused() {
                    self.open_preview(i, cx);
                }
                return;
            }
            "menu" => {
                self.open_menu_at_focus(cx);
                return;
            }
            "f10" if ks.modifiers.shift => {
                self.open_menu_at_focus(cx);
                return;
            }
            "delete" | "backspace" if !self.selected.is_empty() => self.trash_selection(cx),
            _ => return,
        }
        cx.notify();
    }

    /// The card Return opens and Space previews: the cursor's, or the
    /// first selected one.
    pub(super) fn focused(&self) -> Option<usize> {
        let entries = self.listing.entries();
        self.cursor.filter(|c| *c < entries.len()).or_else(|| {
            let first = self.selected.first()?;
            entries.iter().position(|e| e.path == *first)
        })
    }

    fn select_all(&mut self) {
        self.selected = self
            .listing
            .entries()
            .iter()
            .map(|e| e.path.clone())
            .collect();
        self.sel_dirty = true;
    }

    /// A grid move key: move the cursor and the selection with it, keep the
    /// card in view, and follow it with the preview when one is open.
    fn move_cursor(&mut self, step: Step, extend: bool, cx: &mut Context<Self>) {
        let n = self.listing.entries().len();
        let cursor = self.focused();
        let Some((next, anchor, (lo, hi))) =
            arrow(&self.layout, n, cursor, self.anchor, step, extend)
        else {
            return;
        };
        self.cursor = Some(next);
        self.anchor = Some(anchor);
        self.selected = self.listing.entries()[lo..=hi]
            .iter()
            .map(|e| e.path.clone())
            .collect();
        self.sel_dirty = true;
        self.reveal(next);
        if self.preview.as_ref().is_some_and(|p| p.index != next) {
            self.open_preview(next, cx);
        }
        cx.notify();
    }

    /// Scroll the least distance that shows entry `i`'s card, with its
    /// day header when the card is in the day's first row.
    pub(super) fn reveal(&self, i: usize) {
        let Some((top, bottom)) = self.layout.reveal_span(i) else {
            return;
        };
        let view_h = f32::from(self.scroll.bounds().size.height);
        if view_h <= 0.0 {
            return;
        }
        let at = -f32::from(self.scroll.offset().y);
        let to = if top < at {
            top
        } else if bottom + super::layout::BOTTOM_PAD > at + view_h {
            bottom + super::layout::BOTTOM_PAD - view_h
        } else {
            return;
        };
        let max = (self.layout.height - view_h).max(0.0);
        self.scroll
            .set_offset(point(px(0.), px(-to.clamp(0.0, max))));
    }
}
