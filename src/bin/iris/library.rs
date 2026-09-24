//! Library panel: the main window. A grid of captures, newest first.
//!
//! Click opens the canvas editor, Ctrl/Shift-click multi-selects, and
//! hover reveals per-card annotate/copy/delete actions. The toolbar
//! starts a region capture or opens settings. The grid re-reads
//! library.json on a slow timer so captures taken while the panel is
//! open appear without a restart.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::*;

mod card;
mod entries;
mod listing;
mod render;
#[cfg(test)]
mod tests;

pub(super) const CARD_W: f32 = 216.0;
pub(super) const THUMB_H: f32 = 132.0;
pub(super) const GAP: f32 = 16.0;
/// The label row under a card's thumbnail: its inset on each side and
/// the space between the name and the dimensions.
pub(super) const LABEL_PAD: f32 = 2.0;
pub(super) const LABEL_GAP: f32 = 6.0;
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

/// The card rows that intersect the scroll viewport, plus the heights
/// of the full-width spacer rows that stand in for the rows above and
/// below. A spacer occupies a whole wrap line, so its height is the
/// covered rows' pitch minus the inter-row GAP the line break adds.
/// Returns `(first_row, last_row, top_spacer_h, bottom_spacer_h)`;
/// `top_spacer_h`/`bottom_spacer_h` are 0 when no spacer is needed.
/// `viewport_h <= 0` means the scroll bounds are not laid out yet, so
/// every row renders that frame rather than flash an empty grid.
pub(super) fn visible_rows(
    n: usize,
    cols: usize,
    scroll_y: f32,
    viewport_h: f32,
) -> (usize, usize, f32, f32) {
    let card_h = THUMB_H + 8.0 + 18.0;
    let row_pitch = card_h + GAP;
    let rows = n.div_ceil(cols.max(1));
    if rows == 0 {
        return (0, 0, 0.0, 0.0);
    }
    if viewport_h <= 0.0 {
        return (0, rows - 1, 0.0, 0.0);
    }
    let first = (((scroll_y - GAP) / row_pitch).floor().max(0.0) as usize).min(rows - 1);
    // The last row whose top is above the viewport bottom. A row top
    // exactly at the bottom edge is not visible, so ceil(x) - 1, not
    // ceil(x): x = 4.0 means row 4 starts at the edge and is excluded.
    let last = (((scroll_y + viewport_h - GAP) / row_pitch).ceil() as usize)
        .saturating_sub(1)
        .min(rows - 1);
    let first_row = first.saturating_sub(1);
    let last_row = (last + 1).min(rows - 1);
    let top_h = if first_row > 0 {
        first_row as f32 * row_pitch - GAP
    } else {
        0.0
    };
    let bottom_h = if last_row + 1 < rows {
        (rows - last_row - 1) as f32 * row_pitch - GAP
    } else {
        0.0
    };
    (first_row, last_row, top_h, bottom_h)
}

/// The open cascade: each card rises and fades in over CASCADE_RISE
/// seconds, CASCADE_STAGGER after the card before it. The stagger stops
/// growing after CASCADE_CARDS cards, so every card is in place by
/// CASCADE_END: a card scrolled into view early joins the end of the
/// wave instead of staying hidden, and none jumps when the wave stops.
const CASCADE_STAGGER: f32 = 0.025;
const CASCADE_RISE: f32 = 0.35;
const CASCADE_CARDS: usize = 24;
pub(super) const CASCADE_END: f32 = CASCADE_STAGGER * CASCADE_CARDS as f32 + CASCADE_RISE;

/// Card `index`'s cascade progress `at` seconds into the wave: 0 is
/// hidden and lowered, 1 is in place.
pub(super) fn cascade(at: f32, index: usize) -> f32 {
    ((at - index.min(CASCADE_CARDS) as f32 * CASCADE_STAGGER) / CASCADE_RISE).clamp(0.0, 1.0)
}

pub struct Library {
    listing: listing::Listing,
    pub(super) selected: Vec<PathBuf>,
    /// Membership set for `selected`, rebuilt on the render after a
    /// mutation instead of hashed fresh every frame.
    pub(super) sel_set: std::collections::HashSet<PathBuf>,
    pub(super) sel_dirty: bool,
    pub(super) anchor: Option<usize>,
    pub(super) hovered: Option<usize>,
    /// Pointer-coupled springs per card: hover lift and selection
    /// pop. They reverse mid-flight, which is the liquid feel.
    pub(super) springs: std::collections::HashMap<usize, crate::motion::Spring>,
    pub(super) sel_springs: std::collections::HashMap<usize, crate::motion::Spring>,
    pub(super) press_spring: crate::motion::Spring,
    pub(super) pressed: Option<usize>,
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
    /// Empty-state line: the hotkey is fixed for the session.
    pub(super) empty_label: SharedString,
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
    /// A refresh already in flight: the 1.5s poll must not stack
    /// overlapping list() passes on a slow or network shots dir.
    pub(super) refresh_in_flight: bool,
    /// Scroll offset of the card grid, so band math stays in
    /// document space during a band drag.
    pub(super) scroll: gpui::ScrollHandle,
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
    let handle = cx
        .open_window(
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
                app_id: Some("dev.iris.library".to_string()),
                window_min_size: Some(MIN_SIZE),
                window_decorations: Some(WindowDecorations::Client),
                tabbing_identifier: None,
            },
            |window, cx| {
                window.set_window_title("Library - iris");
                cx.new(|cx| {
                    // Listing the shots dir here would stall the open on a
                    // slow or network dir: open empty and populate from the
                    // background, the same path the refresh poll takes.
                    let cfg = iris_lib::config::Config::load();
                    let mut this = Library {
                        listing: listing::Listing::default(),
                        sel_label: SharedString::from(""),
                        empty_label: SharedString::from(format!(
                            "No captures yet — press {}",
                            cfg.capture_hotkey
                        )),
                        selected: Vec::new(),
                        refresh_in_flight: false,
                        sel_set: std::collections::HashSet::new(),
                        sel_dirty: false,
                        anchor: None,
                        hovered: None,
                        springs: std::collections::HashMap::new(),
                        sel_springs: std::collections::HashMap::new(),
                        press_spring: crate::motion::Spring::default(),
                        pressed: None,
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
                    };
                    this.arm_refresh(cx);
                    this.refresh_now(cx);
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
        })
        .detach();
    }
    Ok(())
}
