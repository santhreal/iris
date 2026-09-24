//! Editor operations: history (undo/redo), transformations, tools, clipboard, coordinates.

use std::{rc::Rc, sync::Arc, time::Instant};

use gpui::*;
use iris_lib::history::{self, Edit};

use super::action::{stroke_base, text_size, Action, Tool, Transform};
use super::raster::{pixelate_region_bgra, png_bytes, rasterize};
use super::Editor;
use crate::pipeline;

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
        self.undos.push(edit);
        self.redos.clear();
    }

    pub(crate) fn apply_forward(&mut self, edit: &Edit<Action>) {
        history::apply_forward(&mut self.actions.borrow_mut(), edit);
    }

    pub(crate) fn apply_inverse(&mut self, edit: &Edit<Action>) {
        history::apply_inverse(&mut self.actions.borrow_mut(), edit);
    }

    pub(crate) fn undo(&mut self) {
        if let Some(edit) = self.undos.pop() {
            self.apply_inverse(&edit);
            self.selected = None;
            self.rebuild_for_edit(&edit);
            self.redos.push(edit);
        }
    }

    pub(crate) fn redo(&mut self) {
        if let Some(edit) = self.redos.pop() {
            self.apply_forward(&edit);
            self.selected = None;
            self.rebuild_for_edit(&edit);
            self.undos.push(edit);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.actions.borrow_mut().clear();
        self.undos.clear();
        self.redos.clear();
        self.selected = None;
        self.rebuild_all();
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

    /// Apply the pending crop: everything committed flattens into the
    /// base (a crop re-keys every coordinate system; keeping actions
    /// editable across it is not worth the transform bugs), then the
    /// image is cut to the rect.
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
        let old = std::mem::replace(
            &mut self.base_img,
            crate::widgets::render_image_from_rgba(w, h, cropped.as_raw()),
        );
        crate::widgets::release_render(&old, cx);
        // base and composite share the cropped buffer: the clone this
        // replaces was a full-size copy on the UI thread, and the
        // first post-crop edit pays it through make_mut instead.
        self.composite = Arc::new(cropped);
        self.base = Arc::clone(&self.composite);
        self.base_dims = (w, h);
        self.title = SharedString::from(format!(
            "{} · {}×{}",
            self.filename, self.base_dims.0, self.base_dims.1
        ));
        self.actions.borrow_mut().clear();
        self.undos.clear();
        self.redos.clear();
        self.selected = None;
    }

    /// Rotate the whole image 90° CW, or mirror it horizontally.
    /// Like a crop, this re-keys every coordinate system, so committed
    /// actions flatten into the base first.
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
        let (w, h) = (out.width(), out.height());
        let old = std::mem::replace(
            &mut self.base_img,
            crate::widgets::render_image_from_rgba(w, h, out.as_raw()),
        );
        crate::widgets::release_render(&old, cx);
        // Same sharing as apply_crop: the clone this replaces was a
        // full-size copy on the UI thread per transform.
        self.composite = Arc::new(out);
        self.base = Arc::clone(&self.composite);
        self.base_dims = (w, h);
        self.title = SharedString::from(format!(
            "{} · {}×{}",
            self.filename, self.base_dims.0, self.base_dims.1
        ));
        self.actions.borrow_mut().clear();
        self.undos.clear();
        self.redos.clear();
        self.selected = None;
        cx.notify();
    }

    /// Switch tools from a hotkey or the sidebar: settle any open text
    /// entry first so a typed label is never dropped by the switch.
    pub(crate) fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.commit_text(true);
        self.tool = tool;
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
                    let saved = png_bytes(&img).and_then(|png| {
                        std::fs::write(&path, &png)
                            .map_err(|e| format!("write {}: {e}", path.display()))
                    });
                    saved.map_err(|e| ("Save failed", e))?;
                    // The file changed under the library's feet:
                    // regenerate the thumbnail and refresh the entry,
                    // or the card shows the pre-edit image forever.
                    if let Err(e) = iris_lib::library::add(&path, &img) {
                        iris_lib::ilog!("iris: library refresh after save: {e}");
                    }
                    pipeline::copy_image(&img).map_err(|e| ("Copy failed", e))
                })
                .await;
            // The editor is gone by now: a notice is the only place
            // left to show that the edit did not land.
            if let Err((what, e)) = result {
                iris_lib::ilog!("iris: save: {e}");
                let _ = cx.update(|cx| crate::notice::failed(cx, what, &e));
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

    pub(crate) fn copy_text_ocr(&mut self, cx: &mut Context<Self>) {
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
            72.0 + (f32::from(size.width) - 72.0 - self.base_dims.0 as f32 * scale) / 2.0,
            72.0 + (f32::from(size.height) - 72.0 - self.base_dims.1 as f32 * scale) / 2.0,
        )
    }

    /// Stage geometry: the image fit into the window minus chrome, then
    /// zoomed and panned. Returns (origin_x, origin_y, scale) mapping
    /// image -> window px.
    pub(crate) fn view(&self, window: &Window) -> (f32, f32, f32) {
        // The chrome floats (12px margin + 48px pill), so the stage
        // keeps a 72px clear zone left and top, 24px right and bottom.
        let size = window.bounds().size;
        let usable_w = (f32::from(size.width) - 72.0 - 24.0).max(100.0);
        let usable_h = (f32::from(size.height) - 72.0 - 24.0).max(100.0);
        let fit = (usable_w / self.base_dims.0 as f32)
            .min(usable_h / self.base_dims.1 as f32)
            .min(4.0);
        let scale = fit * self.zoom;
        let (fx, fy) = self.fit_origin(window, scale);
        (fx + self.pan.0, fy + self.pan.1, scale)
    }

    pub(crate) fn to_image(&self, pos: Point<Pixels>, window: &Window) -> (f32, f32) {
        let (ox, oy, scale) = self.view(window);
        let x = ((f32::from(pos.x) - ox) / scale).clamp(0.0, self.base_dims.0 as f32);
        let y = ((f32::from(pos.y) - oy) / scale).clamp(0.0, self.base_dims.1 as f32);
        (x.round(), y.round())
    }
}
