//! The captures a library window shows, what is derived from them, and
//! when the store is read again.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use gpui::SharedString;
use iris_lib::library::CaptureEntry;

use super::Library;

/// The shown captures, newest first, with each card's label pair and
/// the toolbar count. The fields are private: a listing is shown only
/// through [`Listing::set`], so no label, count, or selection outlives
/// the listing it describes.
#[derive(Default)]
pub(super) struct Listing {
    /// Cards share their entry: render closures capture an Rc bump
    /// instead of cloning the path strings per card per frame.
    entries: Vec<Rc<CaptureEntry>>,
    /// Each entry's fitted name and dimensions, parallel to `entries`:
    /// built once per listing, not per card per frame.
    names: Vec<(SharedString, SharedString)>,
    count: SharedString,
    /// When the first listing landed. None until the store answers:
    /// until then the window shows no count and no empty state.
    listed: Option<Instant>,
}

impl Listing {
    pub(super) fn entries(&self) -> &[Rc<CaptureEntry>] {
        &self.entries
    }

    pub(super) fn names(&self) -> &[(SharedString, SharedString)] {
        &self.names
    }

    /// The toolbar count; empty until the first listing lands.
    pub(super) fn count(&self) -> &SharedString {
        &self.count
    }

    pub(super) fn listed(&self) -> Option<Instant> {
        self.listed
    }

    /// Show `fresh` and drop from `selected` each path it does not
    /// list. Returns the paths whose capture changed in place (an
    /// editor re-save keeps the path and moves `created_ms`), whose
    /// thumbnails are stale, or None when `fresh` is what is shown.
    pub(super) fn set(
        &mut self,
        fresh: Vec<CaptureEntry>,
        selected: &mut Vec<PathBuf>,
    ) -> Option<Vec<PathBuf>> {
        let unchanged = self.listed.is_some()
            && fresh.len() == self.entries.len()
            && fresh
                .iter()
                .zip(&self.entries)
                .all(|(f, e)| f.path == e.path && f.created_ms == e.created_ms);
        if unchanged {
            return None;
        }
        let shown: HashMap<&Path, i64> = self
            .entries
            .iter()
            .map(|e| (e.path.as_path(), e.created_ms))
            .collect();
        let stale = fresh
            .iter()
            .filter(|e| {
                shown
                    .get(e.path.as_path())
                    .is_some_and(|&t| t != e.created_ms)
            })
            .map(|e| e.path.clone())
            .collect();
        let paths: HashSet<&Path> = fresh.iter().map(|e| e.path.as_path()).collect();
        selected.retain(|p| paths.contains(p.as_path()));
        self.names = fresh.iter().map(Library::entry_name).collect();
        self.count = SharedString::from(match fresh.len() {
            1 => "1 capture".to_owned(),
            n => format!("{n} captures"),
        });
        self.entries = fresh.into_iter().map(Rc::new).collect();
        self.listed.get_or_insert_with(Instant::now);
        Some(stale)
    }
}

/// When a library window reads the store again: one pass at a time, and
/// a store write during a pass runs one more pass after it, because the
/// pass in flight may have read the store before the write.
#[derive(Default)]
pub(super) struct ListPass {
    in_flight: bool,
    again: bool,
}

impl ListPass {
    /// Requests a pass; true when one starts now. A poll while a pass is
    /// in flight is dropped: on a slow store the 1.5s timer would
    /// otherwise stack overlapping scans. A write while a pass is in
    /// flight queues one pass after it.
    pub(super) fn begin(&mut self, after_write: bool) -> bool {
        if self.in_flight {
            self.again |= after_write;
            return false;
        }
        self.in_flight = true;
        true
    }

    /// Ends the pass in flight; true when a write landed during it and
    /// another pass is due.
    pub(super) fn land(&mut self) -> bool {
        self.in_flight = false;
        std::mem::take(&mut self.again)
    }
}
