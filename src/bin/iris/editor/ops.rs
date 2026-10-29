//! Editor operations: history (undo/redo), transformations, tools, clipboard, coordinates.

use std::{rc::Rc, sync::Arc, time::Instant};

use gpui::*;
use iris_lib::history::{self, Edit};

use super::action::{stroke_base, text_size, Action, Scene, Step, Tool, Transform, STROKE_MULT};
use super::raster::{pixelate_region_bgra, png_bytes, rasterize};
use super::Editor;
use crate::pipeline;

use super::render_chrome::{STAGE_MARGIN, STAGE_TOP};

/// The status line a first Escape over unsaved changes shows.
const DISCARD_PROMPT: &str = "Press Esc again to discard changes";

/// The zoom range, as a multiple of the fit scale.
const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 16.0;

/// The zoom factor of one wheel notch.
pub(super) const WHEEL_STEP: f32 = 1.12;

/// The line scroll of one wheel notch: gpui reports a notch as three
/// lines on Linux and Windows (the Windows default) and as one line on
/// macOS.
#[cfg(target_os = "macos")]
pub(super) const NOTCH_LINES: f32 = 1.0;
#[cfg(not(target_os = "macos"))]
pub(super) const NOTCH_LINES: f32 = 3.0;

/// The pixel scroll that counts as one notch, for a touchpad that
/// reports pixels.
pub(super) const NOTCH_PX: f32 = 60.0;

/// The most notches one scroll event can zoom by: a momentum flick
/// that reports hundreds of pixels at once stays a few steps.
pub(super) const MAX_NOTCHES: f32 = 3.0;

/// The zoom factor for one scroll event. A whole notch is one
/// `WHEEL_STEP` and a fraction of a notch is that fraction of a step,
/// so a touchpad, which reports a gesture as many small deltas, zooms
/// by the distance the fingers travel rather than by the event count.
pub(crate) fn wheel_zoom(delta: ScrollDelta) -> f32 {
    let notches = match delta {
        ScrollDelta::Pixels(p) => f32::from(p.y) / NOTCH_PX,
        ScrollDelta::Lines(p) => p.y / NOTCH_LINES,
    };
    WHEEL_STEP.powf(notches.clamp(-MAX_NOTCHES, MAX_NOTCHES))
}

/// The zoom factor for one pinch event: `delta` is the fractional
/// change, 0.1 for a 10% increase.
pub(crate) fn pinch_zoom(delta: f32) -> f32 {
    (1.0 + delta).clamp(WHEEL_STEP.powf(-MAX_NOTCHES), WHEEL_STEP.powf(MAX_NOTCHES))
}

impl Editor {
    pub(crate) fn img_w(&self) -> u32 {
        self.base_dims.0
    }

    /// The next counter number: one past the highest committed step.
    pub(crate) fn next_step(&self) -> u32 {
        self.actions
            .borrow()
            .iter()
            .filter(|a| a.tool == Tool::Counter)
            .map(|a| a.step)
            .max()
            .unwrap_or(0)
            + 1
    }

    pub(crate) fn commit_current(&mut self) {
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
            // One fused pass pixelates the composite region in place
            // and returns the same pixels as a BGRA tile: the GPU
            // sprite and the saved pixels can never diverge, and the
            // old crop+resize+resize+overlay chain is gone.
            let bgra = pixelate_region_bgra(Arc::make_mut(&mut self.composite), x, y, w, h);
            let render = crate::widgets::render_image_from_bgra_owned(w, h, bgra);
            action.blur_patch = Some(render);
            action.blur_rect = (x as f32, y as f32, w as f32, h as f32);
        } else {
            rasterize(Arc::make_mut(&mut self.composite), &action, 1.0);
        }
        action.bbox = Self::compute_bbox(&action);
        self.push_edit(Edit::Add(action.clone()));
        self.actions.borrow_mut().push(action);
        self.selected = None;
    }

    pub(crate) fn push_edit(&mut self, edit: Edit<Action>) {
        self.push_step(Step::Edit(Box::new(edit)));
    }

    fn push_step(&mut self, step: Step) {
        self.undos.push(step);
        self.redos.clear();
        // A new edit is work the prompt did not warn about.
        self.disarm_discard();
    }

    pub(crate) fn apply_forward(&mut self, edit: &Edit<Action>) {
        history::apply_forward(&mut self.actions.borrow_mut(), edit);
    }

    pub(crate) fn apply_inverse(&mut self, edit: &Edit<Action>) {
        history::apply_inverse(&mut self.actions.borrow_mut(), edit);
    }

    pub(crate) fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(step) = self.undos.pop() else {
            return;
        };
        let step = match step {
            Step::Edit(edit) => {
                self.apply_inverse(&edit);
                self.rebuild_for_edit(&edit);
                Step::Edit(edit)
            }
            Step::Swap(mut scene) => {
                self.swap_scene(&mut scene, cx);
                Step::Swap(scene)
            }
        };
        self.selected = None;
        self.redos.push(step);
    }

    pub(crate) fn redo(&mut self, cx: &mut Context<Self>) {
        let Some(step) = self.redos.pop() else {
            return;
        };
        let step = match step {
            Step::Edit(edit) => {
                self.apply_forward(&edit);
                self.rebuild_for_edit(&edit);
                Step::Edit(edit)
            }
            Step::Swap(mut scene) => {
                self.swap_scene(&mut scene, cx);
                Step::Swap(scene)
            }
        };
        self.selected = None;
        self.undos.push(step);
    }

    /// Exchange the editor's image and actions with `scene`'s, and redraw
    /// the composite from them.
    fn swap_scene(&mut self, scene: &mut Scene, cx: &mut Context<Self>) {
        let same_image = Arc::ptr_eq(&self.base, &scene.base);
        std::mem::swap(&mut self.base, &mut scene.base);
        std::mem::swap(&mut *self.actions.borrow_mut(), &mut scene.actions);
        self.crop_rect = None;
        if !same_image {
            self.show_base(cx);
        }
        // With no actions the composite is the base. A new image also
        // has a new size, which a replay into the old buffer cannot fit.
        if !same_image || self.actions.borrow().is_empty() {
            self.composite = Arc::clone(&self.base);
        }
        if !self.actions.borrow().is_empty() {
            self.rebuild_all();
        }
    }

    /// Point the canvas and the size caption at `base` after it changed
    /// size or content. The composite is the caller's.
    fn show_base(&mut self, cx: &mut Context<Self>) {
        let (w, h) = self.base.dimensions();
        let old = std::mem::replace(
            &mut self.base_img,
            crate::widgets::render_image_from_rgba(w, h, self.base.as_raw()),
        );
        crate::widgets::release_render(&old, cx);
        self.base_dims = (w, h);
        self.dims = SharedString::from(format!("{} × {}", w, h));
    }

    /// Make `image`, with every committed action already in it, the new
    /// base with no actions, as one undoable step: a crop or transform
    /// re-keys every coordinate system, so actions do not carry across.
    fn rebase(&mut self, image: image::RgbaImage, cx: &mut Context<Self>) {
        // Base and composite share the new buffer: the first edit after
        // the step splits it through make_mut, or never on a save.
        self.composite = Arc::new(image);
        let old = Scene {
            base: std::mem::replace(&mut self.base, Arc::clone(&self.composite)),
            actions: std::mem::take(&mut *self.actions.borrow_mut()),
        };
        self.show_base(cx);
        self.push_step(Step::Swap(old));
        self.selected = None;
        self.crop_rect = None;
    }

    /// Remove every committed action, as one undoable step.
    pub(crate) fn clear(&mut self) {
        if self.actions.borrow().is_empty() {
            return;
        }
        let old = Scene {
            base: Arc::clone(&self.base),
            actions: std::mem::take(&mut *self.actions.borrow_mut()),
        };
        self.push_step(Step::Swap(old));
        self.selected = None;
        // With no actions the composite is the base: share it rather
        // than copy the base's pixels back over the composite.
        self.composite = Arc::clone(&self.base);
    }

    /// Translate one action, clamped so its bbox stays on the image.
    /// `bbox` is the action's current bounds, tracked by the caller:
    /// recomputing it from the points every mousemove is O(stroke)
    /// per move for a value the drag already knows.
    pub(crate) fn move_action(
        &mut self,
        i: usize,
        d: (f32, f32),
        bbox: (f32, f32, f32, f32),
    ) -> (f32, f32) {
        let (x, y, w, h) = bbox;
        let (iw, ih) = (self.base_dims.0 as f32, self.base_dims.1 as f32);
        let dx = d.0.clamp(-x, iw - (x + w));
        let dy = d.1.clamp(-y, ih - (y + h));
        let mut actions = self.actions.borrow_mut();
        let Some(action) = actions.get_mut(i) else {
            return (0.0, 0.0);
        };
        for p in Rc::make_mut(&mut action.points).iter_mut() {
            p.0 += dx;
            p.1 += dy;
        }
        // Keep the tessellation cache valid through the drag: the
        // cached triangles are stage-space (image * scale), so the
        // same delta scaled applies, and first/last shift with the
        // points. Without this every mousemove re-tessellated the
        // action being dragged.
        if let Some((kscale, _, kfirst, klast, tris)) = action.cached_path.borrow_mut().as_mut() {
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

    /// An arrow key: move the pending crop, or the selected mark, by `d`
    /// image pixels, clamped to the image. Each move of a mark is one
    /// undo step. A key during a pointer drag does nothing, so the
    /// drag's one step stays whole.
    pub(crate) fn nudge(&mut self, d: (f32, f32)) {
        if self.move_drag.is_some() || self.crop_anchor.is_some() || self.crop_move.is_some() {
            return;
        }
        if let Some((x, y, w, h)) = self.crop_rect {
            let (iw, ih) = (self.base.width() as f32, self.base.height() as f32);
            self.crop_rect = Some((
                (x + d.0).clamp(0.0, (iw - w).max(0.0)),
                (y + d.1).clamp(0.0, (ih - h).max(0.0)),
                w,
                h,
            ));
            return;
        }
        let Some(i) = self.selected else {
            return;
        };
        let Some((old, bbox)) = self
            .actions
            .borrow()
            .get(i)
            .and_then(|a| Some((a.clone(), a.bbox?)))
        else {
            return;
        };
        if self.move_action(i, d, bbox) == (0.0, 0.0) {
            return;
        }
        let new = self.actions.borrow()[i].clone();
        let edit = Edit::Move(i, old, new);
        self.rebuild_for_edit(&edit);
        self.push_edit(edit);
    }

    /// Select markup color `color` for the next mark, and give it to
    /// the selected mark if that takes a color.
    pub(crate) fn set_color(&mut self, color: &'static str) {
        self.color = color;
        self.restyle_selected(|a| {
            if a.tool.colored() {
                a.color = color;
            }
        });
    }

    /// Select stroke width stop `stop` for the next mark, and give it
    /// to the selected mark if that is drawn with a stroke.
    pub(crate) fn set_stroke(&mut self, stop: u8) {
        self.stroke = stop;
        let width = stroke_base(self.img_w()) * STROKE_MULT[stop as usize];
        self.restyle_selected(|a| {
            if a.tool.stroked() {
                a.width = width;
            }
        });
    }

    /// Toggle fill for the next mark, and give the new setting to the
    /// selected mark if that is a rectangle or an ellipse.
    pub(crate) fn toggle_fill(&mut self) {
        self.fill = !self.fill;
        let fill = self.fill;
        self.restyle_selected(|a| {
            if a.tool.fillable() {
                a.filled = fill;
            }
        });
    }

    /// Select mark `hit`, or none, and show the selected mark's color,
    /// stroke width, and fill in the options bar, where it has them: a
    /// change in the bar then steps from the mark's own setting.
    pub(crate) fn select(&mut self, hit: Option<usize>) {
        self.selected = hit;
        let actions = self.actions.borrow();
        let Some(a) = hit.and_then(|i| actions.get(i)) else {
            return;
        };
        if a.tool.colored() {
            self.color = a.color;
        }
        if a.tool.stroked() {
            let base = stroke_base(self.base_dims.0);
            if let Some(stop) = STROKE_MULT
                .iter()
                .position(|m| (base * m - a.width).abs() < 1e-3)
            {
                self.stroke = stop as u8;
            }
        }
        if a.tool.fillable() {
            self.fill = a.filled;
        }
    }

    /// Apply `style` to a copy of the selected mark and, when that
    /// changes its color, width, or fill, put the copy in its place as
    /// one undo step. A key during a move drag does nothing, so the
    /// drag's one step stays whole.
    fn restyle_selected(&mut self, style: impl FnOnce(&mut Action)) {
        if self.move_drag.is_some() {
            return;
        }
        let Some(i) = self.selected else {
            return;
        };
        let Some(old) = self.actions.borrow().get(i).cloned() else {
            return;
        };
        let mut new = old.clone();
        style(&mut new);
        if (new.color, new.width, new.filled) == (old.color, old.width, old.filled) {
            return;
        }
        // The cached triangles are keyed on the points only, and a new
        // width or fill changes the shape the same points draw.
        new.cached_path = std::cell::RefCell::new(None);
        new.bbox = Self::compute_bbox(&new);
        self.actions.borrow_mut()[i] = new.clone();
        let edit = Edit::Move(i, old, new);
        self.rebuild_for_edit(&edit);
        self.push_edit(edit);
    }

    /// Apply the pending crop: everything committed flattens into the
    /// new base, then the image is cut to the rect. Undo restores the
    /// uncut image and its actions.
    pub(crate) fn apply_crop(&mut self, cx: &mut Context<Self>) {
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
        // composite is already current: every commit rasterizes into
        // it, so rebuild_all's restore+replay would produce the same
        // pixels the crop is about to cut.
        let (x, y) = (x.max(0.0) as u32, y.max(0.0) as u32);
        if x >= self.composite.width() || y >= self.composite.height() {
            return;
        }
        let w = (w as u32).min(self.composite.width() - x);
        let h = (h as u32).min(self.composite.height() - y);
        if w < 8 || h < 8 {
            return;
        }
        let cropped = image::imageops::crop_imm(&*self.composite, x, y, w, h).to_image();
        self.rebase(cropped, cx);
    }

    /// Rotate the whole image 90° CW, or mirror it horizontally. Like a
    /// crop, committed actions flatten into the new base, and undo
    /// restores them.
    pub(crate) fn transform(&mut self, op: Transform, cx: &mut Context<Self>) {
        if !self.base_ready {
            self.status = Some("still decoding".into());
            return;
        }
        self.commit_text(true);
        self.commit_current();
        let out = match op {
            Transform::Rot90 => image::imageops::rotate90(&*self.composite),
            Transform::FlipH => image::imageops::flip_horizontal(&*self.composite),
            Transform::FlipV => image::imageops::flip_vertical(&*self.composite),
        };
        self.rebase(out, cx);
        cx.notify();
    }

    /// Switch tools from a hotkey or the sidebar: settle any open text
    /// entry first so a typed label is never dropped by the switch. A
    /// selection is the Select tool's: another tool clears it, so the
    /// keys and the options bar act on the next mark.
    pub(crate) fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.commit_text(true);
        self.tool = tool;
        if tool != Tool::Select {
            self.selected = None;
        }
        cx.notify();
    }

    pub(crate) fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        // Every change is a history step, so with none left to undo the
        // image is the file's. Rewriting it re-encodes the file, moves
        // its library entry to the top, and takes the clipboard, all for
        // a capture that was only looked at.
        if self.undos.is_empty() {
            self.begin_close(window, cx);
            return;
        }
        // Move the pixels out rather than cloning: after a crop or
        // transform, base shares composite's buffer, so resetting base
        // first leaves composite unique and try_unwrap hands the image
        // over whole. A still-shared composite (no crop) is unique
        // already; the Err arm is the pre-decode placeholder.
        self.base = Arc::new(image::RgbaImage::new(1, 1));
        let img = match Arc::try_unwrap(std::mem::replace(
            &mut self.composite,
            Arc::new(image::RgbaImage::new(1, 1)),
        )) {
            Ok(img) => img,
            Err(rc) => (*rc).clone(),
        };
        let path = self.path.clone();
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    // A rename: a full disk or an exit midway leaves the
                    // capture as it was, not cut short.
                    let saved = png_bytes(&img).and_then(|png| {
                        iris_lib::output::replace(&path, &png)
                            .map_err(|e| format!("write {}: {e}", path.display()))
                    });
                    saved.map_err(|e| ("Save failed", e))?;
                    // The file changed under the library's feet:
                    // regenerate the thumbnail and refresh the entry,
                    // or the card shows the pre-edit image forever.
                    // The entry keeps its place and day.
                    if let Err(e) = iris_lib::library::update(&path, &img) {
                        iris_lib::ilog!("iris: library refresh after save: {e}");
                    }
                    pipeline::copy_image(&img).map_err(|e| ("Copy failed", e))
                })
                .await;
            // The editor is gone by now: a notice is the only place
            // left to show that the edit did not land.
            if let Err((what, e)) = result {
                iris_lib::ilog!("iris: save: {e}");
                cx.update(|cx| crate::notice::failed(cx, what, &e));
            }
        })
        .detach();
        self.begin_close(window, cx);
    }

    /// The outro: fade the whole surface out, then remove the window.
    pub(crate) fn begin_close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.closing.is_none() {
            self.closing = Some(Instant::now());
            cx.notify();
        }
    }

    pub(crate) fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_close(window, cx);
    }

    /// Escape with nothing open to dismiss: discard, unless that drops
    /// unsaved changes. Then the first press shows `DISCARD_PROMPT`, and
    /// a second press while it shows discards.
    pub(crate) fn escape_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.undos.is_empty() || self.status.as_deref() == Some(DISCARD_PROMPT) {
            self.discard(window, cx);
        } else {
            self.status = Some(DISCARD_PROMPT.to_string());
        }
    }

    /// Drop the discard prompt, and with it the second Escape's discard.
    fn disarm_discard(&mut self) {
        if self.status.as_deref() == Some(DISCARD_PROMPT) {
            self.status = None;
        }
    }

    /// A close from the close control or the window manager: save and
    /// close, as `Ctrl+W` does. An editor whose image never decoded has
    /// nothing to save and closes.
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.base_ready {
            self.finish(window, cx);
        } else {
            self.begin_close(window, cx);
        }
    }

    pub(crate) fn copy_text_ocr(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        // Copy Text reads the image as shown, as Copy Image does: a crop
        // narrows what it reads and a blur hides what it covers. Before
        // the image decodes the file is that image.
        let shown = self.base_ready.then(|| {
            self.commit_text(true);
            self.commit_current();
            Arc::clone(&self.composite)
        });
        // The encode and the recognition take hundreds of ms on a large
        // capture; run them off the UI thread and report through the
        // status line.
        let task = cx.background_executor().spawn(async move {
            match shown {
                Some(img) => png_bytes(&img).and_then(|png| pipeline::copy_ocr_png(&png)),
                None => pipeline::copy_ocr_text(&path),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(text) => format!("Copied {} characters", text.chars().count()),
                    Err(e) => e,
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn copy_variant(&mut self, variant: &str, cx: &mut Context<Self>) {
        self.copy_menu = false;
        let path = self.path.clone();
        // Copy Image takes the image as shown, marks and all, as the
        // save does. Before the image decodes the file is that image.
        let shown = (variant == "image" && self.base_ready).then(|| {
            self.commit_text(true);
            self.commit_current();
            Arc::clone(&self.composite)
        });
        let variant = variant.to_string();
        // The image variants encode or decode a PNG; keep that off the
        // UI thread.
        let task = cx.background_executor().spawn(async move {
            match (variant.as_str(), shown) {
                ("image", Some(img)) => pipeline::copy_image(&img),
                ("image", None) => pipeline::copy_image_file(&path),
                ("file", _) => pipeline::copy_file(&path),
                ("path", _) => pipeline::copy_path_text(&path),
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

    pub(crate) fn commit_text(&mut self, keep: bool) {
        let Some(entry) = self.text_entry.take() else {
            return;
        };
        let value = entry.buffer.trim().to_string();
        if keep && !value.is_empty() {
            let mut action = Action {
                tool: Tool::Text,
                color: self.color,
                width: stroke_base(self.img_w()),
                points: Rc::new(vec![entry.point]),
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
            rasterize(Arc::make_mut(&mut self.composite), &action, 1.0);
            self.push_edit(Edit::Add(action.clone()));
            self.actions.borrow_mut().push(action);
        }
    }

    /// The centered fit origin for a given scale, before pan. Split out
    /// so scroll-zoom can recompute it at the new scale and solve for
    /// the pan that keeps the cursor's image point fixed.
    pub(crate) fn fit_origin(&self, window: &Window, scale: f32) -> (f32, f32) {
        let size = window.bounds().size;
        (
            (f32::from(size.width) - self.base_dims.0 as f32 * scale) / 2.0,
            STAGE_TOP
                + (f32::from(size.height) - STAGE_TOP - self.base_dims.1 as f32 * scale) / 2.0,
        )
    }

    /// Stage geometry: the image fit into the window minus chrome, then
    /// zoomed and panned. Returns (origin_x, origin_y, scale) mapping
    /// image -> window px.
    pub(crate) fn view(&self, window: &Window) -> (f32, f32, f32) {
        // The stage is the window under the toolbar and the options
        // bar; the fit image keeps STAGE_MARGIN clear on every side.
        let size = window.bounds().size;
        let usable_w = (f32::from(size.width) - 2.0 * STAGE_MARGIN).max(100.0);
        let usable_h = (f32::from(size.height) - STAGE_TOP - 2.0 * STAGE_MARGIN).max(100.0);
        let fit = (usable_w / self.base_dims.0 as f32)
            .min(usable_h / self.base_dims.1 as f32)
            .min(4.0);
        let scale = fit * self.zoom;
        let (fx, fy) = self.fit_origin(window, scale);
        (fx + self.pan.0, fy + self.pan.1, scale)
    }

    /// Multiply the zoom by `factor`, clamped to the zoom range, and
    /// pan so the image point under `at` stays under it. Returns
    /// whether the zoom changed.
    pub(crate) fn zoom_at(&mut self, at: (f32, f32), factor: f32, window: &Window) -> bool {
        let new_zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        if (new_zoom - self.zoom).abs() < f32::EPSILON {
            return false;
        }
        let (ox, oy, scale) = self.view(window);
        let (ix, iy) = ((at.0 - ox) / scale, (at.1 - oy) / scale);
        let new_scale = scale / self.zoom * new_zoom;
        let (fx, fy) = self.fit_origin(window, new_scale);
        self.pan = (at.0 - fx - ix * new_scale, at.1 - fy - iy * new_scale);
        self.zoom = new_zoom;
        true
    }

    pub(crate) fn to_image(&self, pos: Point<Pixels>, window: &Window) -> (f32, f32) {
        let (ox, oy, scale) = self.view(window);
        let x = ((f32::from(pos.x) - ox) / scale).clamp(0.0, self.base_dims.0 as f32);
        let y = ((f32::from(pos.y) - oy) / scale).clamp(0.0, self.base_dims.1 as f32);
        (x.round(), y.round())
    }
}
