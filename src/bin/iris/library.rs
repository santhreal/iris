//! Library panel: the main window. A grid of captures, newest first,
//! grouped by the local day each was taken.
//!
//! Click opens the canvas editor, Ctrl/Shift-click multi-selects, the
//! arrow keys move the selection, Return opens and Space previews the
//! selected capture, and hover reveals per-card copy/reveal/trash
//! actions. The toolbar starts a region capture or opens settings. The
//! grid lists a capture this process saves at once, and a watch on the
//! capture folders drops a capture another program deletes or moves
//! away.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::*;

mod card;
mod context_menu;
mod entries;
mod folders;
mod keys;
mod layout;
mod listing;
mod preview;
mod render;
#[cfg(test)]
mod tests;

pub(super) const CARD_W: f32 = 216.0;
pub(super) const THUMB_H: f32 = 132.0;
/// Space between a card's thumbnail and its caption, and the caption
/// line's height.
pub(super) const CAPTION_GAP: f32 = 7.0;
pub(super) const CAPTION_H: f32 = 15.0;
pub(super) const CARD_H: f32 = THUMB_H + CAPTION_GAP + CAPTION_H;
/// Space between two cards of a row.
pub(super) const GAP: f32 = 16.0;
/// How often a window whose capture folders the OS refused to watch
/// reads the store.
pub(super) const REFRESH: Duration = Duration::from_millis(1500);
/// Rows of thumbnail slack kept decoded beyond the rendered range:
/// scroll-back inside the window hits warm tiles, outside it pays a
/// re-decode. 8 rows at 4 cols is ~64 tiles of 432x264, ~29MB of atlas.
pub(super) const THUMB_KEEP_ROWS: usize = 8;

/// Decoded thumbnails, keyed by capture path.
pub(super) type ThumbCache = std::collections::HashMap<PathBuf, std::sync::Arc<RenderImage>>;

/// Drop the thumbnails `keep` rejects from `cache` and free their
/// sprite-atlas tiles: the last `Arc<RenderImage>` dropping leaves its
/// tile allocated in every window that painted it.
pub(super) fn evict_thumbs(cache: &mut ThumbCache, keep: impl Fn(&Path) -> bool, cx: &mut App) {
    cache.retain(|p, img| {
        let kept = keep(p.as_path());
        if !kept {
            crate::widgets::release_render(img, cx);
        }
        kept
    });
}

pub struct Library {
    listing: listing::Listing,
    /// The grid at the window's width, rebuilt when the listing or the
    /// width changes, not per frame.
    layout: layout::Layout,
    /// The listing generation and width `layout` was built for.
    layout_key: (u64, f32),
    pub(super) selected: Vec<PathBuf>,
    /// Membership set for `selected`, rebuilt on the render after a
    /// mutation instead of hashed fresh every frame.
    pub(super) sel_set: std::collections::HashSet<PathBuf>,
    pub(super) sel_dirty: bool,
    /// Where a Shift-click or Shift-arrow range starts.
    pub(super) anchor: Option<usize>,
    /// The card the arrow keys move from, Return opens, and Space
    /// previews.
    pub(super) cursor: Option<usize>,
    pub(super) hovered: Option<usize>,
    /// Hover springs per card: the lift and the quick actions' fade.
    /// They reverse mid-flight when the pointer leaves.
    pub(super) springs: std::collections::HashMap<usize, crate::motion::Spring>,
    pub(super) last_frame: Option<Instant>,
    pub(super) drag_start: Option<(usize, f32, f32)>,
    /// Decoded thumbnails, keyed by capture path. Reading and
    /// decoding every PNG on every animation frame was the judder.
    /// Bounded to the viewport's keep window: a large library holding
    /// every decoded thumb is unbounded GPU atlas memory, so render
    /// evicts tiles outside the window and prefetch re-decodes them
    /// on scroll-back.
    pub(super) thumb_cache: ThumbCache,
    /// The index range the cache currently keeps: rendered rows plus
    /// THUMB_KEEP_ROWS of slack either side. Recomputed per render;
    /// eviction and prefetch only run when it moves.
    pub(super) thumb_keep: (usize, usize),
    /// A prefetch pass already decoding: scroll moves faster than PNG
    /// decodes, so a new range is parked in prefetch_want instead of
    /// stacking tasks.
    pub(super) prefetch_in_flight: bool,
    /// The newest keep range a prefetch has not covered yet.
    pub(super) prefetch_want: Option<(usize, usize)>,
    /// Set when a new listing is shown: the next render rebuilds the
    /// live-path set and prunes `thumb_cache`, instead of rebuilding
    /// the set every frame.
    pub(super) entries_dirty: bool,
    /// The selection count label, rebuilt only when the selection
    /// changes: formatting it per frame is a String a frame.
    pub(super) sel_label: SharedString,
    /// The empty state's hint line: the hotkey is fixed for the session.
    pub(super) empty_hint: SharedString,
    pub(super) drag_fired: bool,
    pub(super) help: bool,
    pub(super) status: Option<String>,
    pub(super) focus: FocusHandle,
    /// The config snapshot: render reads the hotkey every frame, and
    /// Config::load() hits the disk each call.
    pub(super) cfg: iris_lib::config::Config,
    /// Rubber-band selection: window-space anchor and current point
    /// while the left button is held on empty grid space.
    pub(super) band: Option<(f32, f32, f32, f32)>,
    /// The store read in flight, and whether a write queued another.
    list_pass: listing::ListPass,
    /// The watch on the listed captures' folders.
    folders: folders::Folders,
    /// Scroll offset of the card grid, so band math stays in
    /// document space during a band drag.
    pub(super) scroll: gpui::ScrollHandle,
    /// The Space preview, while it is open.
    preview: Option<preview::Preview>,
    /// The card context menu, while it is open.
    menu: Option<context_menu::ContextMenu>,
}

/// The library window's minimum logical size, where its resize stops.
pub(super) const MIN_SIZE: Size<Pixels> = size(px(640.), px(480.));

/// Open the library window, or raise the one already open.
pub fn open(cx: &mut App) -> Result<(), String> {
    if crate::widgets::raise_open::<Library>(cx, |_| true) {
        return Ok(());
    }
    let focus = cx.focus_handle();
    let win = (960.0f32, 640.0f32);
    let origin = crate::sys::window::centered_origin(cx, win.0, win.1, (140.0, 90.0));
    let handle = crate::widgets::open_window(
        cx,
        "Library - iris",
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(origin.0), px(origin.1)),
                size: size(px(win.0), px(win.1)),
            })),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_movable: true,
            is_resizable: true,
            is_minimizable: true,
            display_id: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_min_size: Some(MIN_SIZE),
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        },
        |_, cx| {
            cx.new(|cx| {
                // Listing the shots dir here would stall the open on a
                // slow or network dir: open empty and populate from the
                // background, the same path every later read takes.
                let cfg = iris_lib::config::Config::load();
                let mut this = Library {
                    listing: listing::Listing::default(),
                    layout: layout::Layout::default(),
                    layout_key: (u64::MAX, 0.0),
                    sel_label: SharedString::from(""),
                    empty_hint: SharedString::from(format!(
                        "Press {} to capture a region of the screen.",
                        cfg.capture_hotkey
                    )),
                    selected: Vec::new(),
                    list_pass: listing::ListPass::default(),
                    folders: folders::Folders::Unwatched,
                    sel_set: std::collections::HashSet::new(),
                    sel_dirty: false,
                    anchor: None,
                    cursor: None,
                    hovered: None,
                    springs: std::collections::HashMap::new(),
                    last_frame: None,
                    drag_start: None,
                    thumb_keep: (0, 0),
                    prefetch_in_flight: false,
                    prefetch_want: None,
                    thumb_cache: std::collections::HashMap::new(),
                    entries_dirty: false,
                    drag_fired: false,
                    help: false,
                    status: None,
                    focus,
                    cfg,
                    band: None,
                    scroll: gpui::ScrollHandle::new(),
                    preview: None,
                    menu: None,
                };
                this.refresh(false, cx);
                this.arm_midnight(cx);
                this
            })
        },
    )
    .map_err(|e| format!("open library window: {e}"))?;
    // The thumbnail images sit in GPUI's app-global asset cache;
    // return them when the window dies, whichever way it closes.
    if let Ok(entity) = handle.entity(cx) {
        cx.observe_release(&entity, |this, cx| {
            for img in this.thumb_cache.values() {
                crate::widgets::release_render(img, cx);
            }
            if let Some(img) = this.preview.as_ref().and_then(|p| p.image.as_ref()) {
                crate::widgets::release_render(img, cx);
            }
        })
        .detach();
    }
    Ok(())
}

/// The store changed in this process: the open library window lists it
/// now.
pub fn store_changed(cx: &mut App) {
    for window in cx.windows() {
        if let Some(library) = window.downcast::<Library>() {
            let _ = library.update(cx, |this, _, cx| this.refresh(true, cx));
        }
    }
}

impl Library {
    /// Rewrite the day titles at each local midnight, for the life of
    /// the window: one timer a day, no per-frame clock read.
    fn arm_midnight(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            let wait = Duration::from_secs(layout::secs_to_midnight() + 1);
            cx.background_executor().timer(wait).await;
            let alive = this.update(cx, |this, cx| {
                if this.listing.relabel(layout::today()) {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        })
        .detach();
    }
}
