use std::path::PathBuf;

use gpui::*;
use iris_lib::library::{self, CaptureEntry};

use crate::{editor, pipeline};

use super::{layout, Library, CARD_W, REFRESH, THUMB_H};

/// How long a status message stays up.
const STATUS_FOR: std::time::Duration = std::time::Duration::from_secs(4);

/// The status line after a trash of `asked` captures that left `errors`
/// behind: None when every capture moved, else the count that stayed
/// and the first reason.
pub(super) fn trash_status(asked: usize, errors: &[String]) -> Option<String> {
    let first = errors.first()?;
    Some(match errors.len() {
        1 if asked == 1 => first.clone(),
        n => format!("{n} of {asked} captures stayed. {first}"),
    })
}

impl Library {
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

    /// One background list + apply: the open path, a folder change, a
    /// store write in this process, and the poll of a window with no
    /// folder watch share it, so none stats the store on the UI thread.
    /// [`ListPass`](super::listing::ListPass) sets which requests start
    /// a pass.
    pub(super) fn refresh(&mut self, after_change: bool, cx: &mut Context<Self>) {
        if !self.list_pass.begin(after_change) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let fresh = cx
                .background_executor()
                .spawn(async move { library::list() })
                .await;
            let _ = this.update(cx, |this, cx| {
                let again = this.list_pass.land();
                this.show(fresh, cx);
                if again {
                    this.refresh(false, cx);
                }
            });
        })
        .detach();
    }

    /// Show a listing from the store: a refresh, or the listing read
    /// back after a trash.
    pub(super) fn show(&mut self, fresh: Vec<CaptureEntry>, cx: &mut Context<Self>) {
        let Some(stale) = self.listing.set(fresh, &mut self.selected, layout::today()) else {
            return;
        };
        // A capture changed under its path: drop its thumbnail so the
        // prefetch decodes the new one.
        super::evict_thumbs(&mut self.thumb_cache, |p| !stale.iter().any(|s| s == p), cx);
        self.entries_dirty = true;
        self.sel_dirty = true;
        // Indexes into the old listing: the cursor stays where it was,
        // on the card that took that place.
        let n = self.listing.entries().len();
        self.cursor = self
            .cursor
            .and_then(|c| n.checked_sub(1).map(|last| c.min(last)));
        self.anchor = self.anchor.filter(|a| *a < n);
        if self
            .preview
            .as_ref()
            .is_some_and(|p| self.listing.entries().get(p.index).map(|e| &e.path) != Some(&p.path))
        {
            self.close_preview(cx);
        }
        // Decode the window the grid keeps: a capture re-saved on
        // screen needs its new thumbnail now, and the keep window only
        // prefetches when it moves. Before the first render with cards
        // there is no window yet; decode the top of the list.
        let keep = self.thumb_keep;
        self.prefetch_thumbs(if keep.0 < keep.1 { keep } else { (0, 48) }, cx);
        self.follow_folders(cx);
        cx.notify();
    }

    /// Read the store every `REFRESH`, for the life of the window: the
    /// fallback of a window whose capture folders the OS refused to
    /// watch. The directory scan runs off the main thread.
    pub(super) fn arm_refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(REFRESH).await;
            let alive = this.update(cx, |this, cx| {
                this.refresh(false, cx);
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }

    /// Show `text` in the status pill for STATUS_FOR; a newer message
    /// replaces it and gets its own time.
    pub(super) fn set_status(&mut self, text: String, cx: &mut Context<Self>) {
        self.status = Some(text.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_FOR).await;
            let _ = this.update(cx, |this, cx| {
                if this.status.as_ref() == Some(&text) {
                    this.status = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
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
            self.cursor = Some(index);
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
                self.cursor = Some(index);
                self.sel_dirty = true;
                cx.notify();
            }
        } else {
            self.cursor = Some(index);
            self.anchor = Some(index);
            self.open_entry(index, window, cx);
        }
    }

    /// Open entry `index` in the editor, morphing out of its card.
    pub(super) fn open_entry(&mut self, index: usize, window: &Window, cx: &mut Context<Self>) {
        let Some(path) = self.listing.entries().get(index).map(|e| e.path.clone()) else {
            return;
        };
        let from = self.card_screen_rect(index, window);
        if let Err(e) = editor::open(cx, &path, from, None) {
            self.set_status(e, cx);
        }
    }

    /// Entry `index`'s thumbnail rect in screen coordinates, the rect the
    /// editor morphs out of; None before the grid is laid out.
    pub(super) fn card_screen_rect(
        &self,
        index: usize,
        window: &Window,
    ) -> Option<(f32, f32, f32, f32)> {
        let (x, y, ..) = self.layout.card_rect(index)?;
        let view = self.scroll.bounds();
        if f32::from(view.size.height) <= 0.0 {
            return None;
        }
        let (wx, wy) = match window.window_bounds() {
            WindowBounds::Windowed(b) => (f32::from(b.origin.x), f32::from(b.origin.y)),
            _ => (0.0, 0.0),
        };
        let scroll_top = -f32::from(self.scroll.offset().y);
        Some((
            wx + f32::from(view.origin.x) + x,
            wy + f32::from(view.origin.y) + y - scroll_top,
            CARD_W,
            THUMB_H,
        ))
    }

    pub(super) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        self.copy_paths(self.selected.clone(), cx);
    }

    /// Copy `paths` to the clipboard: one capture as its image, several
    /// as files. The status line reports the result.
    pub(super) fn copy_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let count = paths.len();
        if count == 0 {
            return;
        }
        // Single-image copy decodes the PNG; keep it off the UI thread.
        let task = cx.background_executor().spawn(async move {
            if count == 1 {
                pipeline::copy_image_file(&paths[0])
            } else {
                pipeline::copy_files(&paths)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let text = match result {
                    Ok(()) if count == 1 => "Copied".to_owned(),
                    Ok(()) => format!("Copied {count} captures"),
                    Err(e) => e,
                };
                this.set_status(text, cx);
            });
        })
        .detach();
    }

    pub(super) fn trash_selection(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = std::mem::take(&mut self.selected);
        self.sel_dirty = true;
        self.trash_paths(paths, cx);
    }

    /// Move `paths` to the system trash and show the listing read back.
    /// A capture that stays keeps its card, and the status line states
    /// why.
    pub(super) fn trash_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        // The trash move + the follow-up list() stat every file; keep the
        // disk work off the UI thread.
        let asked = paths.len();
        let task = cx.background_executor().spawn(async move {
            let errors = library::trash_many(&paths);
            (errors, library::list())
        });
        cx.spawn(async move |this, cx| {
            let (errors, entries) = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(text) = trash_status(asked, &errors) {
                    this.set_status(text, cx);
                }
                this.show(entries, cx);
            });
        })
        .detach();
    }

    /// Close the rubber band: a sub-4px drag is a click on empty
    /// space and clears the selection; anything larger selects every
    /// card whose rect intersects the band.
    pub(super) fn finish_band(&mut self, cx: &mut Context<Self>) {
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
        // The band is in window space; cards are in document space,
        // which starts at the scroll viewport's origin and moves up by
        // the scroll amount (-offset.y: the offset is <= 0).
        let view = self.scroll.bounds().origin;
        let (ox, oy) = (
            f32::from(view.x),
            f32::from(view.y) + f32::from(self.scroll.offset().y),
        );
        let hit = self.layout.cards_in(
            x0.min(x1) - ox,
            y0.min(y1) - oy,
            x0.max(x1) - ox,
            y0.max(y1) - oy,
        );
        let entries = self.listing.entries();
        self.selected = hit.iter().map(|&i| entries[i].path.clone()).collect();
        self.anchor = hit.first().copied();
        self.cursor = hit.last().copied();
        self.sel_dirty = true;
        cx.notify();
    }

    pub(super) fn start_capture(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = crate::daemon::dispatch(cx, &crate::daemon::Command::Capture) {
            self.set_status(e, cx);
        }
    }
}
