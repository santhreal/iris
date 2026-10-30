use std::path::PathBuf;

use gpui::*;
use iris_lib::library::{self, CaptureEntry};

use crate::{editor, pipeline};

use super::{evict_thumbs, layout, Library, REFRESH, THUMB_KEEP_ROWS};

/// A thumbnail decode request: the keep window, and the entries the
/// grid draws now, which decode and land first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct ThumbWant {
    pub(super) keep: (usize, usize),
    pub(super) drawn: (usize, usize),
}

impl ThumbWant {
    /// The entry indexes to decode, in delivery order: the drawn
    /// entries inside the keep window, then the rest of the window.
    pub(super) fn passes(self) -> [Vec<usize>; 2] {
        let (lo, hi) = self.keep;
        let drawn = self.drawn.0.max(lo)..self.drawn.1.min(hi);
        [
            drawn.clone().collect(),
            (lo..hi).filter(|i| !drawn.contains(i)).collect(),
        ]
    }
}

/// Decode the thumbnails of `entries`, one background task each: the
/// executor is a pool, so the PNG decodes (and any rebuild of a stale
/// thumbnail) run across cores instead of serially on one task. A
/// thumbnail that cannot be read is left out.
pub(super) async fn decode_thumbs(
    ex: BackgroundExecutor,
    entries: Vec<CaptureEntry>,
) -> Vec<(PathBuf, std::sync::Arc<RenderImage>)> {
    let tasks: Vec<_> = entries
        .into_iter()
        .map(|e| {
            ex.spawn(async move {
                library::thumbnail(&e)
                    .ok()
                    .map(|img| (e.path, crate::widgets::render_image_from_rgba_owned(img)))
            })
        })
        .collect();
    let mut loaded = Vec::with_capacity(tasks.len());
    for task in tasks {
        if let Some(pair) = task.await {
            loaded.push(pair);
        }
    }
    loaded
}

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
    /// Lay out the grid at `width` when the listing or the width
    /// changed since the last layout.
    pub(super) fn relayout(&mut self, width: f32) {
        let key = (self.listing.generation(), width);
        if key != self.layout_key {
            self.layout = layout::Layout::new(self.listing.sections(), width);
            self.layout_key = key;
        }
    }

    /// The entries of grid rows `first..=last` and of THUMB_KEEP_ROWS
    /// rows either side: the thumbnails the cache keeps.
    pub(super) fn keep_of(&self, (first, last): (usize, usize)) -> (usize, usize) {
        self.layout.entries_of(
            first.saturating_sub(THUMB_KEEP_ROWS),
            last + THUMB_KEEP_ROWS,
        )
    }

    /// Keep the thumbnails around grid rows `rows`, the rows drawn:
    /// release the tiles outside the keep window, and decode the
    /// missing ones, the drawn rows' first.
    pub(super) fn keep_thumbs(&mut self, rows: (usize, usize), cx: &mut Context<Self>) {
        let keep = self.keep_of(rows);
        self.thumb_keep = keep;
        if self.thumb_cache.len() > keep.1 - keep.0 {
            let keep_paths: std::collections::HashSet<&std::path::Path> = self.listing.entries()
                [keep.0..keep.1]
                .iter()
                .map(|e| e.path.as_path())
                .collect();
            evict_thumbs(&mut self.thumb_cache, |p| keep_paths.contains(p), cx);
        }
        let drawn = self.layout.entries_of(rows.0, rows.1);
        self.prefetch_thumbs(ThumbWant { keep, drawn }, cx);
    }

    /// Read thumbnails off the main thread. The drawn entries land in
    /// one delivery, then the rest of the keep window in a second: a
    /// first paint that blocks on N disk reads stutters, and the drawn
    /// cards wait on no slack row. A pass already in flight parks the
    /// newest request in prefetch_want and the running task picks it
    /// up, after the drawn entries of the request it runs, so a fast
    /// scroll never stacks decode waves.
    fn prefetch_thumbs(&mut self, want: ThumbWant, cx: &mut Context<Self>) {
        self.prefetch_want = Some(want);
        if self.prefetch_in_flight {
            return;
        }
        self.prefetch_in_flight = true;
        cx.spawn(async move |this, cx| {
            'want: while let Some(want) = this
                .update(cx, |this, _| this.prefetch_want.take())
                .unwrap_or(None)
            {
                for (pass, indexes) in want.passes().into_iter().enumerate() {
                    if pass > 0
                        && this
                            .update(cx, |this, _| this.prefetch_want.is_some())
                            .unwrap_or(true)
                    {
                        continue 'want;
                    }
                    let missing: Vec<CaptureEntry> = match this.update(cx, |this, _| {
                        let shown = this.listing.entries();
                        indexes
                            .iter()
                            .filter_map(|&i| shown.get(i))
                            .filter(|e| !this.thumb_cache.contains_key(&e.path))
                            // Owned copies: the Rc is not Send, and each
                            // entry moves into a background task.
                            .map(|e| CaptureEntry::clone(e))
                            .collect()
                    }) {
                        Ok(m) => m,
                        Err(_) => break 'want,
                    };
                    if missing.is_empty() {
                        continue;
                    }
                    let loaded = decode_thumbs(cx.background_executor().clone(), missing).await;
                    if this
                        .update(cx, |this, cx| {
                            this.thumb_cache.extend(loaded);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break 'want;
                    }
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
        // A capture this process just saved brings its thumbnail: its
        // card shows the capture on its first frame, with no blank
        // frame and no decode of the file just written.
        let shown = self.listing.entries();
        let cache = &self.thumb_cache;
        let fresh = library::take_fresh_thumbs(|path, revision| {
            !cache.contains_key(path)
                && shown
                    .iter()
                    .any(|e| e.path == path && e.revision() == revision)
        });
        for (path, img) in fresh {
            self.thumb_cache
                .insert(path, crate::widgets::render_image_from_rgba_owned(img));
        }
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
        // Decode the thumbnails the new listing draws now, before the
        // next frame: an opening window's first frame shows the ones
        // that land by then, and a capture re-saved on screen needs its
        // new one. The layout is the last render's width and height.
        self.relayout(self.layout_key.1);
        let top = -f32::from(self.scroll.offset().y);
        if let Some(rows) = self.layout.rows_in(top, top + self.viewport_h) {
            self.keep_thumbs(rows, cx);
        }
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

    /// Show `text` in the status pill for `widgets::STATUS_FOR`; a newer message
    /// replaces it and gets its own time.
    pub(super) fn set_status(&mut self, text: String, cx: &mut Context<Self>) {
        self.status = Some(text.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::widgets::STATUS_FOR)
                .await;
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

    /// Entry `index`'s thumbnail frame in screen coordinates, the rect
    /// the editor morphs out of; None before the grid is laid out.
    pub(super) fn card_screen_rect(
        &self,
        index: usize,
        window: &Window,
    ) -> Option<(f32, f32, f32, f32)> {
        let (x, y, ..) = self.layout.card_rect(index)?;
        let entry = self.listing.entries().get(index)?;
        let (dx, dy, w, h) = super::card::thumb_frame(entry.width, entry.height);
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
            wx + f32::from(view.origin.x) + x + dx,
            wy + f32::from(view.origin.y) + y + dy - scroll_top,
            w,
            h,
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

    /// New Screenshot: draw the window empty and close it, then start the
    /// capture once it is off the screen, so the frozen frame never
    /// contains the library.
    pub(super) fn start_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.leaving {
            return;
        }
        self.leaving = true;
        cx.notify();
        crate::widgets::close_then_run(window, cx, crate::daemon::Command::Capture);
    }
}
