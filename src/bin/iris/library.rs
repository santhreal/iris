//! Library panel: the main window. A grid of captures, newest first.
//!
//! Click opens the canvas editor, Ctrl/Shift-click multi-selects, and
//! hover reveals per-card annotate/copy/delete actions. The toolbar
//! starts a region capture or opens settings. The grid re-reads
//! library.json on a slow timer so captures taken while the panel is
//! open appear without a restart.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iris_lib::library::{self, CaptureEntry};
use gpui::*;

use crate::{editor, pipeline, theme};

const CARD_W: f32 = 216.0;
const THUMB_H: f32 = 132.0;
const GAP: f32 = 16.0;
const REFRESH: Duration = Duration::from_millis(1500);

pub struct Library {
    entries: Vec<CaptureEntry>,
    selected: Vec<PathBuf>,
    anchor: Option<usize>,
    hovered: Option<usize>,
    /// Pointer-coupled springs per card: hover lift and selection
    /// pop. They reverse mid-flight, which is the liquid feel.
    springs: std::collections::HashMap<usize, crate::motion::Spring>,
    sel_springs: std::collections::HashMap<usize, crate::motion::Spring>,
    press_spring: crate::motion::Spring,
    pressed: Option<usize>,
    last_frame: Option<Instant>,
    drag_start: Option<(usize, f32, f32)>,
    /// Decoded thumbnails, keyed by capture path. Reading and
    /// decoding every PNG on every animation frame was the judder.
    thumb_cache: std::collections::HashMap<PathBuf, std::sync::Arc<RenderImage>>,
    /// Set when `entries` is reassigned: the next render rebuilds the
    /// live-path set and prunes `thumb_cache`, instead of rebuilding
    /// the set every frame.
    entries_dirty: bool,
    /// Display strings (truncated file name, dimensions) parallel to
    /// `entries`: rebuilding them per card per frame is an allocation
    /// a frame per card for values that only change with the entry.
    /// SharedString so the per-frame clone is an Arc bump.
    entry_names: Vec<(SharedString, SharedString)>,
    drag_fired: bool,
    help: bool,
    /// First-render clock for the open cascade.
    opened: Instant,
    status: Option<String>,
    focus: FocusHandle,
    /// This window's unique WM_CLASS, for the title-bar drag.
    class: String,
    /// The config snapshot: render reads the hotkey every frame, and
    /// Config::load() hits the disk each call.
    cfg: iris_lib::config::Config,
    /// Rubber-band selection: window-space anchor and current point
    /// while the left button is held on empty grid space.
    band: Option<(f32, f32, f32, f32)>,
    /// Scroll offset of the card grid, so band math stays in
    /// document space while the user drags.
    scroll: gpui::ScrollHandle,
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
                    let entries = library::list();
                    let mut this = Library {
                        entry_names: entries.iter().map(Library::entry_name).collect(),
                        entries,
                        selected: Vec::new(),
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
                        class: win_id.clone(),
                        cfg: iris_lib::config::Config::load(),
                        band: None,
                        scroll: gpui::ScrollHandle::new(),
                    };
                    this.arm_refresh(cx);
                    this.prefetch_thumbs(cx);
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

impl Library {
    /// The card's display strings: the file name truncated to ~19
    /// chars (GPUI's text_ellipsis only fires on wrapped text; nowrap
    /// clips, so the truncation happens here) and the dimensions.
    /// Computed once per entry, not per frame.
    fn entry_name(e: &CaptureEntry) -> (SharedString, SharedString) {
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
        (name, SharedString::from(format!("{}×{}", e.width, e.height)))
    }
    /// Read thumbnails off the main thread and fill the cache in one
    /// delivery: a first paint that blocks on N disk reads stutters.
    fn prefetch_thumbs(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<(std::path::PathBuf, std::path::PathBuf)> = self
            .entries
            .iter()
            .filter(|e| !self.thumb_cache.contains_key(&e.path))
            .map(|e| (e.path.clone(), e.thumb.clone()))
            .collect();
        if missing.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            // One background task per thumb: the executor is a pool,
            // so N PNG decodes run across cores instead of serially
            // on one task.
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
            let _ = this.update(cx, |this, cx| {
                this.thumb_cache.extend(loaded);
                cx.notify();
            });
        })
        .detach();
    }

    /// Poll the store so captures from any process surface here. The
    /// directory scan runs off the main thread.
    fn arm_refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                let fresh = cx
                    .background_executor()
                    .spawn(async move { library::list() })
                    .await;
                let alive = this.update(cx, |this, cx| {
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
                        this.entries = fresh;
                        this.entries_dirty = true;
                        this.entry_names = this.entries.iter().map(Self::entry_name).collect();
                        this.selected
                            .retain(|p| this.entries.iter().any(|e| &e.path == p));
                        this.prefetch_thumbs(cx);
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn toggle_select(&mut self, path: PathBuf) {
        if let Some(i) = self.selected.iter().position(|p| *p == path) {
            self.selected.remove(i);
        } else {
            self.selected.push(path);
        }
    }

    fn click_card(
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
                cx.notify();
            }
        } else if let Err(e) = editor::open(cx, &entry.path, Some(Self::card_morph_rect(ev, window)), None) {
            self.status = Some(e);
            cx.notify();
        }
    }

/// The rect the editor morphs out of when a card opens: the card's
/// thumb geometry centered on the click, in screen coordinates. The
/// true card origin is layout state; centering on the click is within
/// half a card of it, well inside the spring's travel.
fn card_morph_rect(ev: &ClickEvent, window: &Window) -> (f32, f32, f32, f32) {
    let (wx, wy) = match window.window_bounds() {
        WindowBounds::Windowed(b) => (f32::from(b.origin.x), f32::from(b.origin.y)),
        _ => (0.0, 0.0),
    };
    let (mx, my): (f32, f32) = (ev.position().x.into(), ev.position().y.into());
    (wx + mx - CARD_W / 2.0, wy + my - THUMB_H / 2.0, CARD_W, THUMB_H)
}

/// Open the containing directory of a capture with xdg-open.
fn open_containing_folder(path: &std::path::Path) {
    let parent = path.parent().unwrap_or(path).to_path_buf();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("xdg-open")
            .arg(&parent)
            .spawn();
    });
}

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
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

    fn delete_selection(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = std::mem::take(&mut self.selected);
        // delete_many + the follow-up list() stat every file; keep the
        // disk work off the UI thread.
        let task = cx.background_executor().spawn(async move {
            let errors = library::delete_many(&paths);
            (errors, library::list())
        });
        cx.spawn(async move |this, cx| {
            let (errors, entries) = task.await;
            let _ = this.update(cx, |this, cx| {
                this.entries = entries;
                this.entries_dirty = true;
                this.entry_names = this.entries.iter().map(Self::entry_name).collect();
                this.status = (errors > 0).then(|| format!("{errors} delete(s) failed"));
                cx.notify();
            });
        })
        .detach();
    }

    /// Close the rubber band: a sub-4px drag is a click on empty
    /// space and clears the selection; anything larger selects every
    /// card whose rect intersects the band.
    fn finish_band(&mut self, _ev: &MouseUpEvent, window: &Window, cx: &mut Context<Self>) {
        let Some((x0, y0, x1, y1)) = self.band.take() else { return };
        let (dx, dy) = (x1 - x0, y1 - y0);
        if dx * dx + dy * dy < 16.0 {
            if !self.selected.is_empty() {
                self.selected.clear();
                cx.notify();
            }
            return;
        }
        let (bx0, bx1) = (x0.min(x1), x0.max(x1));
        let (by0, by1) = (y0.min(y1), y0.max(y1));
        // Card rects live in document space: grid top is the 56px
        // frame toolbar, scroll shifts rows up.
        let width: f32 = window.bounds().size.width.into();
        let scroll_y: f32 = self.scroll.offset().y.into();
        let cols = ((width - GAP) / (CARD_W + GAP)).floor().max(1.0) as usize;
        let card_h = THUMB_H + 8.0 + 18.0;
        self.selected.clear();
        for (i, e) in self.entries.iter().enumerate() {
            let (r, c) = (i / cols, i % cols);
            let cx0 = GAP + c as f32 * (CARD_W + GAP);
            let cy0 = 56.0 + GAP + r as f32 * (card_h + GAP) - scroll_y;
            if cx0 < bx1 && cx0 + CARD_W > bx0 && cy0 < by1 && cy0 + card_h > by0 {
                self.selected.push(e.path.clone());
            }
        }
        self.anchor = self.entries.iter().position(|e| self.selected.contains(&e.path));
        cx.notify();
    }

    fn start_capture(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = crate::daemon::dispatch(cx, &crate::daemon::Command::Capture) {
            self.status = Some(e);
            cx.notify();
        }
    }
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hotkey = self.cfg.capture_hotkey.clone();
        let selecting = !self.selected.is_empty();
        let this_help = self.help;

        // Right-side toolbar cluster, handed to the shared frame.
        let mut cluster: Vec<AnyElement> = Vec::new();
        if selecting {
            let n = self.selected.len();
            cluster.push(
                div()
                    .text_xs()
                    .text_color(theme::FG_DIM)
                    .child(format!("{n} selected"))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("sel-copy", "Copy", false)
                    .on_click(cx.listener(|this, _, _, cx| this.copy_selection(cx)))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("sel-delete", "Delete", false)
                    .on_click(cx.listener(|this, _, _, cx| this.delete_selection(cx)))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("sel-clear", "Clear", false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected.clear();
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        } else {
            cluster.push(
                div()
                    .text_xs()
                    .text_color(theme::FG_FAINT)
                    .child(format!("{} captures", self.entries.len()))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("capture", "Capture", true)
                    .on_click(cx.listener(|this, _, _, cx| this.start_capture(cx)))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("settings", "Settings", false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Err(e) = crate::settings::open(cx) {
                            this.status = Some(e);
                            cx.notify();
                        }
                    }))
                    .into_any_element(),
            );
            cluster.push(
                crate::widgets::button("help", "?", false)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.help = !this.help;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }

        let mut grid = div()
            .id("grid")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p(px(GAP))
            .flex()
            .flex_wrap()
            .gap(px(GAP))
            .items_start()
            .content_start()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    // A card's own mousedown set drag_start first; only
                    // empty grid space starts a rubber band.
                    if this.drag_start.is_none() {
                        this.band = Some((
                            ev.position.x.into(),
                            ev.position.y.into(),
                            ev.position.x.into(),
                            ev.position.y.into(),
                        ));
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
                cx.listener(|this, ev: &MouseUpEvent, window, cx| {
                    this.finish_band(ev, window, cx);
                }),
            );

        if self.entries.is_empty() {
            grid = grid.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(theme::FG_FAINT)
                    .child(format!("No captures yet — press {hotkey}")),
            );
        }

        let cascade = self.opened.elapsed().as_secs_f32() < 1.6;
        // The cache fills from the background prefetch; a card whose
        // thumb has not landed draws without its image until then.
        // A set, not a nested scan: retain() over the cache against a
        // per-entry linear probe is O(cache * entries) PathBuf
        // compares. Rebuilt only when entries changed: a per-frame
        // rebuild is O(entries) PathBuf hashing for a cache that
        // almost never has stale keys.
        if self.entries_dirty {
            self.entries_dirty = false;
            let live_paths: std::collections::HashSet<&std::path::Path> =
                self.entries.iter().map(|e| e.path.as_path()).collect();
            self.thumb_cache.retain(|p, _| live_paths.contains(p.as_path()));
        }
        // Advance pointer-coupled springs by the real frame delta;
        // keep rendering until everything settles.
        let now = Instant::now();
        let dt = self
            .last_frame
            .map(|t| (now - t).as_secs_f32())
            .unwrap_or(1.0 / 60.0);
        self.last_frame = Some(now);
        let mut live = false;
        let n = self.entries.len();
        if let Some(h) = self.hovered {
            self.springs.entry(h).or_default();
        }
        self.springs.retain(|i, _| *i < n);
        for (i, s) in self.springs.iter_mut() {
            let target = if self.hovered == Some(*i) { 1.0 } else { 0.0 };
            s.to(target, dt);
            if !s.settled(target) {
                live = true;
            }
        }
        self.sel_springs.retain(|i, _| *i < n);
        // Membership is checked per entry below and per card; a Vec
        // scan is O(n*m) PathBuf compares a frame. Build the set once.
        let sel_set: std::collections::HashSet<&std::path::Path> =
            self.selected.iter().map(PathBuf::as_path).collect();
        for i in 0..n {
            let target = if sel_set.contains(self.entries[i].path.as_path()) { 1.0 } else { 0.0 };
            let s = self.sel_springs.entry(i).or_default();
            if target == 0.0 && s.settled(0.0) && s.value == 0.0 {
                continue;
            }
            s.to(target, dt);
            if !s.settled(target) {
                live = true;
            }
        }
        {
            let target = if self.pressed.is_some() { 1.0 } else { 0.0 };
            self.press_spring.to(target, dt);
            if !self.press_spring.settled(target) {
                live = true;
            }
        }
        if live {
            window.request_animation_frame();
        }
        for (index, entry) in self.entries.iter().enumerate() {
            let amt = self.springs.get(&index).map(|s| s.value).unwrap_or(0.0);
            // Open cascade: cards rise and fade in with a 25ms stagger.
            let et = if cascade {
                let t = ((self.opened.elapsed().as_secs_f32() - index as f32 * 0.025) / 0.35)
                    .clamp(0.0, 1.0);
                if t < 1.0 {
                    window.request_animation_frame();
                }
                t
            } else {
                1.0
            };
            grid = grid.child(self.card(index, entry, amt, et, &sel_set, cx));
        }

        let mut root = div()
            .size_full()
            .font_family(theme::FONT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                let meta = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
                match key {
                    "escape" if this.help => {
                        this.help = false;
                    }
                    "?" | "/" => {
                        this.help = !this.help;
                    }
                    "escape" if !this.selected.is_empty() => {
                        this.selected.clear();
                    }
                    "escape" => {
                        window.remove_window();
                        return;
                    }
                    "delete" | "backspace" if !this.selected.is_empty() => {
                        this.delete_selection(cx);
                    }
                    "a" if meta => {
                        this.selected = this.entries.iter().map(|e| e.path.clone()).collect();
                    }
                    _ => return,
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    // Released anywhere: the pressed card un-sinks.
                    if this.pressed.is_some() {
                        this.pressed = None;
                        cx.notify();
                    }
                }),
            )
            .child(crate::widgets::window_frame("Iris", self.class.clone(), cluster, grid));

        if let Some((x0, y0, x1, y1)) = self.band {
            let (bx, by) = (x0.min(x1), y0.min(y1));
            root = root.child(
                div()
                    .absolute()
                    .left(px(bx))
                    .top(px(by))
                    .w(px((x1 - x0).abs()))
                    .h(px((y1 - y0).abs()))
                    .bg(theme::alpha(theme::ACCENT, 0.12))
                    .border_1()
                    .border_color(theme::alpha(theme::ACCENT, 0.6))
                    .rounded(px(4.)),
            );
        }

        if this_help {
            let hotkey = hotkey.clone();
            root = root.child(                crate::widgets::shortcuts_sheet(vec![
                    ("Capture region", hotkey),                    ("Record window", "Ctrl+Shift+R".to_string()),
                    ("Open in editor", "Click".to_string()),
                    ("Select all", "Ctrl+A".to_string()),
                    ("Delete selection", "Delete".to_string()),
                    ("Clear selection", "Esc".to_string()),
                    ("Fullscreen", "Double-click title bar".to_string()),
                    ("This sheet", "?".to_string()),
                ])
                .on_click(cx.listener(|this, _, _, cx| {
                    this.help = false;
                    cx.notify();
                })),
            );
        }

        if let Some(status) = &self.status {
            root = root.child(crate::widgets::status_pill(status, 10.0));
        }

        root
    }
}

impl Library {
    fn card(&self, index: usize, entry: &CaptureEntry, hover_amt: f32, enter: f32, sel_set: &std::collections::HashSet<&std::path::Path>, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = sel_set.contains(entry.path.as_path());
        let hovered = self.hovered == Some(index);
        let sel_amt = self.sel_springs.get(&index).map(|s| s.value).unwrap_or(0.0);
        let squish = if self.pressed == Some(index) {
            self.press_spring.value
        } else {
            0.0
        };
        let path = entry.path.clone();

        let mut thumb = div()
            .w_full()
            .h(px(THUMB_H))
            .rounded(px(10.))
            .overflow_hidden()
            .bg(theme::SURFACE)
            .shadow(vec![{
                let mut s = theme::shadow_rest();
                s.color.a *= 0.6 + 0.9 * hover_amt;
                s.blur_radius = px((10. + 8.0 * hover_amt) * (1.0 - 0.3 * squish));
                s.offset.y = px(2. + 4.0 * hover_amt + 1.0 * squish);
                s
            }])
            .border_1()
            .border_color(if selected { theme::FG } else { theme::alpha(theme::FG, 0.0) });
        thumb = thumb.child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(
                    img(ImageSource::Render(
                        self.thumb_cache.get(&entry.path).cloned().unwrap_or_else(|| {
                            // One shared transparent tile: minting a
                            // RenderImage per missing thumb per frame
                            // allocates an atlas slot on every paint.
                            static BLANK: std::sync::LazyLock<Arc<RenderImage>> =
                                std::sync::LazyLock::new(|| {
                                    crate::widgets::render_image_from_rgba(1, 1, &[0, 0, 0, 0])
                                });
                            BLANK.clone()
                        }),
                    ))
                    .size_full()
                    .object_fit(ObjectFit::Cover),
                ),
        );

        // Selection circle, Photos-style: solid with a check when
        // selected. Its pop is spring-driven; otherwise it rides
        // the hover lift (or stays faintly up while selecting).
        let selecting = !self.selected.is_empty();
        let circle_vis = sel_amt.max(hover_amt).max(if selecting && !selected { 0.85 } else { 0.0 });
        let circle = div()
            .id(ElementId::NamedInteger("sel".into(), index as u64))
            .absolute()
            .top(px(6.))
            .left(px(6.))
            .w(px(18.))
            .h(px(18.))
            .rounded_full()
            .border_1()
            .opacity(circle_vis)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer();
        let circle = if selected {
            let p = path.clone();
            circle
                .bg(theme::ACCENT)
                .border_color(theme::ACCENT)
                .child(crate::icons::icon(crate::icons::Icon::Check, theme::ACCENT_INK, 11.0))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_select(p.clone());
                    this.anchor = Some(index);
                    cx.notify();
                }))
        } else {
            let p = path.clone();
            let c = circle
                .bg(theme::alpha(theme::BG, 0.35))
                .border_color(theme::alpha(theme::FG, 0.45 + 0.55 * hover_amt));
            // An invisible circle must not eat clicks meant for the card.
            if circle_vis > 0.0 {
                c.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_select(p.clone());
                    this.anchor = Some(index);
                    cx.notify();
                }))
            } else {
                c
            }
        };
        thumb = thumb.child(circle);

        // Hover actions: annotate / copy / delete, bottom-right.
        // They exist only while hovered, fading in with the lift.
        if hovered {
            let copy_path = entry.path.clone();
            let folder_path = entry.path.clone();
            let delete_path = entry.path.clone();
            thumb = thumb.child(
                div()
                    .absolute()
                    .bottom(px(6.))
                    .right(px(6.))
                    .flex()
                    .gap(px(4.))
                    .opacity(hover_amt)
                    .child(crate::widgets::overlay_icon_button(ElementId::NamedInteger("cpy".into(), index as u64), crate::icons::Icon::Copy).on_click(cx.listener(move |_this, _, _, cx| {
                        cx.stop_propagation();
                        let path = copy_path.clone();
                        let task = cx.background_executor()
                            .spawn(async move { pipeline::copy_image_file(&path) });
                        cx.spawn(async move |this, cx| {
                            let result = task.await;
                            let _ = this.update(cx, |this, cx| {
                                this.status = Some(
                                    result.map(|_| "copied".to_string()).unwrap_or_else(|e| e),
                                );
                                cx.notify();
                            });
                        })
                        .detach();
                    })))
                    .child(crate::widgets::overlay_icon_button(ElementId::NamedInteger("fld".into(), index as u64), crate::icons::Icon::Folder).on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        Self::open_containing_folder(&folder_path);
                    })))
                    .child(crate::widgets::overlay_icon_button(ElementId::NamedInteger("del".into(), index as u64), crate::icons::Icon::Close).on_click(cx.listener(move |_this, _, _, cx| {
                        cx.stop_propagation();
                        let path = delete_path.clone();
                        let task = cx.background_executor().spawn(async move {
                            let err = library::delete(&path).err();
                            (err, library::list())
                        });
                        cx.spawn(async move |this, cx| {
                            let (err, entries) = task.await;
                            let _ = this.update(cx, |this, cx| {
                                if let Some(e) = err {
                                    this.status = Some(e);
                                }
                                this.entries = entries;
                                this.entries_dirty = true;
                                this.entry_names = this.entries.iter().map(Self::entry_name).collect();
                                cx.notify();
                            });
                        })
                        .detach();
                    }))),
            );
        }

        let card_path = entry.path.clone();

        div()
            .id(ElementId::NamedInteger("card".into(), index as u64))
            .w(px(CARD_W))
            .flex()
            .flex_col()
            .gap(px(8.))
            .opacity(enter)
            .mt(px((1.0 - crate::motion::ease_out_cubic(enter)) * 10.0 + 1.0 * squish))
            .cursor_pointer()
            .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                if *hovering {
                    this.hovered = Some(index);
                } else if this.hovered == Some(index) {
                    this.hovered = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.click_card(index, ev, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |_, _, _, _| {
                    Self::open_containing_folder(&card_path);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.drag_start = Some((index, ev.position.x.into(), ev.position.y.into()));
                    this.drag_fired = false;
                    this.pressed = Some(index);
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.pressed.is_some() {
                        this.pressed = None;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                if ev.pressed_button != Some(MouseButton::Left) && this.pressed.is_some() {
                    this.pressed = None;
                    cx.notify();
                }
                if this.drag_fired || ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let Some((card, sx, sy)) = this.drag_start else {
                    return;
                };
                let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                if (mx - sx).hypot(my - sy) < 8.0 {
                    return;
                }
                this.drag_fired = true;
                // The index was captured at mousedown; a refresh that
                // shrank entries since then must not panic the drag.
                let Some(entry) = this.entries.get(card) else {
                    this.drag_start = None;
                    return;
                };
                let paths: Vec<PathBuf> = if this.selected.contains(&entry.path) {
                    this.selected.clone()
                } else {
                    vec![entry.path.clone()]
                };
                // The cached RenderImage already holds the pixels
                // (BGRA; the swizzle is symmetric): no disk read or
                // PNG decode on the drag-start path.
                let icon = this.thumb_cache.get(&entry.path).and_then(|img| {
                    let size = img.size(0);
                    // Fused copy+swizzle into the Arc the drag owns:
                    // the cache's BGRA bytes become the icon's RGBA in
                    // one pass.
                    let src = img.as_bytes(0)?;
                    let mut rgba = Vec::with_capacity(src.len());
                    unsafe { rgba.set_len(src.len()) };
                    rgba.copy_from_slice(src);
                    crate::widgets::swizzle_rgba_bgra(&mut rgba);
                    Some(iris_lib::dragcopy::DragIcon {
                        width: size.width.0 as u32,
                        height: size.height.0 as u32,
                        rgba: std::sync::Arc::new(rgba),
                    })
                });
                if let Err(e) = iris_lib::dragcopy::start_file_drag_at_cursor(paths, icon) {
                    this.status = Some(e);
                }
            }))
            .child(thumb)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(6.))
                    .px(px(2.))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(theme::FG)
                            .child(self.entry_names.get(index).map(|n| n.0.clone()).unwrap_or_default()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme::FG_FAINT)
                            .child(self.entry_names.get(index).map(|n| n.1.clone()).unwrap_or_default()),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    #[test]
    fn containing_folder_path_resolution() {
        let path = std::path::Path::new("/tmp/iris/captures/screenshot_01.png");
        let parent = path.parent().unwrap_or(path);
        assert_eq!(parent, std::path::Path::new("/tmp/iris/captures"));

        let root_file = std::path::Path::new("/file.png");
        let parent = root_file.parent().unwrap_or(root_file);
        assert_eq!(parent, std::path::Path::new("/"));
    }
}
