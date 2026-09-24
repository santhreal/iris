use std::path::PathBuf;

use gpui::*;
use iris_lib::library::{self, CaptureEntry};

use crate::{editor, pipeline, theme};

use super::{Library, CARD_W, GAP, LABEL_GAP, LABEL_PAD, REFRESH, THUMB_H};

/// Name columns that fit beside `dims` on a card's label row. The font
/// is monospace, so a width is a column count.
pub(super) fn name_cols(dims: &str) -> usize {
    let row = (CARD_W - 2.0 * LABEL_PAD - LABEL_GAP) / theme::SMALL_ADVANCE;
    (row.floor() as usize)
        .saturating_sub(dims.chars().count())
        .max(1)
}

/// `name` in at most `cols` characters. A longer name keeps its head and
/// tail around one ellipsis, as file managers shorten names, so the end
/// of a timestamp and a collision suffix (`-2`) stay readable.
pub(super) fn fit_name(name: &str, cols: usize) -> String {
    let n = name.chars().count();
    if n <= cols {
        return name.to_owned();
    }
    let keep = cols.saturating_sub(1);
    let head = keep.div_ceil(2);
    let mut out: String = name.chars().take(head).collect();
    out.push('\u{2026}');
    out.extend(name.chars().skip(n - (keep - head)));
    out
}

impl Library {
    /// The card's display strings: the file stem fitted beside the
    /// dimensions, and the dimensions. Every entry is a PNG, so the
    /// extension is dropped. GPUI's text_ellipsis only fires on wrapped
    /// text (nowrap clips), so the fitting happens here, once per entry
    /// rather than per frame.
    pub(super) fn entry_name(e: &CaptureEntry) -> (SharedString, SharedString) {
        let dims = format!("{}×{}", e.width, e.height);
        let stem = e
            .path
            .file_stem()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        let name = fit_name(&stem, name_cols(&dims));
        (SharedString::from(name), SharedString::from(dims))
    }

    /// Read thumbnails off the main thread and fill the cache in one
    /// delivery: a first paint that blocks on N disk reads stutters.
    /// `range` is the entry-index window to decode (the keep window
    /// render computed); a pass already in flight parks the newest
    /// range in prefetch_want and the running task picks it up, so a
    /// fast scroll never stacks decode waves.
    pub(super) fn prefetch_thumbs(&mut self, range: (usize, usize), cx: &mut Context<Self>) {
        if self.prefetch_in_flight {
            self.prefetch_want = Some(range);
            return;
        }
        self.prefetch_in_flight = true;
        self.prefetch_want = Some(range);
        cx.spawn(async move |this, cx| {
            while let Some(range) = this
                .update(cx, |this, _| this.prefetch_want.take())
                .unwrap_or(None)
            {
                let missing: Vec<CaptureEntry> = match this.update(cx, |this, _| {
                    let shown = this.listing.entries();
                    let (lo, hi) = (range.0.min(shown.len()), range.1.min(shown.len()));
                    shown[lo..hi]
                        .iter()
                        .filter(|e| !this.thumb_cache.contains_key(&e.path))
                        // Owned copies: the Rc is not Send, and each
                        // entry moves into a background task.
                        .map(|e| CaptureEntry::clone(e))
                        .collect()
                }) {
                    Ok(m) => m,
                    Err(_) => break,
                };
                // One background task per thumb: the executor is a
                // pool, so the window's PNG decodes (and any rebuild of
                // a stale thumbnail) run across cores instead of
                // serially on one task.
                let tasks: Vec<_> = missing
                    .into_iter()
                    .map(|e| {
                        cx.background_executor().spawn(async move {
                            library::thumbnail(&e).ok().map(|img| {
                                (e.path, crate::widgets::render_image_from_rgba_owned(img))
                            })
                        })
                    })
                    .collect();
                let mut loaded = Vec::with_capacity(tasks.len());
                for task in tasks {
                    if let Some(pair) = task.await {
                        loaded.push(pair);
                    }
                }
                if this
                    .update(cx, |this, cx| {
                        this.thumb_cache.extend(loaded);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
            let _ = this.update(cx, |this, _| {
                this.prefetch_in_flight = false;
            });
        })
        .detach();
    }

    /// One background list + apply: the open path and the refresh
    /// poll share it so neither stats the store on the UI thread.
    /// A pass already in flight skips the poll: on a slow store the
    /// 1.5s timer would otherwise stack overlapping scans.
    pub(super) fn refresh_now(&mut self, cx: &mut Context<Self>) {
        if self.refresh_in_flight {
            return;
        }
        self.refresh_in_flight = true;
        cx.spawn(async move |this, cx| {
            let fresh = cx
                .background_executor()
                .spawn(async move { library::list() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.refresh_in_flight = false;
                this.show(fresh, cx);
            });
        })
        .detach();
    }

    /// Show a listing from the store: a refresh, or the listing read
    /// back after a delete.
    pub(super) fn show(&mut self, fresh: Vec<CaptureEntry>, cx: &mut Context<Self>) {
        let Some(stale) = self.listing.set(fresh, &mut self.selected) else {
            return;
        };
        // A capture changed under its path: drop its thumbnail so the
        // prefetch decodes the new one.
        super::evict_thumbs(&mut self.thumb_cache, |p| !stale.iter().any(|s| s == p), cx);
        self.entries_dirty = true;
        self.sel_dirty = true;
        // Decode the window the grid keeps: a capture re-saved on
        // screen needs its new thumbnail now, and the keep window only
        // prefetches when it moves. Before the first render with cards
        // there is no window yet; decode the top of the list.
        let keep = self.thumb_keep;
        self.prefetch_thumbs(if keep.0 < keep.1 { keep } else { (0, 48) }, cx);
        cx.notify();
    }

    /// Poll the store so captures from any process surface here. The
    /// directory scan runs off the main thread.
    pub(super) fn arm_refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(REFRESH).await;
            let alive = this.update(cx, |this, cx| {
                this.refresh_now(cx);
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }

    pub(super) fn toggle_select(&mut self, path: PathBuf) {
        if let Some(i) = self.selected.iter().position(|p| *p == path) {
            self.selected.remove(i);
        } else {
            self.selected.push(path);
        }
        self.sel_dirty = true;
    }

    pub(super) fn click_card(
        &mut self,
        index: usize,
        ev: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.listing.entries().get(index) else {
            return;
        };
        let mods = ev.modifiers();
        let ctrl = mods.control || mods.platform;
        if ctrl {
            self.toggle_select(entry.path.clone());
            self.anchor = Some(index);
            cx.notify();
        } else if mods.shift {
            // The anchor is an index into entries; a refresh that
            // removed cards can leave it out of bounds, and a raw
            // slice range would panic the daemon.
            let shown = self.listing.entries();
            if let Some(anchor) = self.anchor.filter(|a| *a < shown.len()) {
                let (lo, hi) = (anchor.min(index), anchor.max(index));
                for entry in &shown[lo..=hi] {
                    if !self.selected.contains(&entry.path) {
                        self.selected.push(entry.path.clone());
                    }
                }
                self.sel_dirty = true;
                cx.notify();
            }
        } else if let Err(e) = editor::open(
            cx,
            &entry.path,
            Some(Self::card_morph_rect(ev, window)),
            None,
        ) {
            self.status = Some(e);
            cx.notify();
        }
    }

    /// The rect the editor morphs out of when a card opens: the card's
    /// thumb geometry centered on the click, in screen coordinates. The
    /// true card origin is layout state; centering on the click is within
    /// half a card of it, well inside the spring's travel.
    pub(super) fn card_morph_rect(ev: &ClickEvent, window: &Window) -> (f32, f32, f32, f32) {
        let (wx, wy) = match window.window_bounds() {
            WindowBounds::Windowed(b) => (f32::from(b.origin.x), f32::from(b.origin.y)),
            _ => (0.0, 0.0),
        };
        let (mx, my): (f32, f32) = (ev.position().x.into(), ev.position().y.into());
        (
            wx + mx - CARD_W / 2.0,
            wy + my - THUMB_H / 2.0,
            CARD_W,
            THUMB_H,
        )
    }

    pub(super) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let paths = self.selected.clone();
        // Single-image copy decodes the PNG; keep it off the UI thread.
        let task = cx.background_executor().spawn(async move {
            if paths.len() == 1 {
                pipeline::copy_image_file(&paths[0])
            } else {
                pipeline::copy_files(&paths)
            }
        });
        let count = self.selected.len();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(()) => format!("{count} copied"),
                    Err(e) => e,
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn delete_selection(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = std::mem::take(&mut self.selected);
        self.sel_dirty = true;
        // delete_many + the follow-up list() stat every file; keep the
        // disk work off the UI thread.
        let task = cx.background_executor().spawn(async move {
            let errors = library::delete_many(&paths);
            (errors, library::list())
        });
        cx.spawn(async move |this, cx| {
            let (errors, entries) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.status = (errors > 0).then(|| format!("{errors} delete(s) failed"));
                this.show(entries, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Close the rubber band: a sub-4px drag is a click on empty
    /// space and clears the selection; anything larger selects every
    /// card whose rect intersects the band.
    pub(super) fn finish_band(
        &mut self,
        _ev: &MouseUpEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some((x0, y0, x1, y1)) = self.band.take() else {
            return;
        };
        let (dx, dy) = (x1 - x0, y1 - y0);
        if dx * dx + dy * dy < 16.0 {
            if !self.selected.is_empty() {
                self.selected.clear();
                self.sel_dirty = true;
                cx.notify();
            }
            return;
        }
        let (bx0, bx1) = (x0.min(x1), x0.max(x1));
        let (by0, by1) = (y0.min(y1), y0.max(y1));
        // Card rects live in document space: grid top is the 56px
        // frame toolbar, scroll shifts rows up. ScrollHandle::offset
        // is <= 0 (negative when scrolled down), so the positive
        // scroll amount is -offset.y and a card's window-space top is
        // its document top minus that amount.
        let width: f32 = window.bounds().size.width.into();
        let scroll_top: f32 = -f32::from(self.scroll.offset().y);
        let cols = ((width - GAP) / (CARD_W + GAP)).floor().max(1.0) as usize;
        let card_h = THUMB_H + 8.0 + 18.0;
        self.selected.clear();
        self.sel_dirty = true;
        for (i, e) in self.listing.entries().iter().enumerate() {
            let (r, c) = (i / cols, i % cols);
            let cx0 = GAP + c as f32 * (CARD_W + GAP);
            let cy0 = 56.0 + GAP + r as f32 * (card_h + GAP) - scroll_top;
            if cx0 < bx1 && cx0 + CARD_W > bx0 && cy0 < by1 && cy0 + card_h > by0 {
                self.selected.push(e.path.clone());
            }
        }
        self.anchor = self
            .listing
            .entries()
            .iter()
            .position(|e| self.selected.contains(&e.path));
        cx.notify();
    }

    pub(super) fn start_capture(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = crate::daemon::dispatch(cx, &crate::daemon::Command::Capture) {
            self.status = Some(e);
            cx.notify();
        }
    }
}
