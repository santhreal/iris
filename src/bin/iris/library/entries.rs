use std::path::PathBuf;
use std::rc::Rc;

use gpui::*;
use iris_lib::library::{self, CaptureEntry};

use crate::{editor, pipeline};

use super::{Library, CARD_W, GAP, REFRESH, THUMB_H};

impl Library {
    /// The card's display strings: the file name truncated to ~19
    /// chars (GPUI's text_ellipsis only fires on wrapped text; nowrap
    /// clips, so the truncation happens here) and the dimensions.
    /// Computed once per entry, not per frame.
    pub(super) fn entry_name(e: &CaptureEntry) -> (SharedString, SharedString) {
        let name = e
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = if name.chars().count() > 19 {
            let cut: String = name.chars().take(18).collect();
            SharedString::from(format!("{cut}\u{2026}"))
        } else {
            SharedString::from(name)
        };
        (
            name,
            SharedString::from(format!("{}×{}", e.width, e.height)),
        )
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
                let missing: Vec<(std::path::PathBuf, std::path::PathBuf)> = match this
                    .update(cx, |this, _| {
                        let (lo, hi) = (range.0.min(this.entries.len()), range.1.min(this.entries.len()));
                        this.entries[lo..hi]
                            .iter()
                            .filter(|e| !this.thumb_cache.contains_key(&e.path))
                            .map(|e| (e.path.clone(), e.thumb.clone()))
                            .collect()
                    }) {
                    Ok(m) => m,
                    Err(_) => break,
                };
                // One background task per thumb: the executor is a
                // pool, so the window's PNG decodes run across cores
                // instead of serially on one task.
                let tasks: Vec<_> = missing
                    .into_iter()
                    .map(|(p, t)| {
                        cx.background_executor().spawn(async move {
                            std::fs::read(&t)
                                .ok()
                                .and_then(|b| crate::widgets::render_image_from_png(&b))
                                .map(|img| (p, img))
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
                // Path AND timestamp: an editor re-save keeps the
                // path but bumps created_ms, and a path-only diff
                // would keep showing the stale thumbnail.
                let changed = fresh.len() != this.entries.len()
                    || !fresh
                        .iter()
                        .map(|e| (&e.path, e.created_ms))
                        .eq(this.entries.iter().map(|e| (&e.path, e.created_ms)));
                if changed {
                    // Drop cached thumbs whose entry changed under
                    // the same path so prefetch re-decodes them.
                    let stale: std::collections::HashSet<&std::path::Path> = fresh
                        .iter()
                        .filter(|e| {
                            this.entries
                                .iter()
                                .any(|o| o.path == e.path && o.created_ms != e.created_ms)
                        })
                        .map(|e| e.path.as_path())
                        .collect();
                    this.thumb_cache.retain(|p, _| !stale.contains(p.as_path()));
                    this.entries = fresh.into_iter().map(Rc::new).collect();
                    this.entries_dirty = true;
                    this.entry_names = this.entries.iter().map(|e| Self::entry_name(e)).collect();
                    this.count_label =
                        SharedString::from(format!("{} captures", this.entries.len()));
                    this.selected
                        .retain(|p| this.entries.iter().any(|e| &e.path == p));
                    // Decode the top of the list; render's keep-window
                    // prefetch covers whatever the viewport shows.
                    this.prefetch_thumbs((0, 48), cx);
                    cx.notify();
                }
            });
        })
        .detach();
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
        let Some(entry) = self.entries.get(index) else {
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
            if let Some(anchor) = self.anchor.filter(|a| *a < self.entries.len()) {
                let (lo, hi) = (anchor.min(index), anchor.max(index));
                for entry in &self.entries[lo..=hi] {
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

    /// Open the containing directory of a capture with xdg-open.
    pub(super) fn open_containing_folder(path: &std::path::Path) {
        let parent = path.parent().unwrap_or(path).to_path_buf();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("xdg-open").arg(&parent).spawn();
        });
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
                this.entries = entries.into_iter().map(Rc::new).collect();
                this.entries_dirty = true;
                this.entry_names = this.entries.iter().map(|e| Self::entry_name(e)).collect();
                this.status = (errors > 0).then(|| format!("{errors} delete(s) failed"));
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
        for (i, e) in self.entries.iter().enumerate() {
            let (r, c) = (i / cols, i % cols);
            let cx0 = GAP + c as f32 * (CARD_W + GAP);
            let cy0 = 56.0 + GAP + r as f32 * (card_h + GAP) - scroll_top;
            if cx0 < bx1 && cx0 + CARD_W > bx0 && cy0 < by1 && cy0 + card_h > by0 {
                self.selected.push(e.path.clone());
            }
        }
        self.anchor = self
            .entries
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
