//! The captures a library window shows, what is derived from them, and
//! when the store is read again.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gpui::SharedString;
use iris_lib::library::CaptureEntry;

use super::layout::{self, Day, Section};

/// A card's caption: the local time it was taken ("14:02"), its pixel
/// size ("1920 × 1080"), and its file name for the caption's tooltip.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Caption {
    pub time: SharedString,
    pub dims: SharedString,
    pub name: SharedString,
}

/// Order `entries` newest first by `created_ms`, the order a listing
/// shows; captures with equal times keep their order.
fn newest_first(entries: &mut [CaptureEntry]) {
    if !entries.is_sorted_by(|a, b| a.created_ms >= b.created_ms) {
        entries.sort_by_key(|e| std::cmp::Reverse(e.created_ms));
    }
}

/// Order `entries` as a listing shows them, and return the entries a
/// grid of them draws at the top of a viewport `width` wide and
/// `height` high, as a half-open range: the cards a window opening on
/// them shows first.
pub(super) fn first_screen(
    entries: &mut [CaptureEntry],
    today: Day,
    width: f32,
    height: f32,
) -> (usize, usize) {
    newest_first(entries);
    let days: Vec<Day> = entries
        .iter()
        .map(|e| layout::day_of(e.created_ms))
        .collect();
    let grid = layout::Layout::new(&layout::sections(&days, today), width);
    grid.rows_in(0.0, height)
        .map_or((0, 0), |(first, last)| grid.entries_of(first, last))
}

/// The shown captures, newest first, with each card's caption, the day
/// sections, and the toolbar count. The fields are private: a listing is
/// shown only through [`Listing::set`], so no caption, section, count, or
/// selection outlives the listing it describes.
#[derive(Default)]
pub(super) struct Listing {
    /// Cards share their entry: render closures capture an Rc bump
    /// instead of cloning the path strings per card per frame.
    entries: Vec<Rc<CaptureEntry>>,
    /// Each entry's caption, parallel to `entries`: built once per
    /// listing, not per card per frame.
    captions: Vec<Caption>,
    /// Each entry's local day, parallel to `entries`.
    days: Vec<Day>,
    sections: Vec<Section>,
    /// The day the section titles were written against.
    today: Day,
    count: SharedString,
    /// When the first listing landed. None until the store answers:
    /// until then the window shows no count and no empty state.
    listed: Option<Instant>,
    /// Bumped whenever entries or section titles change, so a layout
    /// built from an older listing is rebuilt.
    generation: u64,
}

impl Listing {
    pub(super) fn entries(&self) -> &[Rc<CaptureEntry>] {
        &self.entries
    }

    pub(super) fn captions(&self) -> &[Caption] {
        &self.captions
    }

    pub(super) fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The toolbar count; empty until the first listing lands, and for
    /// an empty listing.
    pub(super) fn count(&self) -> &SharedString {
        &self.count
    }

    pub(super) fn listed(&self) -> Option<Instant> {
        self.listed
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// The folders that hold the shown captures, sorted, each once.
    pub(super) fn folders(&self) -> Vec<PathBuf> {
        let dirs: HashSet<&Path> = self
            .entries
            .iter()
            .filter_map(|e| e.path.parent())
            .collect();
        let mut dirs: Vec<PathBuf> = dirs.into_iter().map(Path::to_path_buf).collect();
        dirs.sort_unstable();
        dirs
    }

    /// Show `fresh` and drop from `selected` each path it does not
    /// list. Returns the paths whose capture changed in place (an
    /// editor save keeps the path and `created_ms` and moves
    /// `edited_ms`), whose thumbnails are stale, or None when `fresh` is
    /// what is shown.
    /// The listing is ordered newest first by `created_ms`, so each day
    /// is one section; captures with equal times keep the store's order.
    pub(super) fn set(
        &mut self,
        mut fresh: Vec<CaptureEntry>,
        selected: &mut Vec<PathBuf>,
        today: Day,
    ) -> Option<Vec<PathBuf>> {
        newest_first(&mut fresh);
        // The shown listing, unchanged, behind `added` new captures: the
        // common change, a capture landing. Only the new captions are
        // built; a library of thousands pays for the few that landed.
        let added = fresh.len().checked_sub(self.entries.len());
        if let Some(added) = added.filter(|_| self.listed.is_some()) {
            let kept = fresh[added..]
                .iter()
                .zip(&self.entries)
                .all(|(f, e)| f.path == e.path && f.revision() == e.revision());
            if kept && added == 0 {
                self.relabel(today);
                return None;
            }
            if kept {
                fresh.truncate(added);
                let (days, captions): (Vec<Day>, Vec<Caption>) = fresh
                    .iter()
                    .map(|e| {
                        let (day, time) = layout::day_and_time(e.created_ms);
                        (day, caption(e, time))
                    })
                    .unzip();
                self.days.splice(0..0, days);
                self.captions.splice(0..0, captions);
                self.entries.splice(0..0, fresh.into_iter().map(Rc::new));
                self.derive(today);
                return Some(Vec::new());
            }
        }
        let shown: HashMap<&Path, (i64, i64)> = self
            .entries
            .iter()
            .map(|e| (e.path.as_path(), e.revision()))
            .collect();
        let stale = fresh
            .iter()
            .filter(|e| {
                shown
                    .get(e.path.as_path())
                    .is_some_and(|&t| t != e.revision())
            })
            .map(|e| e.path.clone())
            .collect();
        let paths: HashSet<&Path> = fresh.iter().map(|e| e.path.as_path()).collect();
        selected.retain(|p| paths.contains(p.as_path()));
        (self.days, self.captions) = fresh
            .iter()
            .map(|e| {
                let (day, time) = layout::day_and_time(e.created_ms);
                (day, caption(e, time))
            })
            .unzip();
        self.entries = fresh.into_iter().map(Rc::new).collect();
        self.derive(today);
        Some(stale)
    }

    /// Rebuild what follows from the entries and days: the sections
    /// against `today`, and the count.
    fn derive(&mut self, today: Day) {
        self.today = today;
        self.sections = layout::sections(&self.days, today);
        // An empty library shows its empty state, which states the
        // count: the toolbar shows none.
        self.count = SharedString::from(match self.entries.len() {
            0 => String::new(),
            1 => "1 capture".to_owned(),
            n => format!("{n} captures"),
        });
        self.listed.get_or_insert_with(Instant::now);
        self.generation += 1;
    }

    /// Rewrite the section titles against `today`: past midnight,
    /// "Today" becomes "Yesterday". True when a title changed.
    pub(super) fn relabel(&mut self, today: Day) -> bool {
        if today == self.today {
            return false;
        }
        self.today = today;
        self.sections = layout::sections(&self.days, today);
        self.generation += 1;
        true
    }
}

/// `e`'s caption with its local time already formatted.
pub(super) fn caption(e: &CaptureEntry, time: SharedString) -> Caption {
    Caption {
        time,
        dims: SharedString::from(format!("{} × {}", e.width, e.height)),
        name: SharedString::from(
            e.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
    }
}

/// When a library window reads the store again: one pass at a time, and
/// a store write or a folder change during a pass runs one more pass
/// after it, because the pass in flight may have read the store, or
/// checked a capture's file, before the change.
#[derive(Default)]
pub(super) struct ListPass {
    in_flight: bool,
    again: bool,
}

impl ListPass {
    /// Requests a pass; true when one starts now. A poll while a pass is
    /// in flight is dropped: on a slow store the `REFRESH` timer of a
    /// window with no folder watch would otherwise stack overlapping
    /// scans. A change while a pass is in flight queues one pass after
    /// it.
    pub(super) fn begin(&mut self, after_change: bool) -> bool {
        if self.in_flight {
            self.again |= after_change;
            return false;
        }
        self.in_flight = true;
        true
    }

    /// Ends the pass in flight; true when a change landed during it and
    /// another pass is due.
    pub(super) fn land(&mut self) -> bool {
        self.in_flight = false;
        std::mem::take(&mut self.again)
    }
}
