//! Library panel: the main window. A grid of captures, newest first.
//!
//! Click opens the canvas editor, Ctrl/Shift-click multi-selects, and
//! hover reveals per-card annotate/copy/delete actions. The toolbar
//! starts a region capture or opens settings. The grid re-reads
//! library.json on a slow timer so captures taken while the panel is
//! open appear without a restart.
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::*;
use iris_lib::library::CaptureEntry;

mod card;
mod entries;
mod render;
#[cfg(test)]
mod tests;

pub(super) const CARD_W: f32 = 216.0;
pub(super) const THUMB_H: f32 = 132.0;
pub(super) const GAP: f32 = 16.0;
pub(super) const REFRESH: Duration = Duration::from_millis(1500);

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

pub struct Library {
    /// Cards share their entry: render closures capture an Rc bump
    /// instead of cloning the path strings per card per frame.
    pub(super) entries: Vec<Rc<CaptureEntry>>,
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
    pub(super) thumb_cache: std::collections::HashMap<PathBuf, std::sync::Arc<RenderImage>>,
    /// Set when `entries` is reassigned: the next render rebuilds the
    /// live-path set and prunes `thumb_cache`, instead of rebuilding
    /// the set every frame.
    pub(super) entries_dirty: bool,
    /// Display strings (truncated file name, dimensions) parallel to
    /// `entries`: rebuilding them per card per frame is an allocation
    /// a frame per card for values that only change with the entry.
    pub(super) entry_names: Vec<(SharedString, SharedString)>,
    /// Toolbar labels rebuilt only when their value changes: the
    /// counts format a String per frame otherwise.
    pub(super) count_label: SharedString,
    pub(super) sel_label: SharedString,
    /// Empty-state line: the hotkey is fixed for the session.
    pub(super) empty_label: SharedString,
    pub(super) drag_fired: bool,
    pub(super) help: bool,
    /// First-render clock for the open cascade.
    pub(super) opened: Instant,
    pub(super) status: Option<String>,
    pub(super) focus: FocusHandle,
    /// This window's unique WM_CLASS, for the title-bar drag.
    pub(super) class: SharedString,
    /// The config snapshot: render reads the hotkey every frame, and
    /// Config::load() hits the disk each call.
    pub(super) cfg: iris_lib::config::Config,
    /// Rubber-band selection: window-space anchor and current point
    /// while the left button is held on empty grid space.
    pub(super) band: Option<(f32, f32, f32, f32)>,
    /// Scroll offset of the card grid, so band math stays in
    /// document space while the user drags.
    /// A refresh already in flight: the 1.5s poll must not stack
    /// overlapping list() passes on a slow or network shots dir.
    pub(super) refresh_in_flight: bool,
    pub(super) scroll: gpui::ScrollHandle,
}

/// Open the library window.
pub fn open(cx: &mut App) -> Result<(), String> {
    let focus = cx.focus_handle();
    let win = (960.0f32, 640.0f32);
    let origin = crate::xwin::centered_origin(cx, win.0, win.1, (140.0, 90.0));
    let win_id = crate::xwin::unique_id("dev.iris.library");
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
                app_id: Some(win_id.clone()),
                window_min_size: Some(size(px(640.), px(480.))),
                window_decorations: Some(WindowDecorations::Client),
                tabbing_identifier: None,
            },
            |_, cx| {
                cx.new(|cx| {
                    // slow or network shots dir that stalls the open.
                    // Open empty and populate from the background, the
                    // same path the refresh poll takes.
                    let mut this = Library {
                        entry_names: Vec::new(),
                        count_label: SharedString::from("0 captures"),
                        sel_label: SharedString::from(""),
                        empty_label: SharedString::from(format!(
                            "No captures yet — press {}",
                            iris_lib::config::Config::load().capture_hotkey
                        )),
                        entries: Vec::new(),
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
                        thumb_cache: std::collections::HashMap::new(),
                        entries_dirty: false,
                        drag_fired: false,
                        help: false,
                        opened: Instant::now(),
                        status: None,
                        focus,
                        class: SharedString::from(win_id.clone()),
                        cfg: iris_lib::config::Config::load(),
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
    crate::xwin::place_after_map(win_id, origin.0, origin.1);
    Ok(())
}
