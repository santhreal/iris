//! Composite rebuild and dirty-region tracking.

use std::sync::Arc;

use gpui::SharedString;
use iris_lib::history::Edit;

use super::action::{Action, Tool};
use super::raster::{pixelate_region_bgra, rasterize};
use super::Editor;

impl Editor {
    /// Replay actions `skip..` onto `composite` in commit order: blur
    /// pixelates whatever is beneath it at that point, so later blurs
    /// sample through earlier ones, matching the canvas2d implementation.
    /// Patches are recomputed here so moved or cropped blurs sample their
    /// new location. Associated function so the partial-replay path is
    /// testable without a Window.
    pub(crate) fn replay_actions(
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
    pub(crate) fn replay_one(composite: &mut image::RgbaImage, action: &mut Action) {
        if action.tool == Tool::Blur {
            let (x, y) = (
                action.blur_rect.0.max(0.0) as u32,
                action.blur_rect.1.max(0.0) as u32,
            );
            let (w, h) = (action.blur_rect.2 as u32, action.blur_rect.3 as u32);
            if w >= 1 && h >= 1 && x < composite.width() && y < composite.height() {
                let w = w.min(composite.width() - x);
                let h = h.min(composite.height() - y);
                // One fused pass pixelates the region in place and
                // returns the BGRA tile for the GPU sprite.
                let bgra = pixelate_region_bgra(composite, x, y, w, h);
                let render = crate::widgets::render_image_from_bgra_owned(w, h, bgra);
                action.blur_patch = Some(render);
            }
        } else {
            rasterize(composite, action, 1.0);
        }
    }

    /// Copy the (x, y, w, h) rect of `base` over `composite`, row by row:
    /// the dirty rebuild restores only the region an edit touched instead
    /// of the whole frame.
    pub(crate) fn restore_region(
        base: &image::RgbaImage,
        composite: &mut image::RgbaImage,
        (x, y, w, h): (u32, u32, u32, u32),
    ) {
        let stride = base.width() as usize * 4;
        let src = base.as_raw();
        let dst: &mut [u8] = composite.as_mut();
        // Band the row copies: a large dirty region (a big blur, a
        // whole-image undo) is an O(area) memcpy that splits across cores.
        let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
        let span = w * 4;
        iris_lib::par::par_bands_mut(dst, stride, |band, start| {
            let first_row = start / stride;
            let band_rows = band.len() / stride;
            for row in first_row.max(y)..(first_row + band_rows).min(y + h) {
                let local = (row - first_row) * stride + x * 4;
                let off = row * stride + x * 4;
                band[local..local + span].copy_from_slice(&src[off..off + span]);
            }
        });
    }

    /// The rect an action can paint or sample, in image pixels: its
    /// stored bbox, padded by the font size for text and counters whose
    /// ink overflows the estimate. `None` means the action could have
    /// painted anywhere and forces a full rebuild.
    pub(crate) fn footprint(action: &Action) -> Option<(f32, f32, f32, f32)> {
        let (x, y, w, h) = action.bbox?;
        let pad = if matches!(action.tool, Tool::Text | Tool::Counter) {
            action.font_size
        } else {
            0.0
        };
        Some((x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad))
    }

    /// Do two (x, y, w, h) rects overlap?
    pub(crate) fn intersects(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)) -> bool {
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
    pub(crate) fn replay_closure(
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
    pub(crate) fn clamp_region(
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

    pub(crate) fn rebuild_all(&mut self) {
        // Replay in commit order: blur pixelates whatever is beneath it
        // at that point, so later blurs sample through earlier ones,
        // matching the canvas2d implementation. Patches are recomputed
        // here so moved or cropped blurs sample their new location.
        // Reuse the composite buffer: it is always the same size as base,
        // so replay writes into it instead of cloning a fresh image.
        // When base and composite still share one buffer (post-decode,
        // pre-first-edit) the restore is a no-op: the pixels are
        // already identical, so only the make_mut split is needed.
        let needs_restore = !Arc::ptr_eq(&self.base, &self.composite);
        let composite = Arc::make_mut(&mut self.composite);
        if needs_restore {
            // A full-image restore is a 33MB memcpy on a 4K capture;
            // band it so undo/crop/transform rebuilds split across
            // cores instead of stalling the UI thread on one.
            let dst: &mut [u8] = composite.as_mut();
            let src = self.base.as_raw();
            iris_lib::par::par_bands_mut(dst, 1 << 20, |band, start| {
                band.copy_from_slice(&src[start..start + band.len()]);
            });
        }
        Self::replay_actions(composite, &mut self.actions.borrow_mut(), 0);
    }

    /// The union of two (x, y, w, h) rects.
    pub(crate) fn union_rect(
        a: (f32, f32, f32, f32),
        b: (f32, f32, f32, f32),
    ) -> (f32, f32, f32, f32) {
        let (x, y) = (a.0.min(b.0), a.1.min(b.1));
        (
            x,
            y,
            (a.0 + a.2).max(b.0 + b.2) - x,
            (a.1 + a.3).max(b.1 + b.3) - y,
        )
    }

    /// Plan a partial rebuild: renumber counters (steps are
    /// commit-order across every counter, so this runs over all
    /// actions even when the replay is partial), seed the dirty
    /// region with the footprints of counters whose number changed
    /// (they paint different ink), then close the region over every
    /// action it touches. Returns the closed region and replay mask.
    pub(crate) fn dirty_plan(
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
    pub(crate) fn apply_plan(
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
    pub(crate) fn rebuild_dirty(&mut self, region: (f32, f32, f32, f32)) {
        let (iw, ih) = (self.composite.width(), self.composite.height());
        let (closed, mark) =
            Self::dirty_plan(&mut self.actions.borrow_mut(), region, iw as f32, ih as f32);
        Self::apply_plan(
            &self.base,
            Arc::make_mut(&mut self.composite),
            &mut self.actions.borrow_mut(),
            closed,
            &mark,
        );
    }

    pub(crate) fn rebuild_for_edit(&mut self, edit: &Edit<Action>) {
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
}
