use std::{sync::Arc, time::Instant};

use gpui::*;
use iris_lib::capture::WinRect;

use super::{
    drag_region,
    shell::{close_other_overlays, poolable, ShellLayout},
    Flight, Overlay, OverlayMode, MIN_SIZE,
};
use crate::{pipeline, pipeline::Region, stage};

impl Overlay {
    pub fn set_frame(
        &mut self,
        img: Arc<RenderImage>,
        width: u32,
        height: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Wayland opened fullscreen with no layout: the frame's own
        // extent is the view, and its whole rect is the one monitor.
        if iris_lib::session::wayland() {
            self.origin = (0, 0);
            self.view = (width, height);
            if self.monitors.is_empty() {
                self.monitors.push(WinRect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                });
            }
        }
        self.frame_size = Some((width, height));
        self.frame_img = Some(img);
        // The prewarmed window maps unfocused and a WM does not
        // re-focus an unminimized window: without this the first
        // keystroke after re-arm can land on the root window. By the
        // time the frame lands the WM has finished its map handling,
        // so activation here sticks.
        if !iris_lib::session::wayland() {
            window.activate_window();
        }
        // Enter raced the grab: the selection is already committed,
        // finish it now that there is a frame to crop.
        if self.pending_finish {
            self.pending_finish = false;
            self.finish(window, cx);
        }
    }

    /// Re-arm a pooled window for a new session: every per-session
    /// field back to its opening state on the new layout. Images were
    /// already released when the window parked.
    pub fn reset(&mut self, layout: &ShellLayout) {
        self.hidden = false;
        self.frame_size = None;
        self.frame_img = None;
        self.origin = (layout.union.x, layout.union.y);
        self.view = (layout.union.width, layout.union.height);
        self.monitors = layout.monitors.clone();
        self.windows = layout.windows.clone();
        self.dragging = false;
        self.current = None;
        self.landed = None;
        self.finalize_failed = false;
        self.mode = OverlayMode::Capture;
        self.cfg = iris_lib::config::Config::load();
        self.hint = SharedString::from(format!(
            "{} capture   {} cancel",
            self.cfg.confirm_keybind, self.cfg.cancel_keybind
        ));
        self.loupe = None;
        self.loupe_at = None;
        self.finishing = false;
        self.pending_finish = false;
        self.opened = None;
        self.resize = None;
        self.moving = None;
        self.coord_at = None;
        self.size_at = None;
        self.hovered = None;
        self.hover_in = None;
        self.hover_out = None;
        self.flight = None;
    }

    /// Frame-pixel hit test for hover-snap. The window list is in frame
    /// (physical) pixels; the cursor arrives logical.
    pub(super) fn window_at(&self, cx: f32, cy: f32, sf: f32, sx: f32, sy: f32) -> Option<WinRect> {
        let (fx, fy) = (
            self.origin.0 as f32 + cx * sf * sx,
            self.origin.1 as f32 + cy * sf * sy,
        );
        // Topmost first: the list is bottom-to-top.
        self.windows
            .iter()
            .rev()
            .find(|w| {
                fx >= w.x as f32
                    && fx < (w.x + w.width as i32) as f32
                    && fy >= w.y as f32
                    && fy < (w.y + w.height as i32) as f32
            })
            .copied()
    }

    /// A frame-pixel window rect to this window's logical px for display.
    pub(super) fn to_logical(
        origin: (i32, i32),
        w: &WinRect,
        sf: f32,
        sx: f32,
        sy: f32,
    ) -> (f32, f32, f32, f32) {
        (
            (w.x - origin.0) as f32 / (sf * sx),
            (w.y - origin.1) as f32 / (sf * sy),
            w.width as f32 / (sf * sx),
            w.height as f32 / (sf * sy),
        )
    }

    pub(super) fn scale(window: &Window, view: (u32, u32)) -> (f32, f32) {
        // Event positions are logical px; the frame is physical. Convert
        // through physical: logical * scale_factor = physical, and the
        // window's physical size maps 1:1 onto its monitor's frame slice.
        let sf = window.scale_factor();
        let size = window.bounds().size;
        (
            view.0 as f32 / (f32::from(size.width) * sf),
            view.1 as f32 / (f32::from(size.height) * sf),
        )
    }

    /// A logical-px rect to frame pixels, intersected with the frame:
    /// a negative origin (a monitor left of or above the primary) or a
    /// drag that overshot the edge must not saturate `as u32` into a
    /// crop anchored at the frame's corner.
    pub(super) fn rect_to_frame(
        window: &Window,
        origin: (i32, i32),
        view: (u32, u32),
        rect: (f32, f32, f32, f32),
    ) -> Region {
        let sf = window.scale_factor();
        let (sx, sy) = Self::scale(window, view);
        let fx0 = origin.0 as f32 + rect.0 * sf * sx;
        let fy0 = origin.1 as f32 + rect.1 * sf * sy;
        let fx1 = fx0 + rect.2 * sf * sx;
        let fy1 = fy0 + rect.3 * sf * sy;
        let x0 = fx0.clamp(0.0, view.0 as f32);
        let y0 = fy0.clamp(0.0, view.1 as f32);
        let x1 = fx1.clamp(0.0, view.0 as f32);
        let y1 = fy1.clamp(0.0, view.1 as f32);
        Region {
            x: x0 as u32,
            y: y0 as u32,
            width: (x1 - x0).max(0.0) as u32,
            height: (y1 - y0).max(0.0) as u32,
        }
    }

    /// Frame-pixel coordinates of a logical cursor position.
    pub(super) fn frame_pos(&self, sf: f32, sx: f32, sy: f32, lx: f32, ly: f32) -> (i64, i64) {
        (
            self.origin.0 as i64 + (lx * sf * sx) as i64,
            self.origin.1 as i64 + (ly * sf * sy) as i64,
        )
    }

    pub(super) fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.finishing {
            return;
        }
        let Some((x, y, w, h)) = self.current else {
            return;
        };
        if w < MIN_SIZE || h < MIN_SIZE {
            return;
        }
        let Some((frame_img, (fw, fh))) = self.frame_img.clone().zip(self.frame_size) else {
            // The grab has not landed yet; set_frame() completes the
            // finish once it does.
            self.pending_finish = true;
            return;
        };
        self.finishing = true;
        let region = Self::rect_to_frame(window, self.origin, self.view, (x, y, w, h));
        if region.width == 0 || region.height == 0 {
            // The selection intersected no frame pixels (fully off the
            // union edge): nothing to crop or record.
            self.cancel(window, cx);
            return;
        }
        if self.mode == OverlayMode::RecordPick {
            // Hand the rect to the daemon; the overlay parks and the
            // recording chip takes over the visual state. The source
            // reads root coordinates: frame-relative plus the union's
            // origin, which is negative with a monitor left of or
            // above the primary.
            crate::daemon::run(
                cx,
                &crate::daemon::Command::RecordRegion {
                    x: region.x as i32 + self.origin.0,
                    y: region.y as i32 + self.origin.1,
                    w: region.width as i32,
                    h: region.height as i32,
                },
            );
            close_other_overlays(cx, Some(window.window_handle()));
            self.park(window, cx);
            return;
        }
        let bgra = frame_img.as_bytes(0).unwrap_or(&[]);
        let crop = match pipeline::crop_bgra(bgra, fw, fh, region) {
            Ok(c) => c,
            Err(e) => {
                // A bad crop loses this capture, never the daemon.
                iris_lib::ilog!("iris: capture: {e}");
                self.cancel(window, cx);
                crate::notice::failed(cx, "Capture failed", &e);
                return;
            }
        };
        pipeline::play_shutter_sound();
        // Every other monitor's overlay leaves with the commit; only
        // this window stays for the flight.
        close_other_overlays(cx, Some(window.window_handle()));
        let (cw, ch) = (region.width, region.height);
        // The flight needs pixels now, not after an encode/decode
        // round trip: the crop is already BGRA, so it wraps straight
        // into a RenderImage with no swizzle. The background finalize
        // re-crops from the shared frame and swizzles there, so the
        // UI thread never pays the RGBA pass.
        let flight_img = crate::widgets::render_image_from_bgra_owned(cw, ch, crop);
        let show_toast = self.cfg.show_toast_after_capture;
        // The toast lands on this window's display, at its scale.
        let sf = window.scale_factor();
        let frame_for_finalize = frame_img.clone();
        let finalize = cx.background_executor().spawn(async move {
            let bgra = frame_for_finalize.as_bytes(0).unwrap_or(&[]);
            let (path, _) = pipeline::finalize_bgra(bgra, fw, fh, region)?;
            // The toast's pixels scale from finalize's stash while the
            // card is still in flight: the landing opens it at once.
            let thumb = show_toast.then(|| stage::prepare_thumb(&path, sf));
            Ok::<_, String>((path, thumb))
        });
        cx.spawn(async move |this, cx| {
            let result = finalize.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((path, Some(thumb))) => {
                    this.landed = Some((path, thumb));
                    cx.notify();
                }
                // No toast, so nothing lands.
                Ok((_, None)) => {}
                Err(e) => {
                    // A failed save (full disk, unwritable dir) loses
                    // the capture; the daemon and the overlay recover.
                    iris_lib::ilog!("iris: capture: {e}");
                    this.finalize_failed = true;
                    cx.notify();
                    crate::notice::failed(cx, "Capture failed", &e);
                }
            });
        })
        .detach();
        if show_toast {
            // The toast lands on the monitor under the selection's
            // center, in that monitor's own bottom-right corner.
            let (fcx, fcy) = (
                region.x as f32 + region.width as f32 / 2.0,
                region.y as f32 + region.height as f32 / 2.0,
            );
            let (sx, sy) = Self::scale(window, self.view);
            let host = self
                .monitors
                .iter()
                .find(|m| {
                    fcx >= m.x as f32
                        && fcx < (m.x + m.width as i32) as f32
                        && fcy >= m.y as f32
                        && fcy < (m.y + m.height as i32) as f32
                })
                .or_else(|| self.monitors.first())
                .copied()
                .unwrap_or(iris_lib::capture::WinRect {
                    x: self.origin.0,
                    y: self.origin.1,
                    width: self.view.0,
                    height: self.view.1,
                });
            let (mx, my, mw, mh) = Self::to_logical(self.origin, &host, sf, sx, sy);
            let r = stage::card_rest_rect(mw, mh, cw, ch);
            let rest = (mx + r.0, my + r.1, r.2, r.3);
            self.flight = Some(Flight {
                img: flight_img,
                from: (x, y, w, h),
                to: rest,
                started: Instant::now(),
                screen: (
                    host.x as f32 / sf,
                    host.y as f32 / sf,
                    host.width as f32 / sf,
                    host.height as f32 / sf,
                ),
            });
            self.current = None;
            self.hovered = None;
            cx.notify();
        } else {
            self.park(window, cx);
        }
    }

    /// Release this window's painted images: with the window pooled
    /// across sessions, an unreleased tile would outlive its session.
    pub(super) fn release_assets(&mut self, cx: &mut App) {
        // take() each field: a second park on the same session (a
        // doubled Escape) must not release the same tile twice.
        if let Some(img) = self.frame_img.take() {
            crate::widgets::release_render(&img, cx);
        }
        if let Some(f) = self.flight.take() {
            crate::widgets::release_render(&f.img, cx);
        }
        if let Some((img, _)) = self.loupe.take() {
            crate::widgets::release_render(&img, cx);
        }
    }

    /// Park the window (minimized) instead of destroying it: the next
    /// capture reuses the live window and skips GPUI's ~130ms init.
    /// Minimize goes through the WM, so no placement constraint can
    /// fight it, and the GPU surface is freed while iconic. Where a
    /// minimized window cannot be restored it is destroyed instead.
    pub(super) fn park(&mut self, window: &mut Window, cx: &mut App) {
        self.release_assets(cx);
        self.hidden = true;
        if poolable() {
            window.minimize_window();
        } else {
            window.remove_window();
        }
    }

    pub(crate) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_finish = false;
        close_other_overlays(cx, Some(window.window_handle()));
        self.park(window, cx);
    }

    pub(super) fn handle_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = ev.keystroke.key.as_str();
        let shift = ev.keystroke.modifiers.shift;
        let dir = match key {
            "left" => Some((-1.0, 0.0)),
            "right" => Some((1.0, 0.0)),
            "up" => Some((0.0, -1.0)),
            "down" => Some((0.0, 1.0)),
            _ => None,
        };
        if let (Some((dx, dy)), Some((x, y, w, h))) = (dir, self.current) {
            let step = if shift { 10.0 } else { 1.0 };
            let size = window.bounds().size;
            let nx = (x + dx * step).clamp(0.0, f32::from(size.width) - w);
            let ny = (y + dy * step).clamp(0.0, f32::from(size.height) - h);
            self.current = Some((nx, ny, w, h));
            cx.notify();
            return;
        }
        // Number keys snap the selection to that monitor.
        if let Ok(d) = key.parse::<usize>() {
            if d >= 1 && d <= self.monitors.len() {
                let m = self.monitors[d - 1];
                let sf = window.scale_factor();
                let (sx, sy) = Self::scale(window, self.view);
                self.current = Some(Self::to_logical(self.origin, &m, sf, sx, sy));
                cx.notify();
                return;
            }
        }
        // 'c' copies the loupe's center hex while dragging. The set waits
        // on the display server, so it runs off the UI thread.
        if key == "c" {
            if let Some((_, info)) = &self.loupe {
                if let Some(hex) = info.split('#').nth(1) {
                    let text = format!("#{hex}");
                    cx.background_executor()
                        .spawn(async move {
                            if let Err(e) = iris_lib::clipboard::set_text(&text) {
                                crate::daemon::report_failure("Copy failed", e);
                            }
                        })
                        .detach();
                }
            }
            cx.notify();
            return;
        }
        let cfg = &self.cfg;
        let m = &ev.keystroke.modifiers;
        if iris_lib::config::keybind_matches(
            &cfg.cancel_keybind,
            key,
            m.control,
            m.shift,
            m.alt,
            m.platform,
        ) {
            self.cancel(window, cx);
        } else if iris_lib::config::keybind_matches(
            &cfg.confirm_keybind,
            key,
            m.control,
            m.shift,
            m.alt,
            m.platform,
        ) {
            self.finish(window, cx);
        }
    }

    pub(super) fn handle_mouse_up(
        &mut self,
        ev: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A handle or interior release ends the reshape or
        // move; the rect stays armed for Enter.
        if self.resize.take().is_some() || self.moving.take().is_some() {
            if let Some((img, _)) = self.loupe.take() {
                self.loupe_at = None;
                crate::widgets::release_render(&img, cx);
            }
            cx.notify();
            return;
        }
        if !self.dragging {
            return;
        }
        self.dragging = false;
        if let Some((img, _)) = self.loupe.take() {
            self.loupe_at = None;
            crate::widgets::release_render(&img, cx);
        }
        let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
        let moved = ((mx - self.anchor.0).powi(2) + (my - self.anchor.1).powi(2)).sqrt();
        if moved < 4.0 {
            // A click (no drag) on a window captures that window.
            let sf = window.scale_factor();
            let (sx, sy) = Self::scale(window, self.view);
            if let Some(w) = self.window_at(mx, my, sf, sx, sy) {
                self.current = Some(Self::to_logical(self.origin, &w, sf, sx, sy));
                self.hovered = None;
                self.finish(window, cx);
            } else {
                self.current = None;
            }
            cx.notify();
            return;
        }
        let size = window.bounds().size;
        let region = drag_region(
            self.anchor,
            (mx, my),
            (f32::from(size.width), f32::from(size.height)),
            ev.modifiers.shift,
        );
        if (region.width as f32) < MIN_SIZE || (region.height as f32) < MIN_SIZE {
            self.current = None;
        } else {
            self.current = Some((
                region.x as f32,
                region.y as f32,
                region.width as f32,
                region.height as f32,
            ));
        }
        cx.notify();
    }
}
