use std::time::Instant;

use gpui::*;

use crate::theme;

use super::{visible_rows, Library, CARD_W, GAP};

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selecting = !self.selected.is_empty();
        let this_help = self.help;

        // Right-side toolbar cluster, handed to the shared frame.
        let mut cluster: Vec<AnyElement> = Vec::new();
        if selecting {
            cluster.push(
                div()
                    .text_xs()
                    .text_color(theme::FG_DIM)
                    .child(self.sel_label.clone())
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("sel-copy", "Copy", false)
                    .on_click(cx.listener(|this, _, _, cx| this.copy_selection(cx)))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("sel-delete", "Delete", false)
                    .on_click(cx.listener(|this, _, _, cx| this.delete_selection(cx)))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("sel-clear", "Clear", false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected.clear();
                        this.sel_dirty = true;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        } else {
            cluster.push(
                div()
                    .text_xs()
                    .text_color(theme::FG_FAINT)
                    .child(self.count_label.clone())
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("capture", "Capture", true)
                    .on_click(cx.listener(|this, _, _, cx| this.start_capture(cx)))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("settings", "Settings", false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Err(e) = crate::settings::open(cx) {
                            this.status = Some(e);
                            cx.notify();
                        }
                    }))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("help", "?", false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.help = !this.help;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        let mut grid = div()
            .id("grid")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p(px(GAP))
            .flex()
            .flex_wrap()
            .gap(px(GAP))
            .items_start()
            .content_start()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    // A card's own mousedown set drag_start first; only
                    // empty grid space starts a rubber band.
                    if this.drag_start.is_none() {
                        this.band = Some((
                            ev.position.x.into(),
                            ev.position.y.into(),
                            ev.position.x.into(),
                            ev.position.y.into(),
                        ));
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if let Some(b) = &mut this.band {
                    b.2 = ev.position.x.into();
                    b.3 = ev.position.y.into();
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, window, cx| {
                    this.finish_band(ev, window, cx);
                }),
            );

        if self.entries.is_empty() {
            grid = grid.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(theme::FG_FAINT)
                    .child(self.empty_label.clone()),
            );
        }

        let cascade = self.opened.elapsed().as_secs_f32() < 1.6;
        // The cache fills from the background prefetch; a card whose
        // thumb has not landed draws without its image until then.
        // A set, not a nested scan: retain() over the cache against a
        // per-entry linear probe is O(cache * entries) PathBuf
        // compares. Rebuilt only when entries changed: a per-frame
        // rebuild is O(entries) PathBuf hashing for a cache that
        // almost never has stale keys.
        if self.entries_dirty {
            self.entries_dirty = false;
            let live_paths: std::collections::HashSet<&std::path::Path> =
                self.entries.iter().map(|e| e.path.as_path()).collect();
            self.thumb_cache
                .retain(|p, _| live_paths.contains(p.as_path()));
        }
        // Advance pointer-coupled springs by the real frame delta;
        // keep rendering until everything settles.
        let now = Instant::now();
        let dt = self
            .last_frame
            .map(|t| (now - t).as_secs_f32())
            .unwrap_or(1.0 / 60.0);
        self.last_frame = Some(now);
        let mut live = false;
        let n = self.entries.len();
        if let Some(h) = self.hovered {
            self.springs.entry(h).or_default();
        }
        self.springs.retain(|i, _| *i < n);
        for (i, s) in self.springs.iter_mut() {
            let target = if self.hovered == Some(*i) { 1.0 } else { 0.0 };
            s.to(target, dt);
            if !s.settled(target) {
                live = true;
            }
        }
        // Membership is checked per entry below and per card; a Vec
        // scan is O(n*m) PathBuf compares a frame. The set rebuilds
        // only when a mutation flagged it dirty.
        if self.sel_dirty {
            self.sel_dirty = false;
            self.sel_set = self.selected.iter().cloned().collect();
            self.sel_label = SharedString::from(format!("{} selected", self.selected.len()));
        }
        let sel_set = &self.sel_set;
        self.sel_springs.retain(|i, _| *i < n);
        for i in 0..n {
            let target = if sel_set.contains(&self.entries[i].path) {
                1.0
            } else {
                0.0
            };
            // get_mut, not entry(): an unselected card with no live
            // spring must not insert a permanent map entry per frame.
            let s = match self.sel_springs.get_mut(&i) {
                Some(s) => s,
                None if target == 0.0 => continue,
                None => self.sel_springs.entry(i).or_default(),
            };
            if target == 0.0 && s.settled(0.0) && s.value == 0.0 {
                continue;
            }
            s.to(target, dt);
            if !s.settled(target) {
                live = true;
            }
        }
        {
            let target = if self.pressed.is_some() { 1.0 } else { 0.0 };
            self.press_spring.to(target, dt);
            if !self.press_spring.settled(target) {
                live = true;
            }
        }
        if live {
            window.request_animation_frame();
        }
        // Virtualize the grid: build a card subtree only for the rows
        // that intersect the scroll viewport. A full render is
        // O(entries) DOM construction a frame, paid on every scroll
        // and animation frame; a large library turns that into the
        // judder the thumb cache was added to fix.
        let width: f32 = window.bounds().size.width.into();
        let cols = ((width - GAP) / (CARD_W + GAP)).floor().max(1.0) as usize;
        // ScrollHandle::offset is <= 0 (negative when scrolled down);
        // the positive scroll amount is -offset.y.
        let scroll_top: f32 = -f32::from(self.scroll.offset().y);
        let viewport_h: f32 = self.scroll.bounds().size.height.into();
        let (first_row, last_row, top_h, bottom_h) = visible_rows(n, cols, scroll_top, viewport_h);
        // A full-width spacer occupies a whole wrap line, standing in
        // for the rows above and below so the rendered cards land at
        // their un-virtualized offsets and the scroll range is kept.
        if top_h > 0.0 {
            grid = grid.child(div().w_full().h(px(top_h)));
        }
        let last_i = ((last_row + 1) * cols).min(n);
        for index in first_row * cols..last_i {
            let entry = &self.entries[index];
            let amt = self.springs.get(&index).map(|s| s.value).unwrap_or(0.0);
            // Open cascade: cards rise and fade in with a 25ms stagger.
            let et = if cascade {
                let t = ((self.opened.elapsed().as_secs_f32() - index as f32 * 0.025) / 0.35)
                    .clamp(0.0, 1.0);
                if t < 1.0 {
                    window.request_animation_frame();
                }
                t
            } else {
                1.0
            };
            grid = grid.child(self.card(index, entry, amt, et, sel_set, cx));
        }
        if bottom_h > 0.0 {
            grid = grid.child(div().w_full().h(px(bottom_h)));
        }

        let mut root = div()
            .size_full()
            .font_family(theme::FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                let meta = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
                match key {
                    "escape" if this.help => {
                        this.help = false;
                    }
                    "?" | "/" => {
                        this.help = !this.help;
                    }
                    "escape" if !this.selected.is_empty() => {
                        this.selected.clear();
                        this.sel_dirty = true;
                    }
                    "escape" => {
                        window.remove_window();
                        return;
                    }
                    "delete" | "backspace" if !this.selected.is_empty() => {
                        this.delete_selection(cx);
                    }
                    "a" if meta => {
                        this.selected = this.entries.iter().map(|e| e.path.clone()).collect();
                        this.sel_dirty = true;
                    }
                    _ => return,
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    // Released anywhere: the pressed card un-sinks.
                    if this.pressed.is_some() {
                        this.pressed = None;
                        cx.notify();
                    }
                }),
            )
            .child(crate::widgets::window_frame(
                "Iris",
                self.class.clone(),
                cluster,
                grid,
            ));

        if let Some((x0, y0, x1, y1)) = self.band {
            let (bx, by) = (x0.min(x1), y0.min(y1));
            root = root.child(
                div()
                    .absolute()
                    .left(px(bx))
                    .top(px(by))
                    .w(px((x1 - x0).abs()))
                    .h(px((y1 - y0).abs()))
                    .bg(theme::alpha(theme::ACCENT, 0.12))
                    .border_1()
                    .border_color(theme::alpha(theme::ACCENT, 0.6))
                    .rounded(px(4.)),
            );
        }

        if this_help {
            root = root.child(
                crate::widgets::shortcuts_sheet(vec![
                    ("Capture region", self.cfg.capture_hotkey.clone().into()),
                    ("Record window", "Ctrl+Shift+R".into()),
                    ("Open in editor", "Click".into()),
                    ("Select all", "Ctrl+A".into()),
                    ("Delete selection", "Delete".into()),
                    ("Clear selection", "Esc".into()),
                    ("Fullscreen", "Double-click title bar".into()),
                    ("This sheet", "?".into()),
                ])
                .on_click(cx.listener(|this, _, _, cx| {
                    this.help = false;
                    cx.notify();
                })),
            );
        }

        if let Some(status) = &self.status {
            root = root.child(crate::widgets::status_pill(status, 10.0));
        }

        root
    }
}
