//! The library window: the unified toolbar, the virtualized grid of day
//! sections, the empty state, and the overlays over them (rubber band,
//! preview, context menu, shortcuts sheet, status pill).

use std::time::Instant;

use gpui::*;

use crate::icons::{self, Icon};
use crate::theme;
use crate::widgets::{self, tip, ButtonStyle, Toolbar};

use super::layout::{Layout, Row, HEADER_H};
use super::{evict_thumbs, Library, MIN_SIZE, THUMB_KEEP_ROWS};

/// Space between a day title's baseline row and the cards under it.
const HEADER_PAD_B: f32 = 10.0;

impl Library {
    /// Rebuild the selection set and its toolbar label after a mutation
    /// flagged them: membership is checked per card per frame, and a Vec
    /// scan is O(cards * selection) PathBuf compares.
    fn sync_selection(&mut self) {
        if self.sel_dirty {
            self.sel_dirty = false;
            self.sel_set = self.selected.iter().cloned().collect();
            self.sel_label = SharedString::from(format!("{} selected", self.selected.len()));
        }
    }

    /// Advance the hover springs by the frame delta; true while one is
    /// still moving.
    fn step_springs(&mut self) -> bool {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map(|t| (now - t).as_secs_f32())
            .unwrap_or(1.0 / 60.0);
        self.last_frame = Some(now);
        let n = self.listing.entries().len();
        if let Some(h) = self.hovered {
            self.springs.entry(h).or_default();
        }
        let hovered = self.hovered;
        let mut live = false;
        self.springs.retain(|i, s| {
            if *i >= n {
                return false;
            }
            let target = if hovered == Some(*i) { 1.0 } else { 0.0 };
            s.to(target, dt);
            let settled = s.settled(target);
            live |= !settled;
            // A spring at rest off the card holds nothing.
            !(settled && target == 0.0)
        });
        live
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Toolbar {
        let mut bar = Toolbar::new("Library");
        if !self.listing.count().is_empty() {
            bar = bar.caption(self.listing.count().clone());
        }
        if !self.selected.is_empty() {
            return bar
                .trailing(
                    div()
                        .text_size(px(theme::TEXT_BODY))
                        .text_color(theme::FG_DIM)
                        .font_features(theme::tabular())
                        .child(self.sel_label.clone()),
                )
                .trailing(
                    widgets::button("sel-copy", "Copy", false)
                        .tooltip(tip("Copy", Some("Ctrl+C".into())))
                        .on_click(cx.listener(|this, _, _, cx| this.copy_selection(cx))),
                )
                .trailing(
                    widgets::push_button("sel-trash", "Move to Trash", ButtonStyle::Destructive)
                        .h(px(theme::TOOLBAR_CONTROL_H))
                        .px(px(12.))
                        .tooltip(tip("Move to Trash", Some("Delete".into())))
                        .on_click(cx.listener(|this, _, _, cx| this.trash_selection(cx))),
                )
                .trailing(
                    widgets::button("sel-done", "Done", false)
                        .tooltip(tip("Clear Selection", Some("Esc".into())))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.selected.clear();
                            this.sel_dirty = true;
                            cx.notify();
                        })),
                );
        }
        bar.trailing(
            widgets::icon_button(
                "shortcuts",
                Icon::Keyboard,
                self.help,
                theme::TOOLBAR_CONTROL_H,
            )
            .tooltip(tip("Keyboard Shortcuts", Some("?".into())))
            .on_click(cx.listener(|this, _, _, cx| {
                this.help = !this.help;
                cx.notify();
            })),
        )
        .trailing(
            widgets::icon_button("settings", Icon::Gear, false, theme::TOOLBAR_CONTROL_H)
                .tooltip(tip("Settings", Some("Ctrl+,".into())))
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Err(e) = crate::settings::open(cx) {
                        this.set_status(e, cx);
                    }
                })),
        )
        .trailing(
            widgets::icon_button("capture", Icon::Viewfinder, false, theme::TOOLBAR_CONTROL_H)
                .tooltip(tip(
                    "New Screenshot",
                    Some(self.cfg.capture_hotkey.clone().into()),
                ))
                .on_click(cx.listener(|this, _, _, cx| this.start_capture(cx))),
        )
    }

    /// A day's title row, `layout.x0` from the left.
    fn day_header(layout: &Layout, title: SharedString, top: f32) -> impl IntoElement {
        let grid_w = layout.cols as f32 * super::CARD_W + (layout.cols - 1) as f32 * super::GAP;
        div()
            .absolute()
            .left(px(layout.x0 + 2.0))
            .top(px(top))
            .w(px(grid_w - 2.0))
            .h(px(HEADER_H))
            .pb(px(HEADER_PAD_B))
            .flex()
            .items_end()
            .whitespace_nowrap()
            .overflow_hidden()
            .text_size(px(theme::TEXT_HEADLINE))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme::FG)
            .child(title)
    }

    fn empty_state(&self) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .pb(px(theme::TOOLBAR_H))
            .child(icons::icon(Icon::Viewfinder, theme::FG_FAINT, 40.0))
            .child(
                div()
                    .mt(px(4.))
                    .text_size(px(theme::TEXT_TITLE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::FG_DIM)
                    .child("No Captures"),
            )
            .child(
                div()
                    .text_size(px(theme::TEXT_BODY))
                    .text_color(theme::FG_DIM)
                    .child(self.empty_hint.clone()),
            )
    }
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_selection();
        // The cache fills from the background prefetch; a card whose
        // thumb has not landed draws without its image until then.
        // A set, not a nested scan, rebuilt only when entries changed.
        if self.entries_dirty {
            self.entries_dirty = false;
            let live_paths: std::collections::HashSet<&std::path::Path> = self
                .listing
                .entries()
                .iter()
                .map(|e| e.path.as_path())
                .collect();
            evict_thumbs(&mut self.thumb_cache, |p| live_paths.contains(p), cx);
        }
        if self.step_springs() {
            window.request_animation_frame();
        }

        // The grid spans the window's width under the toolbar. The
        // window's size is this frame's; the scroll container's bounds
        // are the last paint's, a resize behind on the first frame after
        // one, which laid out columns for the old width.
        let win = window.bounds().size;
        let width = f32::from(win.width);
        let viewport_h = f32::from(win.height) - theme::TOOLBAR_H;
        let key = (self.listing.generation(), width);
        if key != self.layout_key {
            self.layout = Layout::new(self.listing.sections(), width);
            self.layout_key = key;
        }

        // Virtualize: build only the rows that intersect the viewport.
        // ScrollHandle::offset is <= 0 (negative when scrolled down);
        // the positive scroll amount is -offset.y. A wheel motion moves
        // the offset toward target_offset after this render, so every
        // row between the two renders.
        let scroll_top = -f32::from(self.scroll.offset().y);
        let target_top = -f32::from(self.scroll.target_offset().y);
        let shown = self.layout.rows_in(
            scroll_top.min(target_top),
            scroll_top.max(target_top) + viewport_h,
        );

        let mut content = div().relative().w_full().h(px(self.layout.height));
        if let Some((first, last)) = shown {
            for r in first..=last {
                let top = self.layout.tops[r];
                match self.layout.rows[r] {
                    Row::Header(s) => {
                        let title = self.listing.sections()[s].title.clone();
                        content = content.child(Self::day_header(&self.layout, title, top));
                    }
                    Row::Cards { start, end } => {
                        for i in start..end {
                            let Some((x, y, ..)) = self.layout.card_rect(i) else {
                                continue;
                            };
                            let entry = &self.listing.entries()[i];
                            let amt = self.springs.get(&i).map_or(0.0, |s| s.value);
                            let selected = self.sel_set.contains(&entry.path);
                            let card = self.card(
                                i,
                                entry,
                                &self.listing.captions()[i],
                                (x, y),
                                amt,
                                selected,
                                cx,
                            );
                            content = content.child(card);
                        }
                    }
                }
            }
            // Bound the thumbnail cache to the keep window: rendered rows
            // plus THUMB_KEEP_ROWS of slack. Tiles outside it are released
            // and re-decoded on scroll-back. The range moves only when the
            // viewport crosses a row, so eviction and prefetch do not run
            // per frame.
            let keep = self.layout.entries_of(
                first.saturating_sub(THUMB_KEEP_ROWS),
                last + THUMB_KEEP_ROWS,
            );
            if keep != self.thumb_keep {
                self.thumb_keep = keep;
                if self.thumb_cache.len() > keep.1 - keep.0 {
                    let keep_paths: std::collections::HashSet<&std::path::Path> =
                        self.listing.entries()[keep.0..keep.1]
                            .iter()
                            .map(|e| e.path.as_path())
                            .collect();
                    evict_thumbs(&mut self.thumb_cache, |p| keep_paths.contains(p), cx);
                }
                self.prefetch_thumbs(keep, cx);
            }
        }

        let mut grid = widgets::scroll_y(div().id("grid"), &self.scroll)
            .flex_1()
            .min_h_0()
            // A hairline under the toolbar once content scrolls under it.
            .border_t_1()
            .border_color(if scroll_top > 0.5 {
                theme::HAIRLINE
            } else {
                theme::alpha(theme::HAIRLINE, 0.0)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    // A card's own mousedown set drag_start first; only
                    // empty grid space starts a rubber band.
                    if this.drag_start.is_none() {
                        let (x, y) = (ev.position.x.into(), ev.position.y.into());
                        this.band = Some((x, y, x, y));
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
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.finish_band(cx)),
            );
        // Until the store answers, the grid shows nothing: an empty
        // state then is a false "no captures" flash on every open.
        grid = if self.listing.listed().is_some() && self.listing.entries().is_empty() {
            grid.child(self.empty_state())
        } else {
            grid.child(content)
        };

        let toolbar = self.toolbar(cx);
        let mut root = div()
            .size_full()
            .font_family(theme::FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    // Released anywhere: the next press on empty grid
                    // space starts a rubber band.
                    this.drag_start = None;
                }),
            )
            .child(widgets::toolbar_frame(window, toolbar, true, grid));

        if let Some((x0, y0, x1, y1)) = self.band {
            root = root.child(
                div()
                    .absolute()
                    .left(px(x0.min(x1)))
                    .top(px(y0.min(y1)))
                    .w(px((x1 - x0).abs()))
                    .h(px((y1 - y0).abs()))
                    .bg(theme::alpha(theme::ACCENT, 0.12))
                    .border_1()
                    .border_color(theme::alpha(theme::ACCENT, 0.6))
                    .rounded(px(4.)),
            );
        }
        if let Some(preview) = &self.preview {
            root = root.child(self.preview_view(preview, window, cx));
        }
        if let Some(menu) = &self.menu {
            root = root.child(self.context_menu(menu, window, cx));
        }
        if self.help {
            root = root.child(widgets::shortcuts_sheet(help_rows(&self.cfg)).on_click(
                cx.listener(|this, _, _, cx| {
                    this.help = false;
                    cx.notify();
                }),
            ));
        }
        if let Some(status) = &self.status {
            root = root.child(widgets::status_pill(status, 10.0));
        }
        root.children(widgets::resize_edges(window, MIN_SIZE))
    }
}

/// The shortcuts sheet's rows: the global hotkeys as configured, then
/// the library's own keys.
pub(super) fn help_rows(cfg: &iris_lib::config::Config) -> Vec<(&'static str, SharedString)> {
    vec![
        ("New screenshot", cfg.capture_hotkey.clone().into()),
        ("Record window", cfg.record_hotkey.clone().into()),
        ("Open", "Return or click".into()),
        ("Quick Look", "Space".into()),
        ("Move selection", "Arrow keys".into()),
        ("Extend selection", "Shift+Arrow or Shift+click".into()),
        ("Add to selection", "Ctrl+click".into()),
        ("Select all", "Ctrl+A".into()),
        ("Copy", "Ctrl+C".into()),
        ("Move to Trash", "Delete".into()),
        ("Clear selection", "Esc".into()),
        ("Settings", "Ctrl+,".into()),
        ("This sheet", "?".into()),
    ]
}
