//! Capture overlay: the fullscreen region picker over a frozen frame.
//!
//! The screen is grabbed once before this window opens; everything the
//! user sees and selects comes out of that frozen frame. Hover snaps to
//! top-level windows (X11, Windows, macOS), dragging commits a region, the loupe
//! tracks the drag edge for pixel-precise crops. Right-click and Esc
//! cancel; Enter, double-click or a plain click on a window finishes.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::*;
use iris_lib::capture::WinRect;

use crate::{pipeline::Region, theme};
use flight::Flight;

mod flight;
mod input;
mod loupe;
mod shell;
#[cfg(test)]
mod tests;
mod view;

pub use shell::{layout, open_shell, slice_frame_bgra, warmup};

pub(super) const MIN_SIZE: f32 = 3.0;
/// The dim layer fades in when the overlay appears.
pub(super) const DIM_FADE: Duration = Duration::from_millis(180);
/// The dim's opacity over the frozen frame once faded in.
pub(super) const DIM: f32 = 0.32;
/// Window-snap reveal fades both ways, like macOS's highlight.
pub(super) const HOVER_FADE: Duration = Duration::from_millis(90);
/// Edge of a selection resize handle, in logical px.
pub(super) const HANDLE_PX: f32 = 10.0;

/// What a committed selection does. Capture crops and saves; RecordPick
/// reports the rect to the daemon and starts a region recording.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OverlayMode {
    Capture,
    RecordPick,
}

pub struct Overlay {
    /// Parked (minimized): the window stays alive with its renderer,
    /// and the next capture reuses it.
    pub hidden: bool,
    /// What finish() does with the committed selection.
    pub mode: OverlayMode,
    /// None until the background grab lands: the window maps at once,
    /// transparent over the live desktop with crosshair and snap
    /// already live, and the frozen frame fades in behind it.
    /// The frozen frame's pixel size; the pixels themselves live in
    /// `frame_img`'s BGRA buffer (the only CPU copy).
    frame_size: Option<(u32, u32)>,
    /// This window's screen: origin in frame pixels and size. The
    /// displayed image is exactly the frozen frame at native scale;
    /// every coordinate conversion goes through it.
    origin: (i32, i32),
    view: (u32, u32),
    /// Physical monitor rects inside the view: toast anchoring picks
    /// the one under the committed selection.
    monitors: Vec<WinRect>,
    frame_img: Option<Arc<RenderImage>>,
    windows: Vec<WinRect>,
    focus: FocusHandle,
    dragging: bool,
    anchor: (f32, f32),
    /// A committed selection being reshaped by a handle: the handle
    /// index (0-7, corners then edges) and the rect at drag start.
    resize: Option<(usize, (f32, f32, f32, f32))>,
    /// A committed selection being moved by its interior: the pointer
    /// offset into the rect at grab time.
    moving: Option<(f32, f32)>,
    current: Option<(f32, f32, f32, f32)>,
    hovered: Option<WinRect>,
    cursor: (f32, f32),
    loupe: Option<(Arc<RenderImage>, SharedString)>,
    /// Frame pixel the loupe was last built for; a mousemove inside
    /// the same pixel skips the rebuild.
    loupe_at: Option<(i64, i64)>,
    /// Reused loupe pixel buffer: a drag mints one 92KB image per
    /// mousemove; keeping the allocation across rebuilds removes the
    /// per-move alloc/free churn.
    loupe_scratch: Vec<u8>,
    finishing: bool,
    /// Enter landed before the background grab did: finish() defers
    /// here and set_frame() completes it once the frame exists.
    pending_finish: bool,
    /// First-render clock: the dim layer fades in over DIM_FADE.
    opened: Option<Instant>,
    /// Hover-snap fade-in clock for the current window.
    hover_in: Option<Instant>,
    /// The window the cursor just left, fading out.
    hover_out: Option<(WinRect, Instant)>,
    /// Capture flight: the committed region springs from the selection
    /// rect to the toast's resting corner rect, then the toast
    /// materializes underneath it. The signature transition.
    flight: Option<Flight>,
    /// Set when the background finalize fails; the next render
    /// cancels the session (a failed save loses the capture, never
    /// the daemon).
    finalize_failed: bool,
    /// The background finalize's result (the capture's path and the
    /// toast's scaled pixels), parked here when it lands so the flight
    /// hands it to the toast.
    landed: Option<(PathBuf, Result<crate::stage::Thumb, String>)>,
    /// The config snapshot for this session: render and the key handler
    /// read it every frame, and Config::load() hits the disk each call.
    cfg: iris_lib::config::Config,
    /// The committed-selection hint line, built once per session from
    /// cfg: formatting it per frame allocates a String a frame.
    hint: SharedString,
    /// Idle coordinate chip: the frame pixel it was built for, and the
    /// label. A mousemove inside the same pixel reuses it.
    coord_at: Option<(i64, i64)>,
    coord_label: SharedString,
    /// Selection size label: the physical (w, h) it was built for.
    size_at: Option<(u32, u32)>,
    size_label: SharedString,
}

/// The pooled overlay window: created on the first capture, parked
/// (minimized, never destroyed) afterwards. GPUI window init is the
/// largest single chunk of keypress-to-overlay latency (~130ms), and a
/// parked window skips all of it.
pub static POOL: parking_lot::Mutex<Option<WindowHandle<Overlay>>> = parking_lot::Mutex::new(None);

/// The rect a drag from `anchor` to `cursor` selects, clamped to a
/// window of `size`. `square` (Shift held) locks it to a square that
/// grows away from the anchor in the drag's direction.
/// Mouse move and mouse up share it, so the committed rect is the one
/// the drag showed.
fn drag_region(anchor: (f32, f32), cursor: (f32, f32), size: (f32, f32), square: bool) -> Region {
    let region = Region::from_corners(anchor, cursor, size.0 as u32, size.1 as u32);
    if !square {
        return region;
    }
    let right = cursor.0 >= anchor.0;
    let down = cursor.1 >= anchor.1;
    let side = (region.width.max(region.height) as f32)
        .min(if right { size.0 - anchor.0 } else { anchor.0 })
        .min(if down { size.1 - anchor.1 } else { anchor.1 });
    Region {
        x: if right { anchor.0 } else { anchor.0 - side }.max(0.0) as u32,
        y: if down { anchor.1 } else { anchor.1 - side }.max(0.0) as u32,
        width: side as u32,
        height: side as u32,
    }
}

impl Render for Overlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus.focus(window);
        if self.finalize_failed {
            self.finalize_failed = false;
            self.cancel(window, cx);
            return div().into_any_element();
        }
        if self.flight.is_some() {
            return self.render_flight(window, cx).into_any_element();
        }
        let (sx, sy) = Self::scale(window, self.view);

        // Entrance: the dim fades in over the frozen frame. The frame
        // itself is the desktop's own pixels, so only the dim moves.
        let (dim, fading) = self.entrance_dim();
        if fading {
            window.request_animation_frame();
        }

        let mut root = div()
            .id("overlay")
            .size_full()
            .font_family(theme::FONT)
            .cursor_crosshair()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_key_down(ev, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    if ev.click_count == 2 {
                        this.finish(window, cx);
                        return;
                    }
                    let p = (ev.position.x.into(), ev.position.y.into());
                    // A press on a committed selection's handle reshapes
                    // it instead of starting a fresh drag.
                    if let Some(h) = this.handle_at(p) {
                        this.resize = Some((h, this.current.unwrap()));
                        cx.notify();
                        return;
                    }
                    // A press inside the committed rect (not on a
                    // handle) drags the whole selection.
                    if let Some((x, y, w, h)) = this.current {
                        if p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h {
                            this.moving = Some((p.0 - x, p.1 - y));
                            cx.notify();
                            return;
                        }
                    }
                    this.dragging = true;
                    this.anchor = p;
                    this.current = None;
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _ev: &MouseDownEvent, window, cx| {
                    this.cancel(window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                this.cursor = (mx, my);
                let sf = window.scale_factor();
                let (sx, sy) = Self::scale(window, this.view);
                if let Some((ox, oy)) = this.moving {
                    // Drag the committed rect by its interior, clamped
                    // to the window.
                    if let Some((_, _, w, h)) = this.current {
                        let size = window.bounds().size;
                        let nx = (mx - ox).clamp(0.0, f32::from(size.width) - w);
                        let ny = (my - oy).clamp(0.0, f32::from(size.height) - h);
                        this.current = Some((nx, ny, w, h));
                    }
                    cx.notify();
                    return;
                }
                if let Some((h, r0)) = this.resize {
                    let (mut x0, mut y0, mut x1, mut y1) = (r0.0, r0.1, r0.0 + r0.2, r0.1 + r0.3);
                    match h {
                        0 => {
                            x0 = mx;
                            y0 = my;
                        }
                        1 => {
                            x1 = mx;
                            y0 = my;
                        }
                        2 => {
                            x0 = mx;
                            y1 = my;
                        }
                        3 => {
                            x1 = mx;
                            y1 = my;
                        }
                        4 => {
                            y0 = my;
                        }
                        5 => {
                            y1 = my;
                        }
                        6 => {
                            x0 = mx;
                        }
                        _ => {
                            x1 = mx;
                        }
                    }
                    let size = window.bounds().size;
                    // Clamp BOTH edges before subtracting: using the
                    // raw far edge grows the rect past the window when
                    // the handle is dragged outside it.
                    let (nx, nw) = {
                        let lo = x0.min(x1).clamp(0.0, f32::from(size.width));
                        let hi = x0.max(x1).clamp(0.0, f32::from(size.width));
                        (lo, hi - lo)
                    };
                    let (ny, nh) = {
                        let lo = y0.min(y1).clamp(0.0, f32::from(size.height));
                        let hi = y0.max(y1).clamp(0.0, f32::from(size.height));
                        (lo, hi - lo)
                    };
                    this.current = Some((nx, ny, nw, nh));
                    // The loupe tracks the edge being dragged so a
                    // resize lands on the exact pixel, same as the
                    // initial drag.
                    let (fx, fy) = this.frame_pos(sf, sx, sy, mx, my);
                    this.update_loupe(fx, fy, cx);
                    cx.notify();
                    return;
                }
                if this.dragging {
                    if let Some(old) = this.hovered.take() {
                        this.hover_out = Some((old, Instant::now()));
                    }
                    this.hover_in = None;
                    let size = window.bounds().size;
                    let region = drag_region(
                        this.anchor,
                        (mx, my),
                        (f32::from(size.width), f32::from(size.height)),
                        ev.modifiers.shift,
                    );
                    this.current = Some((
                        region.x as f32,
                        region.y as f32,
                        region.width as f32,
                        region.height as f32,
                    ));
                    let (fx, fy) = this.frame_pos(sf, sx, sy, mx, my);
                    this.update_loupe(fx, fy, cx);
                } else {
                    // Releasing the tile matters: a bare None drop
                    // leaves the loupe's atlas slot allocated until
                    // the window parks.
                    if let Some((img, _)) = this.loupe.take() {
                        crate::widgets::release_render(&img, cx);
                    }
                    this.loupe_at = None;
                    let new_hover = this.window_at(mx, my, sf, sx, sy);
                    let changed = match (this.hovered, new_hover) {
                        (None, None) => false,
                        (Some(a), Some(b)) => {
                            a.x != b.x || a.y != b.y || a.width != b.width || a.height != b.height
                        }
                        _ => true,
                    };
                    if changed {
                        if let Some(old) = this.hovered.take() {
                            this.hover_out = Some((old, Instant::now()));
                        }
                        this.hover_in = new_hover.map(|_| Instant::now());
                        this.hovered = new_hover;
                    }
                    // The idle coordinate readout, the loupe, and the
                    // hover highlight all track the cursor; a committed
                    // selection with none of those does not, so a move
                    // over it skips the render entirely.
                    if changed
                        || this.loupe.is_some()
                        || this.hovered.is_some()
                        || this.current.is_none()
                    {
                        cx.notify();
                    }
                    return;
                }
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, window, cx| {
                    this.handle_mouse_up(ev, window, cx);
                }),
            )
            // The frozen frame, dimmed. Before it lands the window
            // is transparent and the dim lies over the live desktop.
            .children(self.frame_img.clone().map(|i| {
                img(ImageSource::Render(i))
                    .size_full()
                    .object_fit(ObjectFit::Fill)
            }))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(theme::alpha(theme::BG, dim)),
            );

        root = self.render_crosshair_coord(root, window, sx, sy);
        root = self.render_hover_snap(root, window, sx, sy);
        root = self.render_selection(root, window, sx, sy);
        root = self.render_loupe_widget(root, window);

        root.into_any_element()
    }
}
