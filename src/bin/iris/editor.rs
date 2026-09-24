//! Canvas editor: markup on a capture before it leaves the machine.
//!
//! Layout: topbar (file name, undo, redo, clear; Discard, Copy text,
//! the Copy menu, Rotate, Flip, Flip V, Done, help), left sidebar
//! (eleven tools, three stroke widths, the fill toggle, eleven
//! swatches), stage with the fit-scaled image.
//! Actions live in image pixels; the stage scales them to view. Vector
//! shapes render as tessellated GPU paths, blur as pre-pixelated patch
//! images, text as shaped text elements. Saving rasterizes everything
//! CPU-side over the base and overwrites the original PNG.

use std::{
    path::PathBuf,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use gpui::*;

use crate::{motion, theme};
use iris_lib::history::Edit;

mod action;
#[cfg(test)]
mod dirty_tests;
mod keys;
mod ops;
mod paint;
mod raster;
mod raster_shapes;
mod rebuild;
mod render_chrome;
mod render_stage;
#[cfg(test)]
mod tests;

pub(crate) use action::Action;
pub(crate) use action::*;
pub(crate) use raster::*;

/// A Select-tool move drag: the action index, the last pointer in image
/// px, and the action and its bbox as they were when the drag started.
pub(crate) type MoveDrag = (usize, (f32, f32), Action, (f32, f32, f32, f32));

/// A crop-rect move drag: the pointer and the rect at drag start.
pub(crate) type CropMove = ((f32, f32), (f32, f32, f32, f32));

pub struct Editor {
    pub(crate) path: PathBuf,
    /// Display name for the topbar: the path's file name, computed
    /// once at open instead of allocating per render.
    pub(crate) filename: String,
    /// The topbar's "name · WxH" label, rebuilt only when the base
    /// image changes: formatting it per frame allocates a String a
    /// frame.
    pub(crate) title: SharedString,
    /// The unedited pixels. Arc so crop/transform can share one buffer
    /// with composite instead of cloning a full-size image per op.
    /// Until the background decode lands this is a 1x1 placeholder:
    /// its pixels are never read before base_ready, and the real
    /// dimensions live in `base_dims` so layout does not need them.
    pub(crate) base: Arc<image::RgbaImage>,
    /// The image's real pixel dimensions, known from the PNG header
    /// before the decode lands. Layout (view/fit_origin/to_image) and
    /// stroke sizing read this, never `base`'s placeholder dims.
    pub(crate) base_dims: (u32, u32),
    pub(crate) base_img: Arc<RenderImage>,
    /// CPU composite: base plus every committed action, used to sample
    /// blur patches and to rasterize the final PNG. After a crop or
    /// transform it shares base's buffer; the first mutation through
    /// Arc::make_mut splits it then (or never, on crop-then-save).
    pub(crate) composite: Arc<image::RgbaImage>,
    /// Committed actions behind Rc<RefCell>: the canvas prep closure
    /// clones a refcount per render, and edits borrow mutably in place.
    /// A plain Rc forced Rc::make_mut to deep-copy every stroke's points
    /// on each edit while the canvas held a ref.
    pub(crate) actions: Rc<std::cell::RefCell<Vec<Action>>>,
    /// Operation-level history: commits, deletes and moves are all
    /// undoable, like Markup. Two stacks; a fresh edit clears redo.
    pub(crate) undos: Vec<Edit<Action>>,
    pub(crate) redos: Vec<Edit<Action>>,
    /// In-progress action behind an Rc<RefCell>: the canvas prep
    /// closure clones a refcount per render, and the mousemove handler
    /// borrows it mutably in place. A plain Rc forced Rc::make_mut to
    /// deep-copy the stroke's points on every move of a drag.
    pub(crate) current: Option<Rc<std::cell::RefCell<Action>>>,
    pub(crate) tool: Tool,
    /// Stroke width stop 0..2; multiplies the image-relative base.
    pub(crate) stroke: u8,
    pub(crate) color: &'static str,
    pub(crate) text_entry: Option<TextEntry>,
    pub(crate) caret_started: Instant,
    /// One-shot flag for the caret blink timer: the timer notifies at
    /// 8Hz while an entry lives instead of request_animation_frame
    /// repainting the whole window at vsync for a 1Hz blink.
    pub(crate) caret_timer: bool,
    /// Select tool: the action under the cursor, and a live move drag
    /// as (action index, last pointer in image px, action as it was
    /// when the drag started, for the undo entry).
    pub(crate) selected: Option<usize>,
    pub(crate) move_drag: Option<MoveDrag>,
    /// Crop tool: the pending rect in image px, plus its drag states.
    pub(crate) crop_rect: Option<(f32, f32, f32, f32)>,
    pub(crate) crop_anchor: Option<(f32, f32)>,
    pub(crate) crop_move: Option<CropMove>,
    pub(crate) copy_menu: bool,
    pub(crate) help: bool,
    pub(crate) status: Option<String>,
    pub(crate) focus: FocusHandle,
    /// Expand-morph from a toast/library card: the image's starting
    /// rect in window coordinates. The clock starts at first render:
    /// window mapping latency would otherwise eat the animation.
    pub(crate) morph: Option<(f32, f32, f32, f32)>,
    pub(crate) morph_started: Option<Instant>,
    /// Toast handoff: signaled once a frame has provably been
    /// presented (the second render: the compositor only asks for a
    /// next frame after consuming the previous one), so the toast
    /// holding the same pixels can leave. None when not opened from
    /// a toast.
    pub(crate) morph_sync: Option<Arc<AtomicBool>>,
    pub(crate) morph_frames: u32,
    /// Outro: the whole surface fades for OUTRO, then the window
    /// goes. Save work happens before the fade starts.
    pub(crate) closing: Option<Instant>,
    /// False until the background PNG decode lands; the base and
    /// composite are transparent placeholders of the right size. Blur
    /// and save, which bake composite pixels, refuse until then.
    pub(crate) base_ready: bool,
    /// Copy dropdown entrance clock.
    pub(crate) copy_menu_opened: Option<Instant>,
    /// Zoom multiplier over the fit scale (1.0 = fit). Scroll zooms
    /// around the cursor; 0 resets.
    pub(crate) zoom: f32,
    /// Pan offset in window px, applied on top of the fit origin.
    pub(crate) pan: (f32, f32),
    /// Middle-button or space-drag pan in progress: last pointer pos.
    pub(crate) pan_drag: Option<(f32, f32)>,
    /// Space held: the stage pans instead of drawing.
    pub(crate) space_pan: bool,
    /// Rect/Ellipse fill toggle for new shapes.
    pub(crate) fill: bool,
}

/// The editor window's minimum logical size, where its resize stops.
const MIN_SIZE: Size<Pixels> = size(px(640.), px(480.));

/// Editor window default placement; morph rects arrive in screen
/// coordinates and the window grows to contain them.
const MORPH: Duration = Duration::from_millis(380);
/// Outro fade on Done/Discard.
const OUTRO: Duration = Duration::from_millis(160);

/// Window origin and dimensions for `open`, bounding against monitors
/// so the editor opens on the host monitor and never spills across edges.
fn resolve_window_placement(
    cx: &mut App,
    from: Option<(f32, f32, f32, f32)>,
) -> ((f32, f32), (f32, f32)) {
    let mut win = (1100.0f32, 720.0f32);
    let mut origin = if from.is_none() {
        crate::sys::window::centered_origin(cx, win.0, win.1, (120.0, 80.0))
    } else {
        (120.0, 80.0)
    };
    if let Some((fx, fy, fw, fh)) = from {
        // Monitors are physical pixels; `from` and window bounds are
        // logical.
        let s = crate::sys::window::root_scale(cx);
        let mons = iris_lib::capture::monitors().unwrap_or_default();
        let host = mons.iter().copied().find(|m| {
            let (mx, my) = (m.x as f32 / s, m.y as f32 / s);
            fx >= mx && fx < mx + m.width as f32 / s && fy >= my && fy < my + m.height as f32 / s
        });
        if let Some(m) = host {
            let (mx, my) = (m.x as f32 / s, m.y as f32 / s);
            let (mx1, my1) = (mx + m.width as f32 / s, my + m.height as f32 / s);
            origin = (mx + 120.0, my + 80.0);
            let x0 = origin.0.min(fx).max(mx);
            let y0 = origin.1.min(fy).max(my);
            let x1 = (origin.0 + win.0).max(fx + fw).min(mx1);
            let y1 = (origin.1 + win.1).max(fy + fh).min(my1);
            origin = (x0, y0);
            win = ((x1 - x0).max(640.0), (y1 - y0).max(480.0));
        } else {
            if fx < origin.0 {
                win.0 += origin.0 - fx;
                origin.0 = fx;
            }
            if fy < origin.1 {
                win.1 += origin.1 - fy;
                origin.1 = fy;
            }
            if fx + fw > origin.0 + win.0 {
                win.0 = fx + fw - origin.0 + 20.0;
            }
            if fy + fh > origin.1 + win.1 {
                win.1 = fy + fh - origin.1 + 20.0;
            }
        }
    }
    (origin, win)
}

/// Open an editor window for the given image file, or raise the one
/// already open on it. The window opens instantly on dimensions parsed
/// from the PNG header; the full decode runs behind it in the
/// background and swaps in. `from` is an optional starting rect for an
/// expand-morph animation (from a library card or toast), in screen
/// coordinates; `morph_sync` is an atomic signaled after the first
/// frame presents so a source toast can leave without visual pop.
pub fn open(
    cx: &mut App,
    path: &std::path::Path,
    from: Option<(f32, f32, f32, f32)>,
    morph_sync: Option<Arc<AtomicBool>>,
) -> Result<(), String> {
    // Two editors on one file would each save their own markup over the
    // other's. An editor fading out after Done or Discard is leaving.
    if crate::widgets::raise_open::<Editor>(cx, |e| e.path == path && e.closing.is_none()) {
        // A toast handing off has no morph to wait for.
        if let Some(ready) = morph_sync {
            ready.store(true, Ordering::Release);
        }
        return Ok(());
    }
    let mut header = [0u8; 24];
    {
        use std::io::Read;
        let mut f =
            std::fs::File::open(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        f.read_exact(&mut header)
            .map_err(|e| format!("read {} header: {e}", path.display()))?;
    }
    let (bw, bh) =
        png_dimensions(&header).ok_or_else(|| format!("not a PNG: {}", path.display()))?;
    // 1x1 placeholder, not a bw*bh buffer: the decode replaces it, and
    // a 33MB zeroed alloc on the UI thread is a stall for pixels that
    // are never read (base_ready gates every read; base_dims carries
    // the real dimensions for layout).
    let base = image::RgbaImage::new(1, 1);
    let base_img = crate::widgets::render_image_from_rgba(1, 1, &[0, 0, 0, 0]);
    let focus = cx.focus_handle();
    let (origin, win) = resolve_window_placement(cx, from);

    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "capture.png".to_string());
    let morph = from.map(|(x, y, w, h)| (x - origin.0, y - origin.1, w, h));
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
                app_id: Some("dev.iris.editor".to_string()),
                window_min_size: Some(MIN_SIZE),
                window_decorations: Some(WindowDecorations::Client),
                tabbing_identifier: None,
            },
            |window, cx| {
                window.set_window_title(&format!("{filename} - iris"));
                cx.new(|_| Editor {
                    title: SharedString::from(format!("{filename} · {bw}×{bh}")),
                    filename,
                    path: path.to_path_buf(),
                    base: Arc::new(base),
                    base_dims: (bw, bh),
                    base_img,
                    composite: Arc::new(image::RgbaImage::new(1, 1)),
                    actions: Rc::new(std::cell::RefCell::new(Vec::new())),
                    undos: Vec::new(),
                    redos: Vec::new(),
                    current: None,
                    tool: Tool::Pen,
                    stroke: 1,
                    color: COLORS[5],
                    text_entry: None,
                    caret_started: Instant::now(),
                    caret_timer: false,
                    selected: None,
                    move_drag: None,
                    crop_rect: None,
                    crop_anchor: None,
                    crop_move: None,
                    copy_menu: false,
                    help: false,
                    status: None,
                    focus,
                    morph,
                    morph_started: None,
                    morph_sync,
                    morph_frames: 0,
                    closing: None,
                    base_ready: false,
                    copy_menu_opened: None,
                    zoom: 1.0,
                    pan: (0.0, 0.0),
                    pan_drag: None,
                    space_pan: false,
                    fill: false,
                })
            },
        )
        .map_err(|e| format!("open editor window: {e}"))?;
    let decode_path = path.to_path_buf();
    handle
        .update(cx, |_, _, cx| {
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move {
                        let img = match crate::pipeline::take_decoded(&decode_path) {
                            Some(img) => img,
                            None => {
                                let png = std::fs::read(&decode_path)
                                    .map_err(|e| format!("read {}: {e}", decode_path.display()))?;
                                let i = image::load_from_memory(&png).map_err(|e| {
                                    format!("decode {}: {e}", decode_path.display())
                                })?;
                                Arc::new(i.to_rgba8())
                            }
                        };
                        let base = img;
                        let composite = base.clone();
                        let render = crate::widgets::render_image_from_rgba(
                            composite.width(),
                            composite.height(),
                            composite.as_raw(),
                        );
                        Ok::<
                            (
                                Arc<image::RgbaImage>,
                                Arc<image::RgbaImage>,
                                Arc<gpui::RenderImage>,
                            ),
                            String,
                        >((base, composite, render))
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    match decoded {
                        Ok((base, composite, render)) => {
                            let old = std::mem::replace(&mut this.base_img, render);
                            crate::widgets::release_render(&old, cx);
                            this.base = base;
                            this.base_dims = (this.base.width(), this.base.height());
                            this.title = SharedString::from(format!(
                                "{} · {}×{}",
                                this.filename, this.base_dims.0, this.base_dims.1
                            ));
                            this.composite = composite;
                            this.base_ready = true;
                            if !this.actions.borrow().is_empty() {
                                this.rebuild_all();
                            }
                        }
                        Err(e) => {
                            this.status = Some(format!("decode: {e}"));
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        })
        .map_err(|e| format!("editor decode: {e}"))?;
    Ok(())
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus.focus(window);
        if let Some(ready) = &self.morph_sync {
            self.morph_frames += 1;
            if self.morph_frames >= 2 {
                ready.store(true, Ordering::Release);
            } else {
                window.request_animation_frame();
            }
        }
        let (ox, oy, scale) = self.view(window);
        let vw = self.base.width() as f32 * scale;
        let vh = self.base.height() as f32 * scale;

        let mut stage_rect = (ox, oy, vw, vh);
        let mut chrome = 1.0f32;
        let mut topbar = 1.0f32;
        let mut sidebar = 1.0f32;
        let mut morph_radius = 6.0f32;
        if let Some(from) = self.morph {
            let started = *self.morph_started.get_or_insert_with(Instant::now);
            let t = (started.elapsed().as_secs_f32() / motion::tempo(MORPH).as_secs_f32()).min(1.0);
            let e = motion::spring(t);
            stage_rect = (
                from.0 + (ox - from.0) * e,
                from.1 + (oy - from.1) * e,
                from.2 + (vw - from.2) * e,
                from.3 + (vh - from.3) * e,
            );
            chrome = (t * 2.2).min(1.0);
            topbar = ((t - 0.08) * 2.2).clamp(0.0, 1.0);
            sidebar = ((t - 0.18) * 2.2).clamp(0.0, 1.0);
            morph_radius = 12.0 - 6.0 * e;
            if t >= 1.0 {
                self.morph = None;
                self.morph_started = None;
            } else {
                window.request_animation_frame();
            }
        }

        let mut outro = 1.0f32;
        if let Some(started) = self.closing {
            let t = (started.elapsed().as_secs_f32() / motion::tempo(OUTRO).as_secs_f32()).min(1.0);
            outro = 1.0 - t;
            if t >= 1.0 {
                crate::widgets::release_render(&self.base_img, cx);
                window.remove_window();
            } else {
                window.request_animation_frame();
            }
        }
        let title = self.title.clone();
        let stage = self.render_stage(stage_rect, morph_radius, chrome, scale, window, cx);

        // Paint order is stacking order: the zoomed or panned image
        // runs under the floating chrome, never over it.
        let root = div()
            .id("editor")
            .size_full()
            .font_family(theme::FONT)
            .bg(theme::alpha(theme::BG, chrome))
            .opacity(outro)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_key_down(ev, window, cx);
            }))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| {
                this.handle_key_up(ev, cx);
            }))
            .child(self.render_backdrop(chrome, cx))
            .child(stage)
            .child(self.render_topbar(topbar, title, window, cx))
            .child(self.render_sidebar(sidebar, cx));

        self.render_menus_and_overlays(root, topbar, window, cx)
            .children(crate::widgets::resize_edges(window, MIN_SIZE))
    }
}
