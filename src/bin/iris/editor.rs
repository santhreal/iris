//! Canvas editor: markup on a capture before it leaves the machine.
//!
//! Layout mirrors the Tauri editor one for one: topbar (file info,
//! Discard, Copy text, Copy variants, Done), left sidebar (eight tools,
//! undo, clear, eleven swatches), stage with the fit-scaled image.
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

use crate::{icons, motion, pipeline, theme};
use iris_lib::history::{self, Edit};
use icons::{push_disc, push_ellipse_fill, push_filled_triangle, push_rect_fill, push_ring, push_segment, Icon};

/// Markup colors are user content, not UI chrome: the monochrome
/// register governs the interface, the palette governs what you draw.
const COLORS: [&str; 11] = [
    "#f5f5f7", "#9c9ca2", "#6b6b71", "#3a3a3f", "#141416", "#ff453a", "#ff9f0a", "#ffd60a",
    "#30d158", "#0a84ff", "#bf5af2",
];

const BLUR_BLOCK: u32 = 12;

fn hex_rgba(hex: &str) -> Rgba {
    let v = u32::from_str_radix(&hex[1..], 16).unwrap_or(0xffffff);
    Rgba {
        r: ((v >> 16) & 0xff) as f32 / 255.0,
        g: ((v >> 8) & 0xff) as f32 / 255.0,
        b: (v & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

/// Whole-image transforms (rotate/flip), applied to base+composite.
#[derive(Clone, Copy)]
enum Transform {
    Rot90,
    FlipH,
    FlipV,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Select,
    Pen,
    Line,
    Arrow,
    Ellipse,
    Rect,
    Text,
    Highlight,
    Blur,
    Crop,
    Counter,
}
const TOOLS: [(Tool, Icon, &str); 11] = [
    (Tool::Select, Icon::Cursor, "Select"),
    (Tool::Pen, Icon::Pen, "Pen"),
    (Tool::Line, Icon::Line, "Line"),
    (Tool::Arrow, Icon::Arrow, "Arrow"),
    (Tool::Ellipse, Icon::Ellipse, "Ellipse"),
    (Tool::Rect, Icon::Rect, "Rectangle"),
    (Tool::Text, Icon::Text, "Text"),
    (Tool::Highlight, Icon::Highlight, "Highlight"),
    (Tool::Blur, Icon::Blur, "Blur"),
    (Tool::Crop, Icon::Crop, "Crop"),
    (Tool::Counter, Icon::Counter, "Counter"),
];

#[derive(Clone)]
struct Action {
    tool: Tool,
    color: &'static str,
    width: f32,
    points: Vec<(f32, f32)>,
    text: Option<SharedString>,
    font_size: f32,
    /// Rect/Ellipse fill: when set the shape paints its interior, not
    /// just the outline.
    filled: bool,
    /// Pixelated patch for blur actions, computed at commit time from
    /// the composite below this action.
    blur_patch: Option<Arc<RenderImage>>,
    blur_rect: (f32, f32, f32, f32),
    /// Counter tool: the step number shown in the badge.
    step: u32,
    /// Its display string, cached at commit: to_string() per counter
    /// per frame was an allocation a frame.
    step_label: SharedString,
    /// Bounding box in image px, computed at commit and translated by
    /// move drags: hit-testing every committed action per click and
    /// the selected outline per frame would otherwise rescan every
    /// stroke's points each time.
    bbox: Option<(f32, f32, f32, f32)>,
    /// The tessellated paint triangles at stage origin (0,0), keyed on
    /// (scale, points fingerprint): building them per frame
    /// re-tessellates 20-vertex discs for every segment of every
    /// stroke, while a re-stamp is one offset per vertex. Origin is
    /// not part of the key, so a pan drag reuses the same geometry
    /// instead of re-tessellating every action every frame. Painted
    /// on the UI thread only, so a RefCell suffices. The fingerprint
    /// (len, first, last) catches every mutation that exists: strokes
    /// only grow, moves shift every point.
    cached_path: std::cell::RefCell<
        Option<(f32, usize, (f32, f32), (f32, f32), Rc<Vec<[f32; 6]>>)>,
    >,
}

struct TextEntry {
    point: (f32, f32),
    buffer: String,
    /// buffer + caret glyph, rebuilt on each edit: formatting it per
    /// render allocates a String a frame while the caret blinks.
    caret: SharedString,
}

pub struct Editor {
    path: PathBuf,
    /// Display name for the topbar: the path's file name, computed
    /// once at open instead of allocating per render.
    filename: String,
    /// The topbar's "name · WxH" label, rebuilt only when the base
    /// image changes: formatting it per frame allocates a String a
    /// frame.
    title: SharedString,
    base: image::RgbaImage,
    base_img: Arc<RenderImage>,
    /// CPU composite: base plus every committed action, used to sample
    /// blur patches and to rasterize the final PNG.
    composite: image::RgbaImage,
    /// Committed actions behind Rc<RefCell>: the canvas prep closure
    /// clones a refcount per render, and edits borrow mutably in place.
    /// A plain Rc forced Rc::make_mut to deep-copy every stroke's points
    /// on each edit while the canvas held a ref.
    actions: Rc<std::cell::RefCell<Vec<Action>>>,
    /// Operation-level history: commits, deletes and moves are all
    /// undoable, like Markup. Two stacks; a fresh edit clears redo.
    undos: Vec<Edit<Action>>,
    redos: Vec<Edit<Action>>,
    /// In-progress action behind an Rc<RefCell>: the canvas prep
    /// closure clones a refcount per render, and the mousemove handler
    /// borrows it mutably in place. A plain Rc forced Rc::make_mut to
    /// deep-copy the stroke's points on every move of a drag.
    current: Option<Rc<std::cell::RefCell<Action>>>,
    tool: Tool,
    /// Stroke width stop 0..2; multiplies the image-relative base.
    stroke: u8,
    color: &'static str,
    text_entry: Option<TextEntry>,
    caret_started: Instant,
    /// One-shot flag for the caret blink timer: the timer notifies at
    /// 8Hz while an entry lives instead of request_animation_frame
    /// repainting the whole window at vsync for a 1Hz blink.
    caret_timer: bool,
    /// Select tool: the action under the cursor, and a live move drag
    /// as (action index, last pointer in image px, action as it was
    /// when the drag started, for the undo entry).
    selected: Option<usize>,
    move_drag: Option<(usize, (f32, f32), Action, (f32, f32, f32, f32))>,
    /// Crop tool: the pending rect in image px, plus its drag states.
    crop_rect: Option<(f32, f32, f32, f32)>,
    crop_anchor: Option<(f32, f32)>,
    crop_move: Option<((f32, f32), (f32, f32, f32, f32))>,
    copy_menu: bool,
    help: bool,
    status: Option<String>,
    focus: FocusHandle,
    /// Expand-morph from a toast/library card: the image's starting
    /// rect in window coordinates. The clock starts at first render:
    /// window mapping latency would otherwise eat the animation.
    morph: Option<(f32, f32, f32, f32)>,
    morph_started: Option<Instant>,
    /// Toast handoff: signaled once a frame has provably been
    /// presented (the second render: the compositor only asks for a
    /// next frame after consuming the previous one), so the toast
    /// holding the same pixels can leave. None when not opened from
    /// a toast.
    morph_sync: Option<Arc<AtomicBool>>,
    morph_frames: u32,
    /// Outro: the whole surface fades for OUTRO, then the window
    /// goes. Save work happens before the fade starts.
    closing: Option<Instant>,
    /// False until the background PNG decode lands; the base and
    /// composite are transparent placeholders of the right size. Blur
    /// and save, which bake composite pixels, refuse until then.
    base_ready: bool,
    /// Copy dropdown entrance clock.
    copy_menu_opened: Option<Instant>,
    /// Zoom multiplier over the fit scale (1.0 = fit). Scroll zooms
    /// around the cursor; 0 resets.
    zoom: f32,
    /// Pan offset in window px, applied on top of the fit origin.
    pan: (f32, f32),
    /// Middle-button or space-drag pan in progress: last pointer pos.
    pan_drag: Option<(f32, f32)>,
    /// Space held: the stage pans instead of drawing.
    space_pan: bool,
    /// Rect/Ellipse fill toggle for new shapes.
    fill: bool,
}

/// Editor window default placement; morph rects arrive in screen
/// coordinates and the window grows to contain them.
const MORPH: Duration = Duration::from_millis(380);
/// Outro fade on Done/Discard.
const OUTRO: Duration = Duration::from_millis(160);

/// Width and height from the PNG IHDR, without decoding. The full
/// decode runs in the background; the window opens on these dims.
fn png_dimensions(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    Some((
        u32::from_be_bytes(png[16..20].try_into().ok()?),
        u32::from_be_bytes(png[20..24].try_into().ok()?),
    ))
}

/// Open the editor for an existing capture PNG. `from` is the screen
/// rect of the card the user expanded from; the image springs out of
/// it into the stage layout.
pub fn open(
    cx: &mut App,
    path: &std::path::Path,
    from: Option<(f32, f32, f32, f32)>,
    morph_sync: Option<Arc<AtomicBool>>,
) -> Result<(), String> {
    // Only the IHDR is needed for the window's size: a noisy 4K PNG
    // is tens of MB, and reading it whole on the UI thread stalls the
    // click that opened us. The full read + decode runs behind the
    // open window; GPUI renders the shared PNG bytes directly.
    let mut header = [0u8; 24];
    {
        use std::io::Read;
        let mut f = std::fs::File::open(path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        f.read_exact(&mut header)
            .map_err(|e| format!("read {} header: {e}", path.display()))?;
    }
    let (bw, bh) = png_dimensions(&header)
        .ok_or_else(|| format!("not a PNG: {}", path.display()))?;
    let base = image::RgbaImage::new(bw, bh);
    // Blank until the background decode lands; previously GPUI's own
    // async decode of these PNG bytes filled the same gap.
    let base_img = crate::widgets::render_image_from_rgba(1, 1, &[0, 0, 0, 0]);
    let focus = cx.focus_handle();
    // The window must contain the morph's start rect, or the expansion
    // clips at the window edge before it reads as flight.
    let mut win = (1100.0f32, 720.0f32);
    // No source card: the window opens centered on the primary display.
    let mut origin = if from.is_none() {
        crate::xwin::centered_origin(cx, win.0, win.1, (120.0, 80.0))
    } else {
        (120.0, 80.0)
    };
    if let Some((fx, fy, fw, fh)) = from {
        // The card lives on one monitor; the editor opens on that
        // monitor and never spills onto a neighbor. A window stretched
        // across the virtual screen reads as a bug, not as flight.
        #[cfg(target_os = "linux")]
        let mons = iris_lib::capture::x11::monitors().unwrap_or_default();
        #[cfg(not(target_os = "linux"))]
        let mons: Vec<iris_lib::capture::WinRect> = Vec::new();
        let host = mons.iter().copied().find(|m| {
            fx >= m.x as f32
                && fx < (m.x + m.width as i32) as f32
                && fy >= m.y as f32
                && fy < (m.y + m.height as i32) as f32
        });
        if let Some(m) = host {
            let (mx, my) = (m.x as f32, m.y as f32);
            let (mx1, my1) = (mx + m.width as f32, my + m.height as f32);
            origin = (mx + 120.0, my + 80.0);
            // Contain the card, clipped to the monitor's own rect.
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
    let win_id = crate::xwin::unique_id("dev.iris.editor");
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
            // Transparent so the window can materialize around the
            // morphing image instead of snapping in fully opaque.
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some(win_id.clone()),
            window_min_size: Some(size(px(640.), px(480.))),
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
        },
        |_, cx| {
            cx.new(|_| {
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "capture.png".to_string());
                Editor {
                title: SharedString::from(format!(
                    "{filename} · {}×{}",
                    base.width(),
                    base.height()
                )),
                filename,
                path: path.to_path_buf(),
                // 1x1 until the decode lands: cloning the zeroed base
                // here allocated a full-size buffer the decode task
                // replaces unread. Every composite read is gated on
                // base_ready or follows the assignment in the task.
                composite: image::RgbaImage::new(1, 1),
                base,
                base_img,
                actions: Rc::new(std::cell::RefCell::new(Vec::new())),
                undos: Vec::new(),
                redos: Vec::new(),
                current: None,
                tool: Tool::Pen,
                stroke: 1,
                color: COLORS[0],
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
                }
            })
        },
    )
    .map_err(|e| format!("open editor window: {e}"))?;
    // Decode the base behind the open window. Vector actions drawn in
    // the gap replay onto the real pixels via rebuild_all; blur and
    // save, which bake composite pixels, refuse until this lands.
    // The RenderImage is built in the same task: its copy+swizzle of
    // the decoded frame is a 33MB memcpy at 4K, too big for the UI
    // thread while the morph is mid-flight.
    let decode_path = path.to_path_buf();
    handle
        .update(cx, |_, _, cx| {
            cx.spawn(async move |this, cx| {
                let decoded = cx
                    .background_executor()
                    .spawn(async move {
                        let png = std::fs::read(&decode_path).map_err(|e| {
                            format!("read {}: {e}", decode_path.display())
                        })?;
                        let i = image::load_from_memory(&png)
                            .map_err(|e| format!("decode {}: {e}", decode_path.display()))?;
                        let img = i.to_rgba8();
                        // Clone for `base` here, off the UI thread:
                        // a 4K memcpy on the main thread stalls the
                        // morph that is mid-flight when this lands.
                        let base = img.clone();
                        let render = crate::widgets::render_image_from_rgba(
                            img.width(),
                            img.height(),
                            img.as_raw(),
                        );
                        Ok::<(image::RgbaImage, image::RgbaImage, Arc<gpui::RenderImage>), String>(
                            (img, base, render),
                        )
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    match decoded {
                        Ok((img, base, render)) => {
                            let old = std::mem::replace(&mut this.base_img, render);
                            crate::widgets::release_render(&old, cx);
                            this.base = base;
                            this.title = SharedString::from(format!(
                                "{} · {}×{}",
                                this.filename,
                                this.base.width(),
                                this.base.height()
                            ));
                            this.composite = img;
                            this.base_ready = true;
                            // composite already holds base's pixels;
                            // the 33MB restore+replay only matters
                            // when strokes drawn before the decode
                            // landed need baking in.
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

/// Stroke, highlight and text scale with the capture's pixel size, so a
/// stroke reads the same on a 400px region grab and a 5K full-screen shot.
/// Stroke width stops: multipliers over the image-relative base.
const STROKE_MULT: [f32; 3] = [0.6, 1.0, 1.8];

fn stroke_base(w: u32) -> f32 {
    (w as f32 * 0.004).round().max(3.0)
}
fn highlight_width(w: u32) -> f32 {
    (w as f32 * 0.019).round().max(14.0)
}
fn text_size(w: u32) -> f32 {
    (w as f32 * 0.026).round().max(20.0)
}

use image::ImageEncoder as _;

fn png_bytes(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    // Fast deflate, same as the capture path: Balanced is ~3x slower
    // on a 4K frame and a screenshot's redundancy compresses well
    // either way.
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| format!("encode png: {e}"))?;
    Ok(out.into_inner())
}

impl Editor {
    fn img_w(&self) -> u32 {
        self.base.width()
    }

    fn new_action(&self, p: (f32, f32)) -> Action {
        let step = self.next_step();
        Action {
            tool: self.tool,
            color: self.color,
            width: if self.tool == Tool::Highlight {
                highlight_width(self.img_w())
            } else {
                stroke_base(self.img_w()) * STROKE_MULT[self.stroke as usize]
            },
            points: vec![p],
            text: None,
            font_size: text_size(self.img_w()),
            filled: self.fill,
            blur_patch: None,
            blur_rect: (0.0, 0.0, 0.0, 0.0),
            step,
            step_label: SharedString::from(step.to_string()),
            bbox: None,
            cached_path: std::cell::RefCell::new(None),
        }
    }

    /// The next counter number: one past the highest committed step.
    fn next_step(&self) -> u32 {
        self.actions
            .borrow()
            .iter()
            .filter(|a| a.tool == Tool::Counter)
            .map(|a| a.step)
            .max()
            .unwrap_or(0)
            + 1
    }

    fn commit_current(&mut self) {
        let Some(rc) = self.current.take() else {
            return;
        };
        // Between frames the paint closure has dropped its clone, so
        // this unwraps without copying; a mid-frame commit clones once.
        let mut action = Rc::try_unwrap(rc)
            .map(std::cell::RefCell::into_inner)
            .unwrap_or_else(|rc| rc.borrow().clone());
        if action.points.len() < 2
            && action.tool != Tool::Pen
            && action.tool != Tool::Highlight
            && action.tool != Tool::Counter
        {
            return;
        }
        if action.tool == Tool::Blur {
            if !self.base_ready {
                // The composite is still the placeholder; a blur would
                // bake transparent pixels into the patch.
                self.status = Some("still decoding".into());
                return;
            }
            let p0 = action.points[0];
            let p1 = action.points[action.points.len() - 1];
            let x = p0.0.min(p1.0).max(0.0) as u32;
            let y = p0.1.min(p1.1).max(0.0) as u32;
            let r = p0.0.max(p1.0).min(self.base.width() as f32) as u32;
            let b = p0.1.max(p1.1).min(self.base.height() as f32) as u32;
            let (w, h) = (r.saturating_sub(x), b.saturating_sub(y));
            if w == 0 || h == 0 {
                return;
            }
            match pixelated_patch_rgba(&self.composite, x, y, w, h) {
                Ok(patch) => {
                    // One resample serves both consumers: the GPU tile
                    // the canvas shows and the CPU pixelation later
                    // blurs sample through. Computing it twice per
                    // commit was two crop+resize+resize chains.
                    let render = crate::widgets::render_image_from_rgba(w, h, patch.as_raw());
                    image::imageops::overlay(&mut self.composite, &patch, x as i64, y as i64);
                    action.blur_patch = Some(render);
                    action.blur_rect = (x as f32, y as f32, w as f32, h as f32);
                }
                Err(_) => return,
            }
        } else {
            rasterize(&mut self.composite, &action, 1.0);
        }
        action.bbox = Self::compute_bbox(&action);
        self.push_edit(Edit::Add(action.clone()));
        self.actions.borrow_mut().push(action);
        self.selected = None;
    }

    fn push_edit(&mut self, edit: Edit<Action>) {
        self.undos.push(edit);
        self.redos.clear();
    }

    fn apply_forward(&mut self, edit: &Edit<Action>) {
        history::apply_forward(&mut self.actions.borrow_mut(), edit);
    }

    fn apply_inverse(&mut self, edit: &Edit<Action>) {
        history::apply_inverse(&mut self.actions.borrow_mut(), edit);
    }

    fn undo(&mut self) {
        if let Some(edit) = self.undos.pop() {
            self.apply_inverse(&edit);
            self.selected = None;
            self.rebuild_for_edit(&edit);
            self.redos.push(edit);
        }
    }

    fn redo(&mut self) {
        if let Some(edit) = self.redos.pop() {
            self.apply_forward(&edit);
            self.selected = None;
            self.rebuild_for_edit(&edit);
            self.undos.push(edit);
        }
    }

    fn clear(&mut self) {
        self.actions.borrow_mut().clear();
        self.undos.clear();
        self.redos.clear();
        self.selected = None;
        self.rebuild_all();
    }

/// Replay actions `skip..` onto `composite` in commit order: blur
/// pixelates whatever is beneath it at that point, so later blurs
/// sample through earlier ones, matching the canvas2d implementation.
/// Patches are recomputed here so moved or cropped blurs sample their
/// new location. Associated function so the partial-replay path is
/// testable without a Window.
fn replay_actions(
    composite: &mut image::RgbaImage,
    actions: &mut [Action],
    skip: usize,
) {
    for action in actions.iter_mut().skip(skip) {
        Self::replay_one(composite, action);
    }
}

/// Replay one action onto `composite`. Blur resamples the pixels
/// under its rect at this point in the order, so a moved or cropped
/// blur pixelates its new location.
fn replay_one(composite: &mut image::RgbaImage, action: &mut Action) {
    if action.tool == Tool::Blur {
        let (x, y) = (
            action.blur_rect.0.max(0.0) as u32,
            action.blur_rect.1.max(0.0) as u32,
        );
        let (w, h) = (action.blur_rect.2 as u32, action.blur_rect.3 as u32);
        if w >= 1 && h >= 1 && x < composite.width() && y < composite.height() {
            let w = w.min(composite.width() - x);
            let h = h.min(composite.height() - y);
            // One resample serves the GPU tile and the CPU
            // pixelation; computing it twice per replayed blur
            // was two crop+resize+resize chains.
            if let Ok(patch) = pixelated_patch_rgba(composite, x, y, w, h) {
                let render = crate::widgets::render_image_from_rgba(w, h, patch.as_raw());
                image::imageops::overlay(composite, &patch, x as i64, y as i64);
                action.blur_patch = Some(render);
            }
        }
    } else {
        rasterize(composite, action, 1.0);
    }
}

/// Copy the (x, y, w, h) rect of `base` over `composite`, row by row:
/// the dirty rebuild restores only the region an edit touched instead
/// of the whole frame.
fn restore_region(
    base: &image::RgbaImage,
    composite: &mut image::RgbaImage,
    (x, y, w, h): (u32, u32, u32, u32),
) {
    let stride = base.width() as usize * 4;
    let src = base.as_raw();
    let dst: &mut [u8] = composite.as_mut();
    for row in 0..h as usize {
        let off = (y as usize + row) * stride + x as usize * 4;
        dst[off..off + w as usize * 4].copy_from_slice(&src[off..off + w as usize * 4]);
    }
}

/// The rect an action can paint or sample, in image pixels: its
/// stored bbox, padded by the font size for text and counters whose
/// ink overflows the estimate. `None` means the action could have
/// painted anywhere and forces a full rebuild.
fn footprint(action: &Action) -> Option<(f32, f32, f32, f32)> {
    let (x, y, w, h) = action.bbox?;
    let pad = if matches!(action.tool, Tool::Text | Tool::Counter) {
        action.font_size
    } else {
        0.0
    };
    Some((x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad))
}

/// Do two (x, y, w, h) rects overlap?
fn intersects(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
    a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
}

/// Grow `region` until it covers the footprint of every action that
/// intersects it, and return the replay mask. A replayed action
/// repaints its whole footprint: pixels outside the region keep the
/// old ink, so repainting there compounds alpha, and a blur samples
/// stale composite. Any action whose footprint intersects the region
/// must therefore be replayed, which pulls its footprint into the
/// region, which can pull in further actions. Iterate to the fixpoint;
/// a footprint-less action could have painted anywhere, so it widens
/// the region to the whole image.
fn replay_closure(
    actions: &[Action],
    mut region: (f32, f32, f32, f32),
    iw: f32,
    ih: f32,
) -> ((f32, f32, f32, f32), Vec<bool>) {
    let mut mark = vec![false; actions.len()];
    loop {
        let mut grew = false;
        for (i, a) in actions.iter().enumerate() {
            if mark[i] {
                continue;
            }
            match Self::footprint(a) {
                Some(f) if Self::intersects(f, region) => {
                    mark[i] = true;
                    // Grow only when the footprint spills outside the
                    // region; a contained footprint changes nothing.
                    if f.0 < region.0
                        || f.1 < region.1
                        || f.0 + f.2 > region.0 + region.2
                        || f.1 + f.3 > region.1 + region.3
                    {
                        region = Self::union_rect(region, f);
                    }
                    grew = true;
                }
                Some(_) => {}
                None => {
                    mark[i] = true;
                    region = (0.0, 0.0, iw, ih);
                    grew = true;
                }
            }
        }
        if !grew {
            return (region, mark);
        }
    }
}

/// Clamp a float (x, y, w, h) region to the image, as pixel bounds.
fn clamp_region(
    region: (f32, f32, f32, f32),
    iw: u32,
    ih: u32,
) -> Option<(u32, u32, u32, u32)> {
    let x0 = (region.0.max(0.0) as u32).min(iw);
    let y0 = (region.1.max(0.0) as u32).min(ih);
    let x1 = ((region.0 + region.2).ceil().max(0.0) as u32).min(iw);
    let y1 = ((region.1 + region.3).ceil().max(0.0) as u32).min(ih);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

    fn rebuild_all(&mut self) {
        // Replay in commit order: blur pixelates whatever is beneath it
        // at that point, so later blurs sample through earlier ones,
        // matching the canvas2d implementation. Patches are recomputed
        // here so moved or cropped blurs sample their new location.
        // Reuse the composite buffer: it is always the same size as base,
        // so replay writes into it instead of cloning a fresh image.
        self.composite.copy_from_slice(&self.base);
        Self::replay_actions(&mut self.composite, &mut self.actions.borrow_mut(), 0);
    }
    /// The union of two (x, y, w, h) rects.
    fn union_rect(
        a: (f32, f32, f32, f32),
        b: (f32, f32, f32, f32),
    ) -> (f32, f32, f32, f32) {
        let (x, y) = (a.0.min(b.0), a.1.min(b.1));
        (x, y, (a.0 + a.2).max(b.0 + b.2) - x, (a.1 + a.3).max(b.1 + b.3) - y)
    }

    /// Plan a partial rebuild: renumber counters (steps are
    /// commit-order across every counter, so this runs over all
    /// actions even when the replay is partial), seed the dirty
    /// region with the footprints of counters whose number changed
    /// (they paint different ink), then close the region over every
    /// action it touches. Returns the closed region and replay mask.
    fn dirty_plan(
        actions: &mut [Action],
        region: (f32, f32, f32, f32),
        iw: f32,
        ih: f32,
    ) -> ((f32, f32, f32, f32), Vec<bool>) {
        let mut step = 0u32;
        let mut seeded = region;
        for action in actions.iter_mut() {
            if action.tool == Tool::Counter {
                step += 1;
                if action.step != step {
                    action.step = step;
                    action.step_label = SharedString::from(step.to_string());
                    if let Some(f) = Self::footprint(action) {
                        seeded = Self::union_rect(seeded, f);
                    }
                }
            }
        }
        Self::replay_closure(actions, seeded, iw, ih)
    }

    /// Apply a plan from `dirty_plan`: restore the closed region from
    /// base, then replay the marked actions in commit order.
    fn apply_plan(
        base: &image::RgbaImage,
        composite: &mut image::RgbaImage,
        actions: &mut [Action],
        closed: (f32, f32, f32, f32),
        mark: &[bool],
    ) {
        let Some(r) = Self::clamp_region(closed, composite.width(), composite.height()) else {
            return;
        };
        Self::restore_region(base, composite, r);
        for (i, action) in actions.iter_mut().enumerate() {
            if mark[i] {
                Self::replay_one(composite, action);
            }
        }
    }

    /// Rebuild only the pixels an edit touched. See `replay_closure`
    /// for why the region must cover every replayed footprint.
    fn rebuild_dirty(&mut self, region: (f32, f32, f32, f32)) {
        let (iw, ih) = (self.composite.width(), self.composite.height());
        let (closed, mark) = Self::dirty_plan(
            &mut self.actions.borrow_mut(),
            region,
            iw as f32,
            ih as f32,
        );
        Self::apply_plan(
            &self.base,
            &mut self.composite,
            &mut self.actions.borrow_mut(),
            closed,
            &mark,
        );
    }
    fn rebuild_for_edit(&mut self, edit: &Edit<Action>) {
        let region = match edit {
            Edit::Add(a) | Edit::Remove(_, a) => a.bbox,
            Edit::Move(_, old, new) => match (old.bbox, new.bbox) {
                (Some(a), Some(b)) => Some(Self::union_rect(a, b)),
                _ => None,
            },
        };
        match region {
            Some(r) => self.rebuild_dirty(r),
            None => self.rebuild_all(),
        }
    }
    /// Bounding box of one action in image pixels, computed at commit.
    /// Stored on the action: hit-testing and the selection outline
    /// read it instead of rescanning every point.
    fn compute_bbox(action: &Action) -> Option<(f32, f32, f32, f32)> {
        match action.tool {
            Tool::Text => {
                let p = action.points.first()?;
                let text = action.text.as_ref()?;
                let w = text.chars().count() as f32 * action.font_size * 0.6;
                Some((p.0, p.1 - action.font_size, w.max(8.0), action.font_size * 1.25))
            }
            Tool::Blur => Some(action.blur_rect),
            Tool::Counter => {
                let p = action.points.first()?;
                let r = action.font_size * 0.9;
                Some((p.0 - r, p.1 - r, r * 2.0, r * 2.0))
            }
            _ => {
                if action.points.is_empty() {
                    return None;
                }
                let (mut x0, mut y0, mut x1, mut y1) =
                    (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for &(x, y) in &action.points {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
                let pad = action.width / 2.0 + 2.0;
                Some((x0 - pad, y0 - pad, (x1 - x0) + 2.0 * pad, (y1 - y0) + 2.0 * pad))
            }
        }
    }

    /// Topmost action whose bbox contains `p`, for the Select tool.
    fn hit_action(&self, p: (f32, f32)) -> Option<usize> {
        for (i, action) in self.actions.borrow().iter().enumerate().rev() {
            let Some((x, y, w, h)) = action.bbox else {
                continue;
            };
            if p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h {
                return Some(i);
            }
        }
        None
    }

    /// Translate one action, clamped so its bbox stays on the image.
    /// `bbox` is the action's current bounds, tracked by the caller:
    /// recomputing it from the points every mousemove is O(stroke)
    /// per move for a value the drag already knows.
    fn move_action(&mut self, i: usize, d: (f32, f32), bbox: (f32, f32, f32, f32)) -> (f32, f32) {
        let (x, y, w, h) = bbox;
        let (iw, ih) = (self.base.width() as f32, self.base.height() as f32);
        let dx = d.0.clamp(-x, iw - (x + w));
        let dy = d.1.clamp(-y, ih - (y + h));
        let mut actions = self.actions.borrow_mut();
        let Some(action) = actions.get_mut(i) else {
            return (0.0, 0.0);
        };
        for p in &mut action.points {
            p.0 += dx;
            p.1 += dy;
        }
        // Keep the tessellation cache valid through the drag: the
        // cached triangles are stage-space (image * scale), so the
        // same delta scaled applies, and first/last shift with the
        // points. Without this every mousemove re-tessellated the
        // action being dragged.
        if let Some((kscale, _, kfirst, klast, tris)) =
            action.cached_path.borrow_mut().as_mut()
        {
            let (sx, sy) = (dx * *kscale, dy * *kscale);
            for t in Rc::make_mut(tris).iter_mut() {
                t[0] += sx;
                t[1] += sy;
                t[2] += sx;
                t[3] += sy;
                t[4] += sx;
                t[5] += sy;
            }
            *kfirst = (kfirst.0 + dx, kfirst.1 + dy);
            *klast = (klast.0 + dx, klast.1 + dy);
        }
        if action.tool == Tool::Blur {
            action.blur_rect.0 += dx;
            action.blur_rect.1 += dy;
        }
        if let Some(bb) = &mut action.bbox {
            bb.0 += dx;
            bb.1 += dy;
        }
        (dx, dy)
    }

    /// Apply the pending crop: everything committed flattens into the
    /// base (a crop re-keys every coordinate system; keeping actions
    /// editable across it is not worth the transform bugs), then the
    /// image is cut to the rect.
    fn apply_crop(&mut self, cx: &mut Context<Self>) {
        if !self.base_ready {
            self.crop_rect = None;
            self.status = Some("still decoding".into());
            return;
        }
        let Some((x, y, w, h)) = self.crop_rect.take() else {
            return;
        };
        self.commit_text(true);
        self.commit_current();
        self.rebuild_all();
        let (x, y) = (x.max(0.0) as u32, y.max(0.0) as u32);
        if x >= self.composite.width() || y >= self.composite.height() {
            return;
        }
        let w = (w as u32).min(self.composite.width() - x);
        let h = (h as u32).min(self.composite.height() - y);
        if w < 8 || h < 8 {
            return;
        }
        let cropped = image::imageops::crop_imm(&self.composite, x, y, w, h).to_image();
        let old = std::mem::replace(
            &mut self.base_img,
            crate::widgets::render_image_from_rgba(w, h, cropped.as_raw()),
        );
        crate::widgets::release_render(&old, cx);
        self.base = cropped.clone();
        self.title = SharedString::from(format!(
            "{} · {}×{}",
            self.filename,
            self.base.width(),
            self.base.height()
        ));
        self.composite = cropped;
        self.actions.borrow_mut().clear();
        self.undos.clear();
        self.redos.clear();
        self.selected = None;
    }

    /// Rotate the whole image 90° CW, or mirror it horizontally.
    /// Like a crop, this re-keys every coordinate system, so committed
    /// actions flatten into the base first.
    fn transform(&mut self, op: Transform, cx: &mut Context<Self>) {
        if !self.base_ready {
            self.status = Some("still decoding".into());
            return;
        }
        self.commit_text(true);
        self.commit_current();
        self.rebuild_all();
        let out = match op {
            Transform::Rot90 => image::imageops::rotate90(&self.composite),
            Transform::FlipH => image::imageops::flip_horizontal(&self.composite),
            Transform::FlipV => image::imageops::flip_vertical(&self.composite),
        };
        let (w, h) = (out.width(), out.height());
        let old = std::mem::replace(
            &mut self.base_img,
            crate::widgets::render_image_from_rgba(w, h, out.as_raw()),
        );
        crate::widgets::release_render(&old, cx);
        self.base = out.clone();
        self.title = SharedString::from(format!(
            "{} · {}×{}",
            self.filename,
            self.base.width(),
            self.base.height()
        ));
        self.composite = out;
        self.actions.borrow_mut().clear();
        self.undos.clear();
        self.redos.clear();
        self.selected = None;
        cx.notify();
    }

    /// Switch tools from a hotkey or the sidebar: settle any open text
    /// entry first so a typed label is never dropped by the switch.
    fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.commit_text(true);
        self.tool = tool;
        cx.notify();
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing.is_some() {
            return;
        }
        if !self.base_ready {
            self.status = Some("still decoding".into());
            cx.notify();
            return;
        }
        // Bake the pending text/stroke into the composite, then hand the
        // encode+write+clipboard to a background task and close. The
        // composite is already current (every commit rasterizes into
        // it), so save() does not re-run rebuild_all. A 4K PNG encode on
        // the UI thread would freeze the outro for hundreds of ms.
        self.commit_text(true);
        self.commit_current();
        let img = std::mem::take(&mut self.composite);
        let path = self.path.clone();
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let png = png_bytes(&img)?;
                    std::fs::write(&path, &png)
                        .map_err(|e| format!("write {}: {e}", path.display()))?;
                    // The file changed under the library's feet:
                    // regenerate the thumbnail and refresh the entry,
                    // or the card shows the pre-edit image forever.
                    if let Err(e) = iris_lib::library::add(&path, &img) {
                        iris_lib::ilog!("iris: library refresh after save: {e}");
                    }
                    pipeline::copy_image(&img)?;
                    Ok::<(), String>(())
                })
                .await;
            if let Err(e) = result {
                iris_lib::ilog!("iris: save: {e}");
            }
        })
        .detach();
        self.begin_close(window, cx);
    }

    /// The outro: fade the whole surface out, then remove the window.
    fn begin_close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.closing.is_none() {
            self.closing = Some(Instant::now());
            cx.notify();
        }
    }

    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_close(window, cx);
    }

    fn copy_text_ocr(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        // Tesseract takes hundreds of ms on a large capture; run it
        // off the UI thread and report through the status line.
        let task = cx
            .background_executor()
            .spawn(async move { pipeline::copy_ocr_text(&path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(text) => format!("{} chars copied", text.len()),
                    Err(e) => e,
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn copy_variant(&mut self, variant: &str, cx: &mut Context<Self>) {
        self.copy_menu = false;
        let path = self.path.clone();
        let variant = variant.to_string();
        // The image variant decodes a PNG; keep that off the UI thread.
        let task = cx.background_executor().spawn(async move {
            match variant.as_str() {
                "image" => pipeline::copy_image_file(&path),
                "file" => pipeline::copy_file(&path),
                "path" => pipeline::copy_path_text(&path),
                _ => Err("unknown copy variant".to_string()),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(()) => "Copied".to_string(),
                    Err(e) => e,
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn commit_text(&mut self, keep: bool) {
        let Some(entry) = self.text_entry.take() else {
            return;
        };
        let value = entry.buffer.trim().to_string();
        if keep && !value.is_empty() {
            let mut action = Action {
                tool: Tool::Text,
                color: self.color,
                width: stroke_base(self.img_w()),
                points: vec![entry.point],
                text: Some(SharedString::from(value)),
                font_size: text_size(self.img_w()),
                filled: false,
                blur_patch: None,
                blur_rect: (0.0, 0.0, 0.0, 0.0),
                step: 0,
                step_label: SharedString::from("0"),
                bbox: None,
                cached_path: std::cell::RefCell::new(None),
            };
            action.bbox = Self::compute_bbox(&action);
            rasterize(&mut self.composite, &action, 1.0);
            self.push_edit(Edit::Add(action.clone()));
            self.actions.borrow_mut().push(action);
        }
    }

    /// The centered fit origin for a given scale, before pan. Split out
    /// so scroll-zoom can recompute it at the new scale and solve for
    /// the pan that keeps the cursor's image point fixed.
    fn fit_origin(&self, window: &Window, scale: f32) -> (f32, f32) {
        let size = window.bounds().size;
        (
            72.0 + (f32::from(size.width) - 72.0 - self.base.width() as f32 * scale) / 2.0,
            72.0 + (f32::from(size.height) - 72.0 - self.base.height() as f32 * scale) / 2.0,
        )
    }

    /// Stage geometry: the image fit into the window minus chrome, then
    /// zoomed and panned. Returns (origin_x, origin_y, scale) mapping
    /// image -> window px.
    fn view(&self, window: &Window) -> (f32, f32, f32) {
        // The chrome floats (12px margin + 48px pill), so the stage
        // keeps a 72px clear zone left and top, 24px right and bottom.
        let size = window.bounds().size;
        let usable_w = (f32::from(size.width) - 72.0 - 24.0).max(100.0);
        let usable_h = (f32::from(size.height) - 72.0 - 24.0).max(100.0);
        let fit = (usable_w / self.base.width() as f32)
            .min(usable_h / self.base.height() as f32)
            .min(4.0);
        let scale = fit * self.zoom;
        let (fx, fy) = self.fit_origin(window, scale);
        (fx + self.pan.0, fy + self.pan.1, scale)
    }

    fn to_image(&self, pos: Point<Pixels>, window: &Window) -> (f32, f32) {
        let (ox, oy, scale) = self.view(window);
        let x = ((f32::from(pos.x) - ox) / scale).clamp(0.0, self.base.width() as f32);
        let y = ((f32::from(pos.y) - oy) / scale).clamp(0.0, self.base.height() as f32);
        (x.round(), y.round())
    }
}

/// Pixelate a region of a CPU image in place.
fn pixelated_patch_rgba(
    img: &image::RgbaImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Result<image::RgbaImage, String> {
    if w == 0 || h == 0 {
        return Err("empty blur region".to_string());
    }
    let sub = image::imageops::crop_imm(img, x, y, w, h).to_image();
    let small = image::imageops::resize(
        &sub,
        (w / BLUR_BLOCK).max(1),
        (h / BLUR_BLOCK).max(1),
        image::imageops::FilterType::Triangle,
    );
    Ok(image::imageops::resize(
        &small,
        w,
        h,
        image::imageops::FilterType::Nearest,
    ))
}

/// Rasterize one action into a CPU image (save path and blur sampling).
/// Strokes are stamped circles along the primitive's outline, which keeps
/// every tool at the same visual weight without a tessellation crate.
fn rasterize(img: &mut image::RgbaImage, action: &Action, alpha_mul: f32) {
    let c = hex_rgba(action.color);
    let px = image::Rgba([
        (c.r * 255.0) as u8,
        (c.g * 255.0) as u8,
        (c.b * 255.0) as u8,
        (c.a * alpha_mul * 255.0) as u8,
    ]);
    let w = action.width;
    match action.tool {
        Tool::Pen => stroke_polyline(img, &action.points, w, px),
        Tool::Highlight => {
            let mut px = px;
            px.0[3] = (0.35 * 255.0) as u8;
            stroke_polyline(img, &action.points, w, px);
        }
        Tool::Line => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                stamp_segment(img, *a, *b, w, px);
            }
        }
        Tool::Arrow => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                stamp_segment(img, *a, *b, w, px);
                stamp_arrow_head(img, *a, *b, w, px);
            }
        }
        Tool::Ellipse => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                let cx = (a.0 + b.0) / 2.0;
                let cy = (a.1 + b.1) / 2.0;
                let rx = (b.0 - a.0).abs() / 2.0;
                let ry = (b.1 - a.1).abs() / 2.0;
                if rx > 0.0 && ry > 0.0 {
                    if action.filled {
                        // Scanline fill: write each row's chord directly.
                        // Stamping a disc per pixel was O(w*h*w) blends.
                        let y0 = (cy - ry).max(0.0) as i64;
                        let y1 = (cy + ry).min(img.height() as f32 - 1.0) as i64;
                        for y in y0..=y1 {
                            let t = (y as f32 - cy) / ry;
                            let half = rx * (1.0 - t * t).max(0.0).sqrt();
                            let x0 = (cx - half).max(0.0) as i64;
                            let x1 = (cx + half).min(img.width() as f32 - 1.0) as i64;
                            blend_row(img, y, x0, x1, px);
                        }
                    } else {
                        let n = ((rx + ry) * 0.35).max(24.0) as usize;
                        for i in 0..n {
                            let t = i as f32 / n as f32 * std::f32::consts::TAU;
                            stamp(img, cx + rx * t.cos(), cy + ry * t.sin(), w / 2.0, px);
                        }
                    }
                }
            }
        }
        Tool::Rect => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                let (tl, br) = (*a, *b);
                if action.filled {
                    // Direct scanline fill: stamping a disc per pixel
                    // was O(w*h*w) blends for a solid rect.
                    let (x0, x1) = (tl.0.min(br.0), tl.0.max(br.0));
                    let (y0, y1) = (tl.1.min(br.1), tl.1.max(br.1));
                    let y0 = y0.max(0.0) as i64;
                    let y1 = y1.min(img.height() as f32 - 1.0) as i64;
                    let x0 = x0.max(0.0) as i64;
                    let x1 = x1.min(img.width() as f32 - 1.0) as i64;
                    for y in y0..=y1 {
                        blend_row(img, y, x0, x1, px);
                    }
                } else {
                    stamp_segment(img, (tl.0, tl.1), (br.0, tl.1), w, px);
                    stamp_segment(img, (br.0, tl.1), (br.0, br.1), w, px);
                    stamp_segment(img, (br.0, br.1), (tl.0, br.1), w, px);
                    stamp_segment(img, (tl.0, br.1), (tl.0, tl.1), w, px);
                }
            }
        }
        Tool::Text => {
            if let (Some(text), Some(p)) = (&action.text, action.points.first()) {
                draw_text(img, *p, action.font_size, text, px);
            }
        }
        Tool::Blur => {}
        Tool::Counter => {
            // Filled disc at the point, then the step number on top.
            if let Some(p) = action.points.first() {
                let r = action.font_size * 0.9;
                stamp(img, p.0, p.1, r, px);
                let num = &action.step_label;
                let nw = num.chars().count() as f32 * action.font_size * 0.6;
                let white = image::Rgba([255, 255, 255, px.0[3]]);
                draw_text(img, (p.0 - nw / 2.0, p.1 + action.font_size * 0.35), action.font_size, num, white);
            }
        }
        Tool::Select | Tool::Crop => {}
    }
}

/// Fill the inclusive span [x0, x1] of row `y` with `src`, bounds
/// already clamped by the caller. One slice borrow per row instead of
/// `blend`'s per-pixel as_mut + bounds check.
fn blend_row(dst: &mut image::RgbaImage, y: i64, x0: i64, x1: i64, src: image::Rgba<u8>) {
    if x0 > x1 || y < 0 || y >= dst.height() as i64 {
        return;
    }
    let w = dst.width() as i64;
    let (x0, x1) = (x0.max(0), x1.min(w - 1));
    let row_start = (y as u32 * dst.width() * 4) as usize;
    let buf: &mut [u8] = dst.as_mut();
    let row = &mut buf[row_start..];
    let row = &mut row[(x0 as usize * 4)..=(x1 as usize * 4) + 3];
    let a = src.0[3] as f32 / 255.0;
    if a >= 1.0 {
        for px in row.chunks_exact_mut(4) {
            px.copy_from_slice(&src.0);
        }
        return;
    }
    let inv = 1.0 - a;
    for px in row.chunks_exact_mut(4) {
        px[0] = (src.0[0] as f32 * a + px[0] as f32 * inv) as u8;
        px[1] = (src.0[1] as f32 * a + px[1] as f32 * inv) as u8;
        px[2] = (src.0[2] as f32 * a + px[2] as f32 * inv) as u8;
        px[3] = px[3].max(src.0[3]);
    }
}


fn stamp(img: &mut image::RgbaImage, cx: f32, cy: f32, r: f32, px: image::Rgba<u8>) {
    let r = r.max(0.5);
    let (y0, y1) = ((cy - r).floor() as i64, (cy + r).ceil() as i64);
    // Each row's disc chord is contiguous: solve the half-width once
    // per row and fill the span, instead of testing dx*dx+dy*dy per
    // pixel.
    for y in y0..=y1 {
        let dy = y as f32 - cy;
        let half = (r * r - dy * dy).max(0.0).sqrt();
        let x0 = (cx - half).floor() as i64;
        let x1 = (cx + half).ceil() as i64;
        blend_row(img, y, x0, x1, px);
    }
}

/// Fill the capsule (stadium) around segment a-b with radius w/2.
/// The old per-point disc stamps covered the same region but blended
/// each pixel ~3x along every straight run: opaque strokes were
/// idempotent, but a semi-transparent highlight compounded to ~0.73
/// alpha where the canvas shows a uniform 0.35. One blend per pixel
/// is both faster and matches the GPU path.
fn stamp_segment(
    img: &mut image::RgbaImage,
    a: (f32, f32),
    b: (f32, f32),
    w: f32,
    px: image::Rgba<u8>,
) {
    let r = (w / 2.0).max(0.5);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    if len < f32::EPSILON {
        stamp(img, a.0, a.1, r, px);
        return;
    }
    // Unit normal: the body's offset edges are a±r·n to b±r·n.
    let (nx, ny) = (-dy / len, dx / len);
    let y0 = (a.1.min(b.1) - r).floor() as i64;
    let y1 = (a.1.max(b.1) + r).ceil() as i64;
    for y in y0..=y1 {
        let yf = y as f32;
        // Body interval: |signed distance to the line| <= r AND the
        // projection parameter t in [0,1]. Both are linear in x, so
        // each is one interval; the body is their intersection.
        let body = (|| {
            let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
            if nx.abs() > f32::EPSILON {
                let base = a.0 - (yf - a.1) * ny / nx;
                let half = (r / nx).abs();
                lo = lo.max(base - half);
                hi = hi.min(base + half);
            } else if ((yf - a.1) * ny).abs() > r {
                return None;
            }
            if dx.abs() > f32::EPSILON {
                let x_t0 = a.0 - (yf - a.1) * dy / dx;
                let x_t1 = x_t0 + len * len / dx;
                lo = lo.max(x_t0.min(x_t1));
                hi = hi.min(x_t0.max(x_t1));
            } else {
                let t = (yf - a.1) * dy / (len * len);
                if !(0.0..=1.0).contains(&t) {
                    return None;
                }
            }
            (lo <= hi).then_some((lo, hi))
        })();
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        if let Some((bl, bh)) = body {
            lo = lo.min(bl);
            hi = hi.max(bh);
        }
        // End-cap chords.
        for &(cx, cy) in &[a, b] {
            let dyc = yf - cy;
            if dyc.abs() <= r {
                let half = (r * r - dyc * dyc).max(0.0).sqrt();
                lo = lo.min(cx - half);
                hi = hi.max(cx + half);
            }
        }
        if lo <= hi {
            blend_row(img, y, lo.floor() as i64, hi.ceil() as i64, px);
        }
    }
}


fn stroke_polyline(img: &mut image::RgbaImage, points: &[(f32, f32)], w: f32, px: image::Rgba<u8>) {
    if points.len() == 1 {
        stamp(img, points[0].0, points[0].1, w / 2.0, px);
        return;
    }
    for seg in points.windows(2) {
        stamp_segment(img, seg[0], seg[1], w, px);
    }
}

fn stamp_arrow_head(
    img: &mut image::RgbaImage,
    a: (f32, f32),
    b: (f32, f32),
    w: f32,
    px: image::Rgba<u8>,
) {
    let head = 14.0 + w;
    let angle = (b.1 - a.1).atan2(b.0 - a.0);
    let p1 = (
        b.0 - head * (angle - 0.45).cos(),
        b.1 - head * (angle - 0.45).sin(),
    );
    let p2 = (
        b.0 - head * (angle + 0.45).cos(),
        b.1 - head * (angle + 0.45).sin(),
    );
    fill_triangle(img, b, p1, p2, px);
}

/// Scanline fill of triangle (v0, v1, v2): each row's chord is the
/// intersection of the two edges the row crosses. The old head fill
/// stamped ~10 overlapping 2px capsules across the span, blending
/// every pixel several times; one blend per pixel is faster and
/// matches the GPU path's single coverage.
fn fill_triangle(
    img: &mut image::RgbaImage,
    v0: (f32, f32),
    v1: (f32, f32),
    v2: (f32, f32),
    px: image::Rgba<u8>,
) {
    // Sort vertices by y: top, middle, bottom.
    let mut vs = [v0, v1, v2];
    vs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let (t, m, b) = (vs[0], vs[1], vs[2]);
    if (b.1 - t.1).abs() < f32::EPSILON {
        // Degenerate: a flat line. Stamp it as a thin segment.
        stamp_segment(img, t, b, 1.0, px);
        return;
    }
    let y0 = t.1.floor().max(0.0) as i64;
    let y1 = b.1.ceil().min(img.height() as f32 - 1.0) as i64;
    for y in y0..=y1 {
        let yf = y as f32;
        // Long edge t->b is always crossed; the short edge switches
        // from t->m to m->b at m's row.
        let u_long = (yf - t.1) / (b.1 - t.1);
        let xl = t.0 + (b.0 - t.0) * u_long;
        let xs = if yf <= m.1 {
            if (m.1 - t.1).abs() < f32::EPSILON {
                m.0
            } else {
                t.0 + (m.0 - t.0) * ((yf - t.1) / (m.1 - t.1))
            }
        } else if (b.1 - m.1).abs() < f32::EPSILON {
            m.0
        } else {
            m.0 + (b.0 - m.0) * ((yf - m.1) / (b.1 - m.1))
        };
        blend_row(
            img,
            y,
            xl.min(xs).floor() as i64,
            xl.max(xs).ceil() as i64,
            px,
        );
    }
}

fn draw_text(
    img: &mut image::RgbaImage,
    p: (f32, f32),
    size: f32,
    text: &str,
    px: image::Rgba<u8>,
) {
    // The font is read and parsed once per process: rasterize replays
    // this for every text action on every rebuild, and a disk read +
    // font parse per replay is pure waste.
    static FONT: std::sync::LazyLock<Option<ab_glyph::FontVec>> =
        std::sync::LazyLock::new(|| {
            let data = std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf")
                .or_else(|_| std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"))
                .ok()?;
            ab_glyph::FontVec::try_from_vec(data).ok()
        });
    let Some(font) = FONT.as_ref() else {
        return;
    };
    imageproc::drawing::draw_text_mut(
        img,
        px,
        p.0 as i32,
        (p.1 - size) as i32,
        ab_glyph::PxScale::from(size),
        font,
        text,
    );
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus.focus(window);
        // The toast holding our pixels leaves once a frame of ours
        // has provably been presented: the compositor only asks for
        // the next frame after consuming the previous one.
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

        // Expand-morph: the stage springs from the card rect to its
        // resting rect while the chrome fades in around it.
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
            // The window materializes around the flying image, then
            // the chrome follows in a short stagger: backdrop, topbar,
            // sidebar.
            chrome = (t * 2.2).min(1.0);
            topbar = ((t - 0.08) * 2.2).clamp(0.0, 1.0);
            sidebar = ((t - 0.18) * 2.2).clamp(0.0, 1.0);
            // The toast's 9px corners meet the stage's 6px: no pop at
            // the handoff, no pop at the landing.
            morph_radius = 12.0 - 6.0 * e;
            if t >= 1.0 {
                self.morph = None;
                self.morph_started = None;
            } else {
                window.request_animation_frame();
            }
        }
        let (sx, sy, sw, sh) = stage_rect;

        // Outro: everything fades together, then the window goes.
        let mut outro = 1.0f32;
        if let Some(started) = self.closing {
            let t = (started.elapsed().as_secs_f32()
                / motion::tempo(OUTRO).as_secs_f32())
            .min(1.0);
            outro = 1.0 - t;
            if t >= 1.0 {
                let img = self.base_img.clone();
                cx.defer(move |cx| crate::widgets::release_render(&img, cx));
                window.remove_window();
            } else {
                window.request_animation_frame();
            }
        }
        let title = self.title.clone();

        let mut root = div()
            .id("editor")
            .size_full()
            .font_family(theme::FONT)
            .bg(theme::alpha(theme::BG, chrome))
            .opacity(outro)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                if this.text_entry.is_some() {
                    match key {
                        "enter" => {
                            this.commit_text(true);
                        }
                        "escape" => {
                            this.commit_text(false);
                        }
                        "backspace" => {
                            if let Some(entry) = &mut this.text_entry {
                                entry.buffer.pop();
                                entry.caret = SharedString::from(format!("{}▏", entry.buffer));
                            }
                        }
                        _ => {
                            if !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform
                            {
                                if let Some(ch) = &ev.keystroke.key_char {
                                    if let Some(entry) = &mut this.text_entry {
                                        entry.buffer.push_str(ch);
                                        entry.caret =
                                            SharedString::from(format!("{}▏", entry.buffer));
                                        this.caret_started = Instant::now();
                                    }
                                }
                            }
                        }
                    }
                    cx.notify();
                    return;
                }
                let meta = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
                match key {
                    "escape" => {
                        if this.help {
                            this.help = false;
                        } else if this.copy_menu {
                            this.copy_menu = false;
                        } else if this.crop_rect.is_some() {
                            this.crop_rect = None;
                        } else if this.selected.is_some() {
                            this.selected = None;
                        } else {
                            this.discard(window, cx);
                        }
                    }
                    "enter" => {
                        if this.crop_rect.is_some() {
                            this.apply_crop(cx);
                        } else {
                            this.finish(window, cx);
                        }
                    }
                    "delete" | "backspace" => {
                        if let Some(i) = this.selected.take() {
                            if i < this.actions.borrow().len() {
                                let removed = this.actions.borrow_mut().remove(i);
                                this.rebuild_for_edit(&Edit::Remove(i, removed.clone()));
                                this.push_edit(Edit::Remove(i, removed));
                            }
                        }
                    }
                    "z" if meta && ev.keystroke.modifiers.shift => {
                        this.redo();
                    }
                    "y" if meta => {
                        this.redo();
                    }
                    "z" if meta => {
                        this.undo();
                    }
                    "s" if meta => {
                        this.finish(window, cx);
                    }
                    "?" | "/" => {
                        this.help = !this.help;
                    }
                    // Tool hotkeys, single letters like Markup/Photoshop.
                    // No modifier: the editor owns the window's keys.
                    "v" => this.set_tool(Tool::Select, cx),
                    "p" => this.set_tool(Tool::Pen, cx),
                    "l" => this.set_tool(Tool::Line, cx),
                    "a" => this.set_tool(Tool::Arrow, cx),
                    "e" => this.set_tool(Tool::Ellipse, cx),
                    "r" => this.set_tool(Tool::Rect, cx),
                    "t" => this.set_tool(Tool::Text, cx),
                    "h" => this.set_tool(Tool::Highlight, cx),
                    "b" => this.set_tool(Tool::Blur, cx),
                    "c" => this.set_tool(Tool::Crop, cx),
                    "n" => this.set_tool(Tool::Counter, cx),
                    "f" => {
                        this.fill = !this.fill;
                    }
                    "1" | "2" | "3" => {
                        this.stroke = key.as_bytes()[0] - b'1';
                    }
                    // Zoom: 0 fits, +/- step, space+drag pans.
                    "0" => {
                        this.zoom = 1.0;
                        this.pan = (0.0, 0.0);
                    }
                    "=" | "+" => {
                        this.zoom = (this.zoom * 1.25).min(16.0);
                    }
                    "-" => {
                        this.zoom = (this.zoom / 1.25).max(0.1);
                    }
                    "space" => {
                        this.space_pan = true;
                    }
                    _ => {}
                }
                cx.notify();
            }))
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| {
                if ev.keystroke.key == "space" {
                    this.space_pan = false;
                    cx.notify();
                }
            }))
            // Dim backdrop behind everything; clicking outside the image
            // discards. First child: every other surface stacks above it.
            .child(
                div()
                    .absolute()
                    .left(px(64.))
                    .top(px(52.))
                    .right_0()
                    .bottom_0()
                    .id("backdrop")
                    .opacity(chrome)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _ev: &MouseDownEvent, _, _| {
                            // Clicking outside the image only settles an
                            // open text entry; work is never thrown away
                            // by a stray click. Discard is explicit.
                            this.commit_text(true);
                        }),
                    ),
            )
            // Topbar: a floating pill, like Markup's chrome over the
            // canvas. No edge-to-edge bars, no hairlines.
            .child(
                div()
                    .absolute()
                    .top(px(12.))
                    .left(px(12.))
                    .right(px(12.))
                    .h(px(48.))
                    .opacity(topbar)
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(14.))
                    .rounded(px(12.))
                    .bg(theme::alpha(theme::BG_ELEV, 0.96))
                    .shadow(theme::shadow_float())
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::FG_DIM)
                            .child(title),
                    )
                    .child({
                        let mut buttons = div().flex().items_center().gap(px(6.));
                        buttons = buttons
                            .child(crate::widgets::button("btn-discard", "Discard", false).on_click(cx.listener(
                                |this, _, window, cx| this.discard(window, cx),
                            )))
                            .child(crate::widgets::button("btn-copy-text", "Copy text", false).on_click(cx.listener(
                                |this, _, _, cx| this.copy_text_ocr(cx),
                            )))
                            .child(
                                div()
                                    .relative()
                                    .child(crate::widgets::button_with_icon("btn-copy", "Copy", Icon::ChevronDown, false).on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.copy_menu = !this.copy_menu;
                                            this.copy_menu_opened = None;
                                            cx.notify();
                                        },
                                    ))),
                            )
                            .child(crate::widgets::button("btn-rotate", "Rotate", false).on_click(cx.listener(
                                |this, _, _, cx| this.transform(Transform::Rot90, cx),
                            )))
                            .child(crate::widgets::button("btn-flip", "Flip", false).on_click(cx.listener(
                                |this, _, _, cx| this.transform(Transform::FlipH, cx),
                            )))
                            .child(crate::widgets::button("btn-flipv", "Flip V", false).on_click(cx.listener(
                                |this, _, _, cx| this.transform(Transform::FlipV, cx),
                            )))
                            .child(crate::widgets::button("btn-done", "Done", true).on_click(cx.listener(
                                |this, _, window, cx| this.finish(window, cx),
                            )))
                            .child(crate::widgets::button("btn-help", "?", false).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.help = !this.help;
                                    cx.notify();
                                },
                            )));
                        buttons
                    }),
            )
            // Sidebar
            .child({
                let mut bar = div()
                    .id("sidebar")
                    .absolute()
                    .left(px(12.))
                    .top(px(72.))
                    .bottom(px(12.))
                    .w(px(48.))
                    .opacity(sidebar)
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .py(px(10.))
                    .overflow_y_scroll()
                    .rounded(px(12.))
                    .bg(theme::alpha(theme::BG_ELEV, 0.96))
                    .shadow(theme::shadow_float());
                for (tool, glyph, label) in TOOLS {
                    let active = self.tool == tool;
                    bar = bar.child(
                        crate::widgets::icon_button(ElementId::Name(label.into()), glyph, active, 32.0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.commit_text(true);
                                this.tool = tool;
                                cx.notify();
                            })),
                    );
                }
                bar = bar
                    .child(div().h(px(1.)).w(px(28.)).my(px(4.)).flex_shrink_0().bg(theme::HAIRLINE));
                // Stroke width stops for new vector actions.
                for (i, glyph) in [Icon::Stroke1, Icon::Stroke2, Icon::Stroke3]
                    .into_iter()
                    .enumerate()
                {
                    let active = self.stroke == i as u8;
                    bar = bar.child(
                        crate::widgets::icon_button(ElementId::NamedInteger("stroke".into(), i as u64), glyph, active, 32.0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.stroke = i as u8;
                                cx.notify();
                            })),
                    );
                }
                // Fill toggle for Rect/Ellipse: paints the interior
                // instead of just the outline.
                bar = bar.child(
                    crate::widgets::icon_button(ElementId::Name("fill-toggle".into()), Icon::Rect, self.fill, 32.0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.fill = !this.fill;
                            cx.notify();
                        })),
                );
                bar = bar
                    .child(div().h(px(1.)).w(px(28.)).my(px(4.)).flex_shrink_0().bg(theme::HAIRLINE))
                    .child(
                        crate::widgets::icon_button(ElementId::Name("action-undo".into()), Icon::Undo, false, 32.0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.undo();
                                cx.notify();
                            })),
                    )
                    .child(
                        crate::widgets::icon_button(ElementId::Name("action-redo".into()), Icon::Redo, false, 32.0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.redo();
                                cx.notify();
                            })),
                    )
                    .child(
                        crate::widgets::icon_button(ElementId::Name("action-clear".into()), Icon::Trash, false, 32.0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear();
                                cx.notify();
                            })),
                    )
                    .child(div().h(px(1.)).w(px(28.)).my(px(4.)).flex_shrink_0().bg(theme::HAIRLINE));
                // Swatches in a two-column grid; the active color gets a
                // ring, like Markup's swatch selection.
                let mut swatches = div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(4.))
                    .w(px(40.));
                for c in COLORS {
                    let active = c == self.color;
                    swatches = swatches.child(
                        div()
                            .id(ElementId::Name(c.into()))
                            .w(px(18.))
                            .h(px(18.))
                            .flex_shrink_0()
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .border_1()
                            .border_color(if active { theme::FG } else { theme::alpha(theme::FG, 0.0) })
                            .cursor_pointer()
                            .child(
                                div()
                                    .w(px(12.))
                                    .h(px(12.))
                                    .rounded_full()
                                    .bg(hex_rgba(c))
                                    .border_1()
                                    .border_color(theme::HAIRLINE),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.color = c;
                                cx.notify();
                            })),
                    );
                }
                bar.child(swatches)
            });


        // Stage: the image and everything drawn on it.
        let mut stage = div()
            .id("stage")
            .absolute()
            .left(px(sx))
            .top(px(sy))
            .w(px(sw))
            .h(px(sh))
            .rounded(px(morph_radius))
            .overflow_hidden()
            .border_1()
            // The toast has no border and one shadow; the stage's
            // hairline and contact shadow fade in with the chrome so
            // the flying card never gains edges mid-flight.
            .border_color(theme::alpha(theme::HAIRLINE, 0.09 * chrome))
            .shadow(vec![
                BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.35),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(24.),
                    spread_radius: px(0.),
                },
                BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.22 * chrome),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(6.),
                    spread_radius: px(0.),
                },
            ])
            .child(
                img(ImageSource::Render(self.base_img.clone()))
                    .size_full()
                    .rounded(px(morph_radius)),
            );

        // Committed vector actions + blur patches + text.
        let actions = Rc::clone(&self.actions);
        let current = self.current.clone();
        let (base_w, base_h) = (self.base.width(), self.base.height());
        stage = stage.child(
            canvas(
                move |_, _, _| (actions.clone(), current.clone()),
                move |bounds, (actions, current), window, _cx| {
                    let cur = current.as_ref().map(|rc| rc.borrow());
                    for action in actions.borrow().iter().chain(cur.as_deref()) {
                        paint_action(action, bounds, scale, base_w, base_h, window);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        let cur = self.current.as_ref().map(|rc| rc.borrow());
        for action in self.actions.borrow().iter().chain(cur.as_deref()) {
            if action.tool == Tool::Blur {
                if let Some(patch) = &action.blur_patch {
                    let (x, y, w, h) = action.blur_rect;
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px(x * scale))
                            .top(px(y * scale))
                            .w(px(w * scale))
                            .h(px(h * scale))
                            .child(img(ImageSource::Render(patch.clone())).size_full()),
                    );
                }
            } else if action.tool == Tool::Text {
                if let (Some(text), Some(p)) = (&action.text, action.points.first()) {
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px(p.0 * scale))
                            .top(px((p.1 - action.font_size) * scale))
                            .text_color(hex_rgba(action.color))
                            .text_size(px(action.font_size * scale))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(text.clone()),
                    );
                }
            } else if action.tool == Tool::Counter {
                if let Some(p) = action.points.first() {
                    let num = action.step_label.clone();
                    let nw = num.chars().count() as f32 * action.font_size * 0.6;
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px((p.0 - nw / 2.0) * scale))
                            .top(px((p.1 - action.font_size * 0.55) * scale))
                            .text_color(theme::FG)
                            .text_size(px(action.font_size * scale))
                            .font_weight(FontWeight::BOLD)
                            .child(num),
                    );
                }
            }
        }

        // Active text entry with a blinking caret. The blink is a
        // 1.06s cycle; an 8Hz timer repaints it instead of pinning the
        // whole window to vsync for the entry's lifetime.
        if let Some(entry) = &self.text_entry {
            if !self.caret_timer {
                self.caret_timer = true;
                cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(125))
                            .await;
                        let alive = this.update(cx, |this, cx| {
                            if this.text_entry.is_some() {
                                cx.notify();
                                true
                            } else {
                                this.caret_timer = false;
                                false
                            }
                        });
                        if matches!(alive, Ok(false) | Err(_)) {
                            break;
                        }
                    }
                })
                .detach();
            }
            let blink_on = (self.caret_started.elapsed().as_millis() % 1060) < 580;
            let p = entry.point;
            let size = text_size(base_w);
            stage = stage.child(
                div()
                    .absolute()
                    .left(px(p.0 * scale))
                    .top(px((p.1 - size) * scale))
                    .text_color(hex_rgba(self.color))
                    .text_size(px(size * scale))
                    .child(if blink_on {
                        entry.caret.clone()
                    } else {
                        SharedString::from(entry.buffer.clone())
                    }),
            );
        }

        // Selection outline around the active action.
        if let Some(i) = self.selected {
            if let Some((x, y, w, h)) = self.actions.borrow().get(i).and_then(|a| a.bbox) {
                stage = stage.child(
                    div()
                        .absolute()
                        .left(px(x * scale))
                        .top(px(y * scale))
                        .w(px(w * scale))
                        .h(px(h * scale))
                        .border_1()
                        .border_color(theme::alpha(theme::FG, 0.75)),
                );
            }
        }

        // Pending crop: dim everything outside the rect, hairline and
        // corner handles on it.
        if let Some((x, y, w, h)) = self.crop_rect {
            let (rx, ry, rw, rh) = (x * scale, y * scale, w * scale, h * scale);
            let dim = hsla(0.0, 0.0, 0.0, 0.45);
            for (bx, by, bw, bh) in [
                (0.0, 0.0, sw, ry),
                (0.0, ry + rh, sw, sh - ry - rh),
                (0.0, ry, rx, rh),
                (rx + rw, ry, sw - rx - rw, rh),
            ] {
                if bw > 0.0 && bh > 0.0 {
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px(bx))
                            .top(px(by))
                            .w(px(bw))
                            .h(px(bh))
                            .bg(dim),
                    );
                }
            }
            stage = stage.child(
                div()
                    .absolute()
                    .left(px(rx))
                    .top(px(ry))
                    .w(px(rw))
                    .h(px(rh))
                    .border_1()
                    .border_color(theme::FG),
            );
            for (hx, hy) in [
                (rx, ry),
                (rx + rw, ry),
                (rx, ry + rh),
                (rx + rw, ry + rh),
            ] {
                stage = stage.child(
                    div()
                        .absolute()
                        .left(px(hx - 3.5))
                        .top(px(hy - 3.5))
                        .w(px(7.))
                        .h(px(7.))
                        .rounded(px(2.))
                        .bg(theme::FG),
                );
            }
        }

        // The cursor follows the active tool: I-beam over text,
        // crosshair for everything drawable, arrow for Select.
        stage = match self.tool {
            Tool::Text => stage.cursor_text(),
            Tool::Select => stage,
            _ => stage.cursor_crosshair(),
        };

        // Stage input.
        stage = stage
            // Scroll zooms around the cursor: the image point under the
            // pointer stays put while the scale changes.
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                let dy: f32 = match ev.delta {
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                    ScrollDelta::Lines(p) => p.y * 20.0,
                };
                if dy.abs() < 0.5 {
                    return;
                }
                let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                let (ox, oy, scale) = this.view(window);
                let factor = if dy > 0.0 { 1.12 } else { 1.0 / 1.12 };
                let new_zoom = (this.zoom * factor).clamp(0.1, 16.0);
                if (new_zoom - this.zoom).abs() < f32::EPSILON {
                    return;
                }
                // Keep the image point under the cursor fixed:
                //   img = (m - o) / s  =>  o' = m - img * s'
                let img_x = (mx - ox) / scale;
                let img_y = (my - oy) / scale;
                let fit = scale / this.zoom.max(f32::EPSILON);
                let nscale = fit * new_zoom;
                this.zoom = new_zoom;
                let (fx, fy) = this.fit_origin(window, nscale);
                this.pan.0 = mx - img_x * nscale - fx;
                this.pan.1 = my - img_y * nscale - fy;
                cx.notify();
            }))
            // Middle-button drag pans; so does a left drag while space
            // is held.
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.pan_drag = Some((ev.position.x.into(), ev.position.y.into()));
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    if this.space_pan {
                        this.pan_drag = Some((ev.position.x.into(), ev.position.y.into()));
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    let p = this.to_image(ev.position, window);
                    // Mouse events travel the whole hit stack: without this
                    // the backdrop's handler below the stage fires too.
                    cx.stop_propagation();
                    this.copy_menu = false;
                    match this.tool {
                        Tool::Select => {
                            this.commit_text(true);
                            if let Some(i) = this.hit_action(p) {
                                this.selected = Some(i);
                                this.move_drag = {
                                    let old = this.actions.borrow()[i].clone();
                                    old.bbox.map(|bb| (i, p, old, bb))
                                };
                            } else {
                                this.selected = None;
                            }
                        }
                        Tool::Crop => {
                            this.commit_text(true);
                            if ev.click_count == 2 && this.crop_rect.is_some() {
                                this.apply_crop(cx);
                                cx.notify();
                                return;
                            }
                            // A press inside the pending rect moves it;
                            // anywhere else starts a new one.
                            if let Some((x, y, w, h)) = this.crop_rect {
                                if p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h {
                                    this.crop_move = Some((p, (x, y, w, h)));
                                    cx.notify();
                                    return;
                                }
                            }
                            this.crop_anchor = Some(p);
                            this.crop_rect = None;
                        }
                        Tool::Text => {
                            this.commit_text(true);
                            this.text_entry = Some(TextEntry {
                                point: p,
                                buffer: String::new(),
                                caret: SharedString::from("▏"),
                            });
                            this.caret_started = Instant::now();
                        }
                        _ => {
                            this.commit_text(true);
                            this.current = Some(Rc::new(std::cell::RefCell::new(this.new_action(p))));
                        }
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, window, cx| {
                // Pan drag (middle button or space+left) moves the view,
                // not the image content.
                if let Some((lx, ly)) = this.pan_drag {
                    let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                    this.pan.0 += mx - lx;
                    this.pan.1 += my - ly;
                    this.pan_drag = Some((mx, my));
                    cx.notify();
                    return;
                }
                if ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let p = this.to_image(ev.position, window);
                // Select: move the dragged action live; the composite
                // rebuilds once at release.
                if let Some((i, last, old, bbox)) = this.move_drag.take() {
                    let d = (p.0 - last.0, p.1 - last.1);
                    let (dx, dy) = this.move_action(i, d, bbox);
                    this.move_drag = Some((i, p, old, (bbox.0 + dx, bbox.1 + dy, bbox.2, bbox.3)));
                    cx.notify();
                    return;
                }
                // Crop: drawing a new rect.
                if let Some(anchor) = this.crop_anchor {
                    let (x0, y0) = (anchor.0.min(p.0), anchor.1.min(p.1));
                    let (x1, y1) = (anchor.0.max(p.0), anchor.1.max(p.1));
                    this.crop_rect = Some((x0, y0, x1 - x0, y1 - y0));
                    cx.notify();
                    return;
                }
                // Crop: moving the pending rect.
                if let Some((last, (x, y, w, h))) = this.crop_move {
                    let (iw, ih) =
                        (this.base.width() as f32, this.base.height() as f32);
                    let dx = (p.0 - last.0).clamp(-x, iw - (x + w));
                    let dy = (p.1 - last.1).clamp(-y, ih - (y + h));
                    let moved = (x + dx, y + dy, w, h);
                    this.crop_rect = Some(moved);
                    this.crop_move = Some((p, moved));
                    cx.notify();
                    return;
                }
                let Some(rc) = this.current.as_ref() else {
                    return;
                };
                let mut action = rc.borrow_mut();
                if action.tool == Tool::Pen || action.tool == Tool::Highlight {
                    action.points.push(p);
                } else if action.tool == Tool::Counter {
                    action.points = vec![p];
                } else {
                    action.points = vec![action.points[0], p];
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _ev: &MouseUpEvent, _, cx| {
                    if this.pan_drag.take().is_some() {
                        cx.notify();
                        return;
                    }
                    if let Some((i, _, old, _)) = this.move_drag.take() {
                        // A drag that changed nothing is not an edit.
                        if i < this.actions.borrow().len() && this.actions.borrow()[i].points != old.points {
                            let new = this.actions.borrow()[i].clone();
                            this.rebuild_for_edit(&Edit::Move(i, old.clone(), new.clone()));
                            this.push_edit(Edit::Move(i, old, new));
                        }
                        // Points back at the drag's start means the
                        // composite still shows the action correctly.
                    }
                    this.crop_anchor = None;
                    this.crop_move = None;
                    this.commit_current();
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, _ev: &MouseUpEvent, _, cx| {
                    if this.pan_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            );

        // Clicking the dim backdrop outside the image discards.
        root = root.child(stage);

        // Copy dropdown menu, fading in over 120ms. Painted after the
        // stage so its rows are never under the canvas.
        if self.copy_menu {
            let opened = *self.copy_menu_opened.get_or_insert_with(Instant::now);
            let mt = (opened.elapsed().as_secs_f32()
                / motion::tempo(motion::FADE).as_secs_f32())
            .min(1.0);
            if mt < 1.0 {
                window.request_animation_frame();
            }
            let mut menu = crate::widgets::menu()
                .absolute()
                .top(px(48. + 3.0 * (1.0 - motion::ease_out_cubic(mt))))
                .right(px(86.))
                .opacity(mt);
            for (id, label) in [("copy-image", "Copy Image"), ("copy-file", "Copy File"), ("copy-path", "Copy Path")] {
                let variant = id.strip_prefix("copy-").unwrap_or(id);
                menu = menu.child(
                    crate::widgets::menu_row(id, label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.copy_variant(variant, cx);
                        })),
                );
            }
            root = root.child(menu);
        }

        // Transient status line (copy results, OCR counts, save errors).
        if let Some(status) = &self.status {
            root = root.child(crate::widgets::status_pill(status, 76.0));
        }

        if self.help {
            root = root.child(
                crate::widgets::shortcuts_sheet(vec![
                    ("Tools", "V P L A E R T H B C".to_string()),
                    ("Stroke width", "1 / 2 / 3".to_string()),
                    ("Save and close", "Ctrl+S / Enter".to_string()),
                    ("Discard", "Esc".to_string()),
                    ("Undo / Redo", "Ctrl+Z / Ctrl+Shift+Z".to_string()),
                    ("Delete selected action", "Delete".to_string()),
                    ("Apply crop", "Enter".to_string()),
                    ("Copy image / file / path", "Copy menu".to_string()),
                    ("This sheet", "?".to_string()),
                ])
                .opacity(topbar)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.help = false;
                    cx.notify();
                })),
            );
        }

        root
    }
}

/// Tessellate and paint one action on the stage canvas. Coordinates are
/// image pixels scaled into stage space.
fn paint_action(
    action: &Action,
    bounds: Bounds<Pixels>,
    scale: f32,
    _base_w: u32,
    _base_h: u32,
    window: &mut Window,
) {
    if action.tool == Tool::Blur || action.tool == Tool::Text || action.points.is_empty() {
        return;
    }
    let c = hex_rgba(action.color);
    let color = if action.tool == Tool::Highlight {
        theme::alpha(c, 0.35)
    } else {
        c
    };
    let w = action.width * scale;
    let (ox, oy): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
    let s = move |p: (f32, f32)| point(px(p.0 * scale), px(p.1 * scale));
    // Cache hit: the triangles for these exact points at this scale
    // are already tessellated; re-stamping them at this paint's
    // origin is one add per vertex instead of re-tessellating every
    // segment's quad and cap discs. A pan drag changes only the
    // origin, so it hits this path every frame.
    let fingerprint = (
        scale,
        action.points.len(),
        *action.points.first().unwrap(),
        *action.points.last().unwrap(),
    );
    if let Some((kscale, klen, kfirst, klast, cached)) =
        action.cached_path.borrow().as_ref()
    {
        if (*kscale, *klen, *kfirst, *klast) == fingerprint {
            let mut path = Path::new(point(px(ox), px(oy)));
            stamp_tris(&mut path, cached, ox, oy);
            window.paint_path(path, color);
            return;
        }
        // Incremental: a growing pen/highlight stroke keeps its scale
        // and first point, so the new segments append onto the cached
        // triangles instead of re-tessellating the whole polyline on
        // every mousemove (O(stroke) per move became O(new segments)).
        if matches!(action.tool, Tool::Pen | Tool::Highlight)
            && (*kscale, *kfirst) == (scale, fingerprint.2)
            && *klen >= 2
            && *klen < action.points.len()
        {
            let mut tris = (**cached).clone();
            {
                let mut rec = icons::TriRecorder(tris);
                for seg in action.points[*klen - 1..].windows(2) {
                    push_segment(&mut rec, s(seg[0]), s(seg[1]), w);
                }
                tris = rec.0;
            }
            let tris = Rc::new(tris);
            let mut path = Path::new(point(px(ox), px(oy)));
            stamp_tris(&mut path, &tris, ox, oy);
            *action.cached_path.borrow_mut() = Some((
                scale,
                action.points.len(),
                fingerprint.2,
                fingerprint.3,
                tris,
            ));
            window.paint_path(path, color);
            return;
        }
    }

    // Cache miss: tessellate at origin (0,0) into the recorder, then
    // stamp at this paint's origin.
    let mut rec = icons::TriRecorder(Vec::new());
    let mut path = &mut rec;

    match action.tool {
        Tool::Pen | Tool::Highlight => {
            if action.points.len() == 1 {
                push_disc(&mut path, s(action.points[0]), w / 2.0);
            } else {
                for seg in action.points.windows(2) {
                    push_segment(&mut path, s(seg[0]), s(seg[1]), w);
                }
            }
        }
        Tool::Line => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                push_segment(&mut path, s(*a), s(*b), w);
            }
        }
        Tool::Arrow => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                let (a, b) = (s(*a), s(*b));
                push_segment(&mut path, a, b, w);
                let head = px(14.0) + px(w);
                let angle = (f32::from(b.y) - f32::from(a.y)).atan2(f32::from(b.x) - f32::from(a.x));
                let h: f32 = head.into();
                let p1 = point(
                    b.x - px(h * (angle - 0.45).cos()),
                    b.y - px(h * (angle - 0.45).sin()),
                );
                let p2 = point(
                    b.x - px(h * (angle + 0.45).cos()),
                    b.y - px(h * (angle + 0.45).sin()),
                );
                push_filled_triangle(&mut path, b, p1, p2);
            }
        }
        Tool::Ellipse => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                let (a, b) = (s(*a), s(*b));
                let cx = (f32::from(a.x) + f32::from(b.x)) / 2.0;
                let cy = (f32::from(a.y) + f32::from(b.y)) / 2.0;
                let rx = (f32::from(b.x) - f32::from(a.x)).abs() / 2.0;
                let ry = (f32::from(b.y) - f32::from(a.y)).abs() / 2.0;
                if rx > 0.0 && ry > 0.0 {
                    if action.filled {
                        push_ellipse_fill(&mut path, point(px(cx), px(cy)), rx, ry);
                    } else {
                        push_ring(&mut path, point(px(cx), px(cy)), rx, ry, w);
                    }
                }
            }
        }
        Tool::Rect => {
            if let (Some(a), Some(b)) = (action.points.first(), action.points.last()) {
                let (a, b) = (s(*a), s(*b));
                if action.filled {
                    push_rect_fill(&mut path, a, b);
                } else {
                    push_segment(&mut path, a, point(b.x, a.y), w);
                    push_segment(&mut path, point(b.x, a.y), b, w);
                    push_segment(&mut path, b, point(a.x, b.y), w);
                    push_segment(&mut path, point(a.x, b.y), a, w);
                }
            }
        }
        Tool::Counter => {
            if let Some(p) = action.points.first() {
                push_disc(&mut path, s(*p), action.font_size * 0.9 * scale);
            }
        }
        _ => {}
    }
    let tris = Rc::new(rec.0);
    let mut path = Path::new(point(px(ox), px(oy)));
    stamp_tris(&mut path, &tris, ox, oy);
    *action.cached_path.borrow_mut() = Some((
        fingerprint.0,
        fingerprint.1,
        fingerprint.2,
        fingerprint.3,
        tris,
    ));
    window.paint_path(path, color);
}

/// Re-stamp recorded triangles at paint offset (ox, oy): the cached
/// geometry is origin-relative, so a moved stage reuses it verbatim.
fn stamp_tris(path: &mut Path<Pixels>, tris: &[[f32; 6]], ox: f32, oy: f32) {
    for t in tris {
        path.push_triangle(
            (
                point(px(t[0] + ox), px(t[1] + oy)),
                point(px(t[2] + ox), px(t[3] + oy)),
                point(px(t[4] + ox), px(t[5] + oy)),
            ),
            (point(0., 1.), point(0., 1.), point(0., 1.)),
        );
    }
}

// WHY: rasterize is the CPU mirror of the GPU paint path; a tool that
// fills when it should outline (or vice versa) is a visible defect the
// editor's own pixels must catch. Not covered: GPU paint_action.
#[cfg(test)]
mod tests {
    use super::{rasterize, Action, Tool};

    fn action(tool: Tool, points: Vec<(f32, f32)>, filled: bool) -> Action {
        Action {
            tool,
            color: "#ff0000",
            width: 2.0,
            points,
            text: None,
            font_size: 16.0,
            filled,
            blur_patch: None,
            blur_rect: (0.0, 0.0, 0.0, 0.0),
            step: 0,
            step_label: "0".into(),
            bbox: None,
            cached_path: std::cell::RefCell::new(None),
        }
    }

    fn painted(img: &image::RgbaImage, x: u32, y: u32) -> bool {
        img.get_pixel(x, y).0[3] > 0
    }

    #[test]
    fn filled_rect_paints_interior() {
        let mut img = image::RgbaImage::new(20, 20);
        rasterize(&mut img, &action(Tool::Rect, vec![(2.0, 2.0), (18.0, 18.0)], true), 1.0);
        assert!(painted(&img, 10, 10), "interior must be painted");
        assert!(painted(&img, 2, 2), "corner must be painted");
    }

    #[test]
    fn outline_rect_leaves_interior_clear() {
        let mut img = image::RgbaImage::new(20, 20);
        rasterize(&mut img, &action(Tool::Rect, vec![(2.0, 2.0), (18.0, 18.0)], false), 1.0);
        assert!(!painted(&img, 10, 10), "interior must stay clear");
        assert!(painted(&img, 2, 10), "edge must be painted");
    }

    #[test]
    fn line_stamps_both_endpoints() {
        let mut img = image::RgbaImage::new(30, 10);
        rasterize(&mut img, &action(Tool::Line, vec![(2.0, 5.0), (28.0, 5.0)], false), 1.0);
        assert!(painted(&img, 2, 5));
        assert!(painted(&img, 28, 5));
        assert!(painted(&img, 15, 5));
    }
    #[test]
    fn arrow_paints_head_at_tip_not_tail() {
        let mut img = image::RgbaImage::new(60, 30);
        rasterize(&mut img, &action(Tool::Arrow, vec![(5.0, 15.0), (50.0, 15.0)], false), 1.0);
        // The head fans out behind the tip: its base sits ~14px back
        // from (50,15) and spans y 8..22 there, narrowing to the tip.
        assert!(painted(&img, 36, 8), "upper fan base must be painted");
        assert!(painted(&img, 36, 22), "lower fan base must be painted");
        assert!(painted(&img, 44, 15), "fan interior must be painted");
        assert!(painted(&img, 50, 15), "tip must be painted");
        assert!(!painted(&img, 10, 8), "tail must not fan out");
    }

    #[test]
    fn filled_ellipse_paints_center_not_corners() {
        let mut img = image::RgbaImage::new(30, 30);
        rasterize(&mut img, &action(Tool::Ellipse, vec![(5.0, 5.0), (25.0, 25.0)], true), 1.0);
        assert!(painted(&img, 15, 15), "center must be painted");
        assert!(!painted(&img, 5, 5), "bounding corner must stay clear");
    }
}

// WHY: rebuild_dirty must produce byte-identical composites to
// rebuild_all while touching only the edited region. The class closed
// here: a replayed action repaints its whole footprint, so a dirty
// region that does not cover every replayed footprint either
// compounds alpha over surviving ink or lets a blur sample stale
// composite. Not covered: the rasterizer's pixel math, which the
// tests above cover.
#[cfg(test)]
mod dirty_tests {
    use super::{Action, Editor, Tool};

    fn base_action(tool: Tool) -> Action {
        Action {
            tool,
            color: "#ff0000",
            width: 2.0,
            points: Vec::new(),
            text: None,
            font_size: 16.0,
            filled: false,
            blur_patch: None,
            blur_rect: (0.0, 0.0, 0.0, 0.0),
            step: 0,
            step_label: "0".into(),
            bbox: None,
            cached_path: std::cell::RefCell::new(None),
        }
    }

    fn stroke(points: Vec<(f32, f32)>, bbox: (f32, f32, f32, f32)) -> Action {
        let mut a = base_action(Tool::Pen);
        a.points = points;
        a.bbox = Some(bbox);
        a
    }

    fn highlight(points: Vec<(f32, f32)>, bbox: (f32, f32, f32, f32)) -> Action {
        let mut a = base_action(Tool::Highlight);
        a.points = points;
        a.bbox = Some(bbox);
        a
    }

    fn blur(rect: (f32, f32, f32, f32)) -> Action {
        let mut a = base_action(Tool::Blur);
        a.blur_rect = rect;
        a.bbox = Some(rect);
        a
    }

    fn counter(at: (f32, f32), step: u32) -> Action {
        let mut a = base_action(Tool::Counter);
        a.points = vec![at];
        a.step = step;
        a.step_label = step.to_string().into();
        let r = a.font_size * 0.9;
        a.bbox = Some((at.0 - r, at.1 - r, r * 2.0, r * 2.0));
        a
    }

    /// Full-rebuild reference: base plus every action in commit order.
    fn full_rebuild(
        base: &image::RgbaImage,
        actions: &mut [Action],
    ) -> image::RgbaImage {
        let mut img = base.clone();
        Editor::replay_actions(&mut img, actions, 0);
        img
    }

    /// The dirty path: plan against `region`, apply to a composite
    /// that already holds the pre-edit render.
    fn dirty_rebuild(
        base: &image::RgbaImage,
        composite: &mut image::RgbaImage,
        actions: &mut [Action],
        region: (f32, f32, f32, f32),
    ) {
        let (closed, mark) = Editor::dirty_plan(
            actions,
            region,
            composite.width() as f32,
            composite.height() as f32,
        );
        Editor::apply_plan(base, composite, actions, closed, &mark);
    }

    #[test]
    fn dirty_rebuild_matches_full_rebuild() {
        // Two disjoint strokes: one top-left, one bottom-right.
        let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
        let b = stroke(vec![(30.0, 30.0), (38.0, 38.0)], (26.0, 26.0, 16.0, 16.0));
        let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

        let mut full = full_rebuild(&base, &mut [a.clone(), b.clone()]);

        // Remove b: the dirty region is its bbox; a must survive.
        let mut dirty = full.clone();
        let mut remaining = [a.clone()];
        dirty_rebuild(&base, &mut dirty, &mut remaining, (26.0, 26.0, 16.0, 16.0));
        full = full_rebuild(&base, &mut [a.clone()]);
        assert_eq!(dirty.as_raw(), full.as_raw(), "dirty rebuild must equal full rebuild");
        // The skipped stroke's pixels survived: they were never restored.
        assert_eq!(dirty.get_pixel(5, 2), full.get_pixel(5, 2));
    }

    #[test]
    fn alpha_does_not_compound_outside_dirty() {
        // A highlight stroke whose footprint spills past the dirty
        // rect: replaying it over its own surviving ink doubles the
        // 0.35 alpha outside the rect. The region must grow to cover
        // the whole footprint.
        let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
        let h = highlight(vec![(4.0, 20.0), (36.0, 20.0)], (0.0, 15.0, 40.0, 10.0));
        let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

        let mut dirty = full_rebuild(&base, &mut [a.clone(), h.clone()]);
        // Remove a; h stays and intersects the dirty rect at its left.
        let mut remaining = [h.clone()];
        dirty_rebuild(&base, &mut dirty, &mut remaining, (0.0, 0.0, 14.0, 6.0));

        let full = full_rebuild(&base, &mut [h.clone()]);
        assert_eq!(
            dirty.as_raw(),
            full.as_raw(),
            "highlight ink outside the dirty rect must not be repainted over itself"
        );
    }

    #[test]
    fn blur_resamples_restored_base() {
        // A blur whose rect spills past the dirty rect: replaying it
        // while the spill still holds the old blur output samples
        // blur-of-blur. The region must cover the blur's whole rect.
        let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
        let b = blur((0.0, 0.0, 40.0, 40.0));
        let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

        let mut dirty = full_rebuild(&base, &mut [a.clone(), b.clone()]);
        let mut remaining = [b.clone()];
        dirty_rebuild(&base, &mut dirty, &mut remaining, (0.0, 0.0, 14.0, 6.0));

        let full = full_rebuild(&base, &mut [b.clone()]);
        assert_eq!(
            dirty.as_raw(),
            full.as_raw(),
            "blur must resample restored base, not its own stale output"
        );
    }

    #[test]
    fn transitive_pull_in_replays_chain() {
        // Removing a pulls in the blur that samples its ink; the
        // blur's footprint pulls in the counter stamped inside it.
        let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
        let b = blur((0.0, 0.0, 30.0, 30.0));
        let c = counter((20.0, 20.0), 1);
        let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

        let mut dirty = full_rebuild(&base, &mut [a.clone(), b.clone(), c.clone()]);
        let mut remaining = [b.clone(), c.clone()];
        dirty_rebuild(&base, &mut dirty, &mut remaining, (0.0, 0.0, 14.0, 6.0));

        let full = full_rebuild(&base, &mut [b.clone(), c.clone()]);
        assert_eq!(
            dirty.as_raw(),
            full.as_raw(),
            "the closure must replay every action the region transitively touches"
        );
    }

    #[test]
    fn renumbered_counter_repaints() {
        // Removing counter 1 renumbers counter 2 to 1: its ink
        // changes even though its footprint never intersected the
        // dirty rect.
        let c1 = counter((5.0, 5.0), 1);
        let c2 = counter((30.0, 30.0), 2);
        let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

        let mut dirty = full_rebuild(&base, &mut [c1.clone(), c2.clone()]);
        let mut remaining = [c2.clone()];
        dirty_rebuild(&base, &mut dirty, &mut remaining, c1.bbox.unwrap());

        let mut c2_renumbered = c2.clone();
        c2_renumbered.step = 1;
        c2_renumbered.step_label = "1".into();
        let full = full_rebuild(&base, &mut [c2_renumbered]);
        assert_eq!(
            dirty.as_raw(),
            full.as_raw(),
            "a renumbered counter must repaint its new digit"
        );
    }

    #[test]
    fn no_intersecting_action_leaves_composite_alone() {
        let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
        let (region, mark) =
            Editor::replay_closure(&[a], (30.0, 30.0, 8.0, 8.0), 40.0, 40.0);
        assert!(mark.iter().all(|m| !*m), "a far-away region marks nothing");
        assert_eq!(region, (30.0, 30.0, 8.0, 8.0), "the region stays put");
    }

    #[test]
    fn missing_bbox_replays_everything() {
        let mut a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
        a.bbox = None;
        let (region, mark) =
            Editor::replay_closure(&[a], (30.0, 30.0, 8.0, 8.0), 40.0, 40.0);
        assert_eq!(mark, [true], "a bbox-less action conservatively replays");
        assert_eq!(region, (0.0, 0.0, 40.0, 40.0), "and widens to the full image");
    }

    /// The pre-closure algorithm: restore only the edit's rect, then
    /// replay the suffix from the first intersecting action. Kept to
    /// prove the regression tests above observe the bug it produced.
    fn legacy_dirty(
        base: &image::RgbaImage,
        composite: &mut image::RgbaImage,
        actions: &mut [Action],
        region: (f32, f32, f32, f32),
    ) {
        let (iw, ih) = (composite.width(), composite.height());
        let Some(r) = Editor::clamp_region(region, iw, ih) else {
            return;
        };
        Editor::restore_region(base, composite, r);
        let first = actions.iter().position(|a| {
            Editor::footprint(a).map_or(true, |f| Editor::intersects(f, region))
        });
        if let Some(first) = first {
            Editor::replay_actions(composite, actions, first);
        }
    }

    #[test]
    fn legacy_algorithm_is_observably_wrong() {
        // Alpha compounding: the old suffix replay repaints the
        // highlight over its own ink outside the dirty rect.
        let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
        let h = highlight(vec![(4.0, 20.0), (36.0, 20.0)], (0.0, 15.0, 40.0, 10.0));
        let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));
        let mut legacy = full_rebuild(&base, &mut [a.clone(), h.clone()]);
        legacy_dirty(&base, &mut legacy, &mut [a.clone(), h.clone()], (0.0, 0.0, 14.0, 6.0));
        let full = full_rebuild(&base, &mut [h.clone()]);
        assert_ne!(
            legacy.as_raw(),
            full.as_raw(),
            "the old algorithm must compound alpha outside the dirty rect"
        );

        // Stale blur sampling: the old restore leaves the blur's own
        // output under the spill, so the resample blurs blur.
        let b = blur((0.0, 0.0, 40.0, 40.0));
        let mut legacy = full_rebuild(&base, &mut [a.clone(), b.clone()]);
        legacy_dirty(&base, &mut legacy, &mut [a.clone(), b.clone()], (0.0, 0.0, 14.0, 6.0));
        let full = full_rebuild(&base, &mut [b.clone()]);
        assert_ne!(
            legacy.as_raw(),
            full.as_raw(),
            "the old algorithm must sample its own stale blur output"
        );
    }
}
