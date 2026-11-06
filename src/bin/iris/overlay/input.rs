use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::*;
use iris_lib::capture::WinRect;
use iris_lib::density::Density;

use super::{
    drag_region,
    shell::{close_other_overlays, poolable, ShellLayout},
    slide_rect, Flight, Overlay, OverlayMode, MIN_SIZE,
};
use crate::{pipeline, pipeline::Region, stage};

/// Bound on the wait for a parking window's cleared frame: a window
/// whose frame never reaches the X server is still withdrawn.
const PARK_DEADLINE: Duration = Duration::from_millis(100);

/// The id of each park in flight: an overlay that draws its cleared
/// frame and is not withdrawn yet.
static WITHDRAWING: parking_lot::Mutex<Vec<u64>> = parking_lot::Mutex::new(Vec::new());
/// The id the next park takes.
static NEXT_PARK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Commands that read the screen, sent while an overlay withdraws.
static HELD: parking_lot::Mutex<Vec<crate::daemon::Command>> = parking_lot::Mutex::new(Vec::new());

/// Hold `cmd`, which reads the screen, while an overlay withdraws: the
/// screen shows that overlay, or a frame a window manager draws around
/// it as it goes, until the withdrawal and the compositor's next frame.
/// `cmd` runs once no overlay withdraws. Returns whether it is held.
pub fn hold_while_withdrawing(cmd: &crate::daemon::Command) -> bool {
    if WITHDRAWING.lock().is_empty() {
        return false;
    }
    HELD.lock().push(cmd.clone());
    true
}

/// The park `park` ran its withdrawal. Once no park is in flight, each
/// held command runs after the compositor has had the time to drop the
/// withdrawn window (`widgets::run_after_vanish`). A second call for the
/// same park does nothing.
fn withdrawn(cx: &mut App, park: u64) {
    {
        let mut withdrawing = WITHDRAWING.lock();
        let Some(at) = withdrawing.iter().position(|&p| p == park) else {
            return;
        };
        withdrawing.swap_remove(at);
        if !withdrawing.is_empty() {
            return;
        }
    }
    let held = std::mem::take(&mut *HELD.lock());
    for cmd in held {
        crate::widgets::run_after_vanish(cx, cmd);
    }
}

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
        if iris_lib::sys::session::wayland() {
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
        if std::mem::take(&mut self.map_on_frame) && !self.hidden {
            self.span(window);
        }
        // The prewarmed window maps unfocused and a WM does not
        // re-focus a window it maps again: without this the first
        // keystroke after re-arm can land on the root window. By the
        // time the frame lands the WM has finished its map handling,
        // so activation here sticks.
        if !iris_lib::sys::session::wayland() {
            window.activate_window();
        }
        // Enter raced the grab: the selection is already committed,
        // finish it now that there is a frame to crop.
        if self.pending_finish {
            self.pending_finish = false;
            self.finish(window, cx);
        }
    }

    /// Bring the parked window back for the session `reset` armed. Where
    /// no compositor blends, a frame drawn before the frozen screen
    /// lands shows the dim over black, so the window maps when the frame
    /// lands (`set_frame`), and the screen shows until then.
    pub fn unpark(&mut self, window: &mut Window) {
        if !crate::sys::window::blends() {
            self.map_on_frame = true;
            return;
        }
        self.span(window);
        // Activate on GPUI's own connection too: a window manager that
        // handles the map after the request applies its own focus.
        window.activate_window();
    }

    /// Map the parked window over the virtual screen, above and focused.
    fn span(&self, window: &Window) {
        let ((x, y), (w, h)) = (self.origin, self.view);
        crate::sys::window::unpark_span(window, x, y, w, h);
    }

    /// Re-arm a pooled window for a new session: every per-session
    /// field back to its opening state on the new layout. Images were
    /// already released when the window parked.
    pub fn reset(&mut self, layout: &ShellLayout) {
        self.hidden = false;
        self.parking = None;
        self.map_on_frame = false;
        self.frame_size = None;
        self.frame_img = None;
        self.origin = (layout.union.x, layout.union.y);
        self.view = (layout.union.width, layout.union.height);
        self.monitors = layout.monitors.clone();
        self.windows = layout.windows.clone();
        self.dragging = false;
        self.space = false;
        self.slide = (0.0, 0.0);
        self.current = None;
        self.landed = None;
        self.finalize_failed = false;
        self.cfg = iris_lib::config::Config::load();
        self.set_mode(OverlayMode::Capture);
        self.scope = super::Scope::Selection;
        self.bar_at = layout.pointer;
        self.over_bar = false;
        self.cleared = false;
        self.loupe = None;
        self.loupe_at = None;
        self.finishing = false;
        self.pending_finish = false;
        self.resize = None;
        self.moving = None;
        self.coord_at = None;
        self.hovered = None;
        self.hover_in = None;
        self.hover_out = None;
        self.flight = None;
        self.seed = layout.pointer;
        self.cursor_known = false;
    }

    /// Place the cursor and the hover-snap at the session's opening
    /// pointer, once, on the first render. Root pixels map to logical
    /// through the scale factor alone: the shell spans the virtual
    /// screen 1:1, and its bounds may not have caught up with a span
    /// requested this tick.
    pub(super) fn apply_seed(&mut self, window: &Window) {
        let Some((px_, py_)) = self.seed.take() else {
            return;
        };
        let sf = window.scale_factor();
        let (mx, my) = (
            (px_ - self.origin.0) as f32 / sf,
            (py_ - self.origin.1) as f32 / sf,
        );
        self.cursor = (mx, my);
        self.cursor_known = true;
        self.hovered = self.window_at(mx, my, sf, 1.0, 1.0);
        self.hover_in = self.hovered.map(|_| Instant::now());
    }

    /// Set what a committed selection does.
    pub fn set_mode(&mut self, mode: OverlayMode) {
        self.mode = mode;
    }

    /// Root-pixel hit test for hover-snap. The window list is in root
    /// (physical) pixels, bottom to top; the cursor arrives logical.
    pub(super) fn window_at(&self, cx: f32, cy: f32, sf: f32, sx: f32, sy: f32) -> Option<WinRect> {
        let (fx, fy) = (
            self.origin.0 as f32 + cx * sf * sx,
            self.origin.1 as f32 + cy * sf * sy,
        );
        // Topmost first.
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

    /// A logical-px rect to frame pixels, intersected with the frame.
    /// Frame pixel (0, 0) is the window's own top-left corner, the
    /// virtual screen's origin in root space. A drag that overshot the
    /// edge must not saturate `as u32` into a crop anchored at the
    /// frame's corner.
    pub(super) fn rect_to_frame(
        window: &Window,
        view: (u32, u32),
        rect: (f32, f32, f32, f32),
    ) -> Region {
        let (sx, sy) = Self::scale(window, view);
        frame_rect(window.scale_factor(), (sx, sy), view, rect)
    }

    /// Frame-pixel coordinates of a logical cursor position.
    pub(super) fn frame_pos(sf: f32, sx: f32, sy: f32, lx: f32, ly: f32) -> (i64, i64) {
        ((lx * sf * sx) as i64, (ly * sf * sy) as i64)
    }

    /// The fixed corner of a drag: the press point, moved by Space and
    /// the arrow keys during the drag.
    pub(super) fn drag_anchor(&self) -> (f32, f32) {
        (self.anchor.0 + self.slide.0, self.anchor.1 + self.slide.1)
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
        let region = Self::rect_to_frame(window, self.view, (x, y, w, h));
        if region.width == 0 || region.height == 0 {
            // The selection intersected no frame pixels (fully off the
            // union edge): nothing to crop or record.
            self.cancel(window, cx);
            return;
        }
        if self.mode == OverlayMode::RecordPick {
            // Hand the rect to the daemon as the overlay parks: the
            // daemon holds it until the overlay is off the screen, so
            // the recording's first frame does not show the selection.
            // The recording chip takes over the visual state. The
            // source reads root coordinates: frame-relative plus the
            // union's origin, which is negative with a monitor left of
            // or above the primary.
            let cmd = crate::daemon::Command::RecordRegion {
                x: region.x as i32 + self.origin.0,
                y: region.y as i32 + self.origin.1,
                w: region.width as i32,
                h: region.height as i32,
            };
            close_other_overlays(cx, Some(window.window_handle()));
            self.park(window, cx);
            crate::daemon::run(cx, &cmd);
            return;
        }
        let show_toast = self.cfg.show_toast_after_capture;
        // The card flies only where the screen blends the overlay over
        // the desktop: elsewhere every transparent pixel around the card
        // shows black. There the overlay leaves with the commit and the
        // toast opens at rest.
        let flies = show_toast && crate::sys::window::blends();
        // The flight needs pixels now, not after an encode/decode
        // round trip: the crop is already BGRA, so it wraps straight
        // into a RenderImage with no swizzle. The background finalize
        // re-crops from the shared frame and swizzles there, so the
        // UI thread never pays the RGBA pass.
        let flight_img = if flies {
            let bgra = frame_img.as_bytes(0).unwrap_or(&[]);
            match pipeline::crop_bgra(bgra, fw, fh, region) {
                Ok(crop) => Some(crate::widgets::render_image_from_bgra_owned(
                    region.width,
                    region.height,
                    crop,
                )),
                Err(e) => {
                    // A bad crop loses this capture, never the daemon.
                    iris_lib::ilog!("iris: capture: {e}");
                    self.cancel(window, cx);
                    crate::notice::failed(cx, "Capture failed", &e);
                    return;
                }
            }
        } else {
            None
        };
        pipeline::play_shutter_sound(&self.cfg);
        // Every other monitor's overlay leaves with the commit; only
        // this window stays for the flight.
        close_other_overlays(cx, Some(window.window_handle()));
        // The toast lands on this window's display, at its scale.
        let sf = window.scale_factor();
        // The file states the frame pixels per logical pixel of this
        // window, the scale the selection showed at.
        let density = Density::of_scale(sf * Self::scale(window, self.view).0);
        // Where the toast lands: on the monitor under the selection's
        // center, in the toast's corner of that monitor's work area. The
        // card's resting rect, the card, that monitor's rect, and the
        // selection's, all in logical screen coordinates.
        let landing = if show_toast {
            // The selection in root space: the frame plus the origin.
            let root = WinRect {
                x: region.x as i32 + self.origin.0,
                y: region.y as i32 + self.origin.1,
                width: region.width,
                height: region.height,
            };
            let (sx, sy) = Self::scale(window, self.view);
            let host = root.host(&self.monitors).copied().unwrap_or(WinRect {
                x: self.origin.0,
                y: self.origin.1,
                width: self.view.0,
                height: self.view.1,
            });
            let host = crate::sys::window::work_area(cx, host);
            let (mx, my, mw, mh) = Self::to_logical(self.origin, &host, sf, sx, sy);
            let (r, card) = stage::card_rest_rect(mw, mh, region.width, region.height);
            let logical = |r: &WinRect| {
                (
                    r.x as f32 / sf,
                    r.y as f32 / sf,
                    r.width as f32 / sf,
                    r.height as f32 / sf,
                )
            };
            Some((
                (mx + r.0, my + r.1, r.2, r.3),
                card,
                logical(&host),
                logical(&root),
            ))
        } else {
            None
        };
        let lands_on = landing.map(|(_, _, screen, source)| (screen, source));
        let frame_for_finalize = frame_img.clone();
        let finalize = cx.background_executor().spawn(async move {
            let bgra = frame_for_finalize.as_bytes(0).unwrap_or(&[]);
            let path = pipeline::finalize_bgra(bgra, fw, fh, region, density)?;
            // The toast's pixels scale from finalize's stash while the
            // card is still in flight: the landing opens it at once.
            let thumb = show_toast.then(|| stage::prepare_thumb(&path, sf));
            Ok::<_, String>((path, thumb))
        });
        cx.spawn(async move |this, cx| {
            let result = finalize.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((path, Some(thumb))) if this.flight.is_some() => {
                    this.landed = Some((path, thumb));
                    cx.notify();
                }
                // No card in flight: the toast opens at rest.
                Ok((path, Some(thumb))) => {
                    let (screen, source) = lands_on.unzip();
                    cx.defer(move |cx| {
                        let handoff = Box::new(|_: &mut App, _: Option<Instant>| {});
                        stage::show_toast_landed(cx, &path, thumb, screen, source, handoff);
                    });
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
        let Some(((rest, card, screen, source), img)) = landing.zip(flight_img) else {
            self.park(window, cx);
            return;
        };
        // The flight paints only its card over the live desktop. The
        // finalize task retains the pixel Arc; the full-screen atlas tile
        // no longer needs to survive the selection UI.
        if let Some(frame) = self.frame_img.take() {
            crate::widgets::release_render(&frame, cx);
        }
        if let Some((_, glass)) = self.bar_glass.take() {
            crate::widgets::release_render(&glass, cx);
        }
        self.flight = Some(Flight {
            img,
            from: (x, y, w, h),
            to: rest,
            card,
            started: Instant::now(),
            screen,
            source,
            handed: false,
            turned: None,
        });
        self.current = None;
        self.hovered = None;
        cx.notify();
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
        if let Some((_, glass)) = self.bar_glass.take() {
            crate::widgets::release_render(&glass, cx);
        }
    }

    /// Park the window instead of destroying it: the next capture reuses
    /// the live window and skips GPUI's ~130ms init. Parking withdraws
    /// the window from the WM (`sys::window::park`), so no taskbar or
    /// window switcher lists it and no placement constraint can fight
    /// it, and the GPU surface is freed while it is unmapped. Where a
    /// parked window cannot be restored it is destroyed instead.
    ///
    /// A compositor animates a withdrawn window from its last frame:
    /// GNOME Shell scales and fades it out over 150 ms. Where a
    /// compositor blends, the window draws a cleared frame first and is
    /// withdrawn once that frame is on the window, so the animation
    /// shows nothing. Without one a transparent pixel shows black, and
    /// the window is withdrawn at once.
    pub(super) fn park(&mut self, window: &mut Window, cx: &mut App) {
        self.release_assets(cx);
        self.hidden = true;
        if !poolable() {
            window.remove_window();
            return;
        }
        let overlay = window.window_handle().downcast::<Overlay>();
        match overlay.filter(|_| crate::sys::window::blends()) {
            Some(overlay) => self.withdraw_cleared(overlay, window, cx),
            None => crate::sys::window::park(window),
        }
    }

    /// Withdraw `overlay` once its next frame, drawn hidden, is on the
    /// window, or at [`PARK_DEADLINE`]. A session re-armed before then
    /// owns the window and is left alone. A command that reads the screen
    /// waits for the withdrawal (`hold_while_withdrawing`).
    fn withdraw_cleared(
        &mut self,
        overlay: WindowHandle<Overlay>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let token = Instant::now();
        self.parking = Some(token);
        let park = NEXT_PARK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        WITHDRAWING.lock().push(park);
        let withdraw = move |cx: &mut App| {
            let _ = overlay.update(cx, |this, window, _| {
                if this.parking == Some(token) {
                    this.parking = None;
                    crate::sys::window::park(window);
                }
            });
            withdrawn(cx, park);
        };
        let watch = crate::sys::window::PresentWatch::open(window, token + PARK_DEADLINE);
        window.on_next_frame(move |window, cx| {
            // A park from inside a draw leaves the window clean; the
            // frame to watch must draw.
            window.refresh();
            match watch.and_then(|w| w.arm()) {
                Some(presented) => cx
                    .spawn(async move |cx| {
                        let _ = presented.await;
                        cx.update(withdraw);
                    })
                    .detach(),
                // The frame callback after the cleared frame's.
                None => window.on_next_frame(move |_, cx| cx.defer(withdraw)),
            }
        });
        cx.spawn(async move |cx| {
            cx.background_executor().timer(PARK_DEADLINE).await;
            cx.update(withdraw);
        })
        .detach();
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
        // Space during a drag moves the selection until it is released.
        if key == "space" && self.dragging {
            self.space = true;
            return;
        }
        let dir = match key {
            "left" => Some((-1.0, 0.0)),
            "right" => Some((1.0, 0.0)),
            "up" => Some((0.0, -1.0)),
            "down" => Some((0.0, 1.0)),
            _ => None,
        };
        if let (Some((dx, dy)), Some(rect)) = (dir, self.current) {
            let step = if shift { 10.0 } else { 1.0 };
            let size = window.bounds().size;
            let size = (f32::from(size.width), f32::from(size.height));
            let (moved, d) = slide_rect(rect, (dx * step, dy * step), size);
            self.current = Some(moved);
            // During a drag the anchor moves too, so the next pointer
            // motion keeps the move.
            if self.dragging {
                self.slide = (self.slide.0 + d.0, self.slide.1 + d.1);
            }
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
        // 'c' copies the loupe's center hex while dragging, and the
        // loupe's banner states the copy until the pointer leaves that
        // pixel. The set waits on the display server, so it runs off
        // the UI thread.
        if key == "c" {
            if let Some((img, info)) = &self.loupe {
                if let Some(hex) = info.split('#').nth(1) {
                    let text = format!("#{hex}");
                    let label = SharedString::from(format!("Copied {text}"));
                    self.loupe = Some((img.clone(), label));
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

    pub(super) fn handle_key_up(&mut self, ev: &KeyUpEvent, cx: &mut Context<Self>) {
        if ev.keystroke.key == "space" && self.space {
            self.space = false;
            cx.notify();
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
            // A click (no drag) on a window captures that window, unless
            // it cleared a committed selection.
            let sf = window.scale_factor();
            let (sx, sy) = Self::scale(window, self.view);
            let picked = if self.cleared {
                None
            } else {
                self.window_at(mx, my, sf, sx, sy)
            };
            if let Some(w) = picked {
                self.current = Some(Self::to_logical(self.origin, &w, sf, sx, sy));
                self.hovered = None;
                self.finish(window, cx);
            } else {
                self.current = None;
            }
            cx.notify();
            return;
        }
        // With Space held the selection is the one shown; otherwise the
        // release point is its far corner.
        let rect = if self.space {
            self.current
        } else {
            let size = window.bounds().size;
            let region = drag_region(
                self.drag_anchor(),
                (mx, my),
                (f32::from(size.width), f32::from(size.height)),
                ev.modifiers.shift,
            );
            Some((
                region.x as f32,
                region.y as f32,
                region.width as f32,
                region.height as f32,
            ))
        };
        self.current = rect.filter(|r| r.2 >= MIN_SIZE && r.3 >= MIN_SIZE);
        cx.notify();
    }
}

/// `rect` in logical px to frame pixels at scale factor `sf` and the
/// window-to-frame `scale`, intersected with a frame of size `view`.
/// Each edge rounds to the nearest pixel, a half toward zero, as gpui
/// snaps the edges it draws: the crop is the pixels the selection
/// shows, and the selection's size readout states it.
pub(super) fn frame_rect(
    sf: f32,
    scale: (f32, f32),
    view: (u32, u32),
    rect: (f32, f32, f32, f32),
) -> Region {
    let snap = |v: f32, max: u32| ((v.abs() - 0.5).ceil().copysign(v)).clamp(0.0, max as f32);
    let x0 = snap(rect.0 * sf * scale.0, view.0);
    let y0 = snap(rect.1 * sf * scale.1, view.1);
    let x1 = snap((rect.0 + rect.2) * sf * scale.0, view.0);
    let y1 = snap((rect.1 + rect.3) * sf * scale.1, view.1);
    Region {
        x: x0 as u32,
        y: y0 as u32,
        width: (x1 - x0).max(0.0) as u32,
        height: (y1 - y0).max(0.0) as u32,
    }
}
