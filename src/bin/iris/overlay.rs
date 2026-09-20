//! Capture overlay: the fullscreen region picker over a frozen frame.
//!
//! The screen is grabbed once before this window opens; everything the
//! user sees and selects comes out of that frozen frame. Hover snaps to
//! top-level windows (X11 only), dragging commits a region, the loupe
//! tracks the drag edge for pixel-precise crops. Right-click and Esc
//! cancel; Enter, double-click or a plain click on a window finishes.

use std::{path::PathBuf, sync::Arc, time::{Duration, Instant}};

use iris_lib::capture::{Frame, WinRect};
use gpui::*;

use crate::{pipeline, pipeline::Region, stage, theme};

const MIN_SIZE: f32 = 3.0;
const LOUPE_SRC: u32 = 19; // source pixels across the loupe
const LOUPE_ZOOM: u32 = 8;
const LOUPE_PX: u32 = LOUPE_SRC * LOUPE_ZOOM;
/// The dim layer fades in when the overlay appears.
const DIM_FADE: Duration = Duration::from_millis(180);
/// Window-snap reveal fades both ways, like macOS's highlight.
const HOVER_FADE: Duration = Duration::from_millis(90);
/// Edge of a selection resize handle, in logical px.
const HANDLE_PX: f32 = 10.0;

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
    /// The background finalize's result, parked here when it lands so
    /// the flight's last frame can hand it to the toast.
    landed: Option<(PathBuf, PathBuf, u32, u32)>,
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
/// off-screen (never destroyed) afterwards. GPUI window init is the
/// largest single chunk of keypress-to-overlay latency (~130ms), and
/// a parked window skips all of it. The string is the window's class.
pub static POOL: parking_lot::Mutex<Option<(WindowHandle<Overlay>, String)>> =
    parking_lot::Mutex::new(None);

#[derive(Clone)]
struct Flight {
    img: Arc<RenderImage>,
    from: (f32, f32, f32, f32),
    to: (f32, f32, f32, f32),
    started: Instant,
    /// The committing window's screen rect: the toast lands on this
    /// monitor, not wherever the primary display happens to be.
    screen: (f32, f32, f32, f32),
}

/// The monitor layout and window list for an interactive overlay
/// session, queried before the grab so the shell can map at once.
pub struct ShellLayout {
    pub monitors: Vec<WinRect>,
    pub windows: Vec<WinRect>,
    /// The virtual screen: every monitor's union, the shell's rect
    /// and the frozen frame's own extents.
    pub union: WinRect,
}

/// True on a Wayland session: no randr, no window list, no client-side
/// placement. The overlay opens fullscreen and the compositor picks the
/// output; the frame's own size becomes the view when it lands.
fn wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub fn layout() -> ShellLayout {
    #[cfg(target_os = "linux")]
    let (monitors, windows) =
        iris_lib::capture::x11::layout().unwrap_or((Vec::new(), Vec::new()));
    #[cfg(not(target_os = "linux"))]
    let (monitors, windows): (Vec<WinRect>, Vec<WinRect>) = (Vec::new(), Vec::new());
    let union = monitors.iter().skip(1).fold(
        monitors.first().copied().unwrap_or(WinRect { x: 0, y: 0, width: 0, height: 0 }),
        |u, m| {
            let x0 = u.x.min(m.x);
            let y0 = u.y.min(m.y);
            let x1 = (u.x + u.width as i32).max(m.x + m.width as i32);
            let y1 = (u.y + u.height as i32).max(m.y + m.height as i32);
            WinRect { x: x0, y: y0, width: (x1 - x0) as u32, height: (y1 - y0) as u32 }
        },
    );
    ShellLayout { monitors, windows, union }
}

/// The frozen frame as one display image: the frame's RGBA buffer is
/// moved into the RenderImage and swizzled to BGRA in place, so the
/// frame's only CPU copy IS the GPU-bound buffer. Crop and loupe read
/// pixels back out of it (the swizzle is symmetric). Callers run this
/// off the main thread.
pub fn slice_frame(frame: Frame) -> Arc<RenderImage> {
    crate::widgets::render_image_from_rgba_owned(frame.width, frame.height, frame.rgba)
}

/// Open the fullscreen overlay shell over the whole virtual screen,
/// transparent and interactive at once; the frozen frame lands via
/// `Overlay::set_frame`. One window: each GPUI window pays its own
/// renderer init, and a window per monitor doubled the time to
/// first feedback on dual setups.
pub fn open_shell(cx: &mut App, layout: &ShellLayout) -> Result<WindowHandle<Overlay>, String> {
    open_shell_opts(cx, layout, false)
}

/// `prewarm` opens the shell unfocused so the daemon can park it into
/// the pool without stealing input focus at startup. The window still
/// maps (show:true) so GPUI builds its renderer; a `show:false` window
/// never does and the pooled handle comes up dead.
fn open_shell_opts(
    cx: &mut App,
    layout: &ShellLayout,
    prewarm: bool,
) -> Result<WindowHandle<Overlay>, String> {
    if !wayland() && (layout.union.width == 0 || layout.union.height == 0) {
        return Err("no monitor layout".into());
    }
    let focus = cx.focus_handle();
    let win_id = crate::xwin::unique_id("dev.iris.overlay");
    let windows = layout.windows.clone();
    let monitors = layout.monitors.clone();
    let (ux, uy, uw, uh) = (
        layout.union.x,
        layout.union.y,
        layout.union.width,
        layout.union.height,
    );
    // Wayland: the compositor owns placement, so the shell is a
    // fullscreen window and the frame's own size becomes the view when
    // it lands. The bounds still need a nonzero size: GPUI sizes the
    // Vulkan surface from them before the compositor's configure, and
    // a 0x0 swapchain segfaults lavapipe. X11: one windowed shell
    // spanning the virtual screen — _NET_WM_STATE_FULLSCREEN would pin
    // it to a single monitor.
    let bounds = {
        let (w, h) = if wayland() {
            cx.primary_display()
                .map(|d| (d.bounds().size.width, d.bounds().size.height))
                .unwrap_or((px(1280.0), px(800.0)))
        } else {
            (px(uw as f32), px(uh as f32))
        };
        WindowBounds::Windowed(Bounds {
            origin: point(px(ux as f32), px(uy as f32)),
            size: size(w, h),
        })
    };
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(bounds),
                titlebar: None,
                focus: !prewarm,
                show: true,
                kind: WindowKind::Normal,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                display_id: None,
                // Transparent until the frame lands: the live desktop
                // shows through, dimmed, while the grab runs.
                window_background: WindowBackgroundAppearance::Transparent,
                app_id: Some(win_id.clone()),
                window_min_size: None,
                window_decorations: Some(WindowDecorations::Client),
                tabbing_identifier: None,
            },
            move |_, cx| {
                let cfg = iris_lib::config::Config::load();
                let hint = SharedString::from(format!(
                    "{} capture   {} cancel",
                    cfg.confirm_keybind, cfg.cancel_keybind
                ));
                cx.new(|_| Overlay {
                    hidden: false,
                    mode: OverlayMode::Capture,
                    frame_size: None,
                    origin: (ux, uy),
                    view: (uw, uh),
                    monitors,
                    frame_img: None,
                    windows,
                    focus,
                    resize: None,
                    moving: None,
                    current: None,
                    dragging: false,
                    anchor: (0.0, 0.0),
                    hovered: None,
                    cursor: (0.0, 0.0),
                    loupe: None,
                    loupe_at: None,
                    loupe_scratch: Vec::new(),
                    finishing: false,
                    pending_finish: false,
                    coord_at: None,
                    coord_label: SharedString::default(),
                    size_at: None,
                    size_label: SharedString::default(),
                    opened: None,
                    hover_in: None,
                    hover_out: None,
                    flight: None,
                    landed: None,
                    finalize_failed: false,
                    hint,
                    cfg,
                })
            },
        )
        .map_err(|e| format!("open overlay window: {e}"))?;
    crate::xwin::span_after_map(win_id.clone(), ux, uy, uw, uh);
    // Pooling needs minimize+restore, which Wayland cannot do; the
    // window is destroyed on park there instead.
    if !wayland() {
        *POOL.lock() = Some((handle, win_id));
    }
    Ok(handle)
}

/// Pre-create the overlay at daemon start and park it into the pool,
/// so the first capture reuses a live window instead of paying GPUI's
/// ~130ms renderer init on the hotkey press. The shell maps unfocused
/// and transparent, then minimizes in the same tick: invisible, and it
/// never takes input focus. X11 only — Wayland cannot unminimize, so
/// the pool stays empty there.
pub fn warmup(cx: &mut App) {
    if wayland() {
        return;
    }
    if POOL.lock().is_some() {
        return;
    }
    let layout = layout();
    if let Ok(handle) = open_shell_opts(cx, &layout, true) {
        let _ = handle.update(cx, |o, window, cx| o.park(window, cx));
    }
}

/// Close every overlay window except `keep` (the one mid-flight).
/// Destroying a sibling window's renderer inside another window's
/// refresh tick took down the GPU context on software Vulkan, so
/// the closes run on a short timer, off the frame path.
fn close_other_overlays(cx: &mut App, keep: Option<AnyWindowHandle>) {
    let handles: Vec<WindowHandle<Overlay>> = cx
        .windows()
        .into_iter()
        .filter(|h| Some(*h) != keep)
        .filter_map(|h| h.downcast::<Overlay>())
        .collect();
    if handles.is_empty() {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(80))
            .await;
        let _ = cx.update(|cx| {
            for h in handles {
                let _ = h.update(cx, |_, window, _| window.remove_window());
            }
        });
    })
    .detach();
}

/// BMP for the monitor slices, written straight from the frame: a
/// 32bpp BI_RGB header plus bottom-up BGRA rows. One pass over the
/// the cursor, plus the crosshair-free info line (coords + hex).
fn loupe_image(
    img: &Arc<RenderImage>,
    width: u32,
    height: u32,
    fx: i64,
    fy: i64,
    scratch: &mut Vec<u8>,
) -> (Arc<RenderImage>, SharedString) {
    let bgra = img.as_bytes(0).unwrap_or(&[]);
    // Uninit, not zeroed: the row fill writes every byte, so a 92KB
    // memset before the fill is a wasted pass per mousemove. The
    // reserve is load-bearing: mem::take below hands the buffer to
    // the RenderImage, so the next rebuild starts from capacity 0
    // and set_len without it writes into a dangling allocation.
    scratch.clear();
    scratch.reserve((LOUPE_PX * LOUPE_PX * 4) as usize);
    #[allow(clippy::uninit_vec)] // the row fill writes every byte
    unsafe { scratch.set_len((LOUPE_PX * LOUPE_PX * 4) as usize) };
    // The scratch holds BGRA, the RenderImage's own order: the frame
    // bytes copy straight in with no swizzle pass either way.
    let out = scratch.as_mut_slice();
    let half = LOUPE_SRC as i64 / 2;
    let mut center = [0u8, 0u8, 0u8];
    let row_px = LOUPE_PX as usize;
    for sy in 0..LOUPE_SRC as i64 {
        // Build one zoomed row (each source pixel becomes LOUPE_ZOOM
        // horizontal copies), then memcpy it down LOUPE_ZOOM rows:
        // 19 row builds + 152 row copies instead of 23k pixel writes.
        let row_start = (sy as u32 * LOUPE_ZOOM) as usize * row_px * 4;
        let row = &mut out[row_start..row_start + row_px * 4];
        for sx in 0..LOUPE_SRC as i64 {
            let px_x = fx + sx - half;
            let px_y = fy + sy - half;
            let inside =
                px_x >= 0 && px_y >= 0 && px_x < width as i64 && px_y < height as i64;
            let src = if inside {
                let i = ((px_y as u32 * width + px_x as u32) * 4) as usize;
                [bgra[i], bgra[i + 1], bgra[i + 2], 255]
            } else {
                [0x14, 0x14, 0x16, 255]
            };
            if sx == half && sy == half {
                center = [src[2], src[1], src[0]];
            }
            let dx = (sx as u32 * LOUPE_ZOOM) as usize * 4;
            for bx in 0..LOUPE_ZOOM as usize {
                row[dx + bx * 4..dx + bx * 4 + 4].copy_from_slice(&src);
            }
        }
        for by in 1..LOUPE_ZOOM as usize {
            let dst = row_start + by * row_px * 4;
            let (head, tail) = out.split_at_mut(dst);
            tail[..row_px * 4].copy_from_slice(&head[row_start..row_start + row_px * 4]);
        }
    }
    // Crosshair on the center pixel.
    let mid = LOUPE_PX / 2;
    let z = LOUPE_ZOOM;
    for i in 0..z {
        for (x, y) in [
            (mid - z / 2 + i, mid - z / 2),
            (mid - z / 2 + i, mid + z / 2 - 1),
            (mid - z / 2, mid - z / 2 + i),
            (mid + z / 2 - 1, mid - z / 2 + i),
        ] {
            let d = ((y * LOUPE_PX + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&[255, 255, 255, 242]);
        }
    }
    // Straight into a RenderImage: this rebuilds on every drag
    // mousemove, so an encode/decode round trip is out of the question.
    // The scratch moves in wholesale; next rebuild allocates fresh.
    let buf = image::RgbaImage::from_raw(LOUPE_PX, LOUPE_PX, std::mem::take(scratch))
        .expect("loupe buffer size");
    let img = Arc::new(RenderImage::new([image::Frame::new(buf)]));
    let info = SharedString::from(format!(
        "{fx}, {fy}  #{:02x}{:02x}{:02x}",
        center[0], center[1], center[2]
    ));
    (img, info)
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
        if wayland() {
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
        if !wayland() {
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

    /// The 8 resize handles of a committed selection, in logical px:
    /// 4 corners (NW, NE, SW, SE) then 4 edges (N, S, W, E). Each is a
    /// HANDLE_PX square centered on the point it drags.
    fn handles(x: f32, y: f32, w: f32, h: f32) -> [(f32, f32); 8] {
        [
            (x, y),
            (x + w, y),
            (x, y + h),
            (x + w, y + h),
            (x + w / 2.0, y),
            (x + w / 2.0, y + h),
            (x, y + h / 2.0),
            (x + w, y + h / 2.0),
        ]
    }

    /// Which resize handle, if any, the logical point `p` is inside.
    fn handle_at(&self, p: (f32, f32)) -> Option<usize> {
        let (x, y, w, h) = self.current?;
        let half = HANDLE_PX / 2.0;
        Self::handles(x, y, w, h)
            .iter()
            .position(|(hx, hy)| {
                p.0 >= hx - half && p.0 <= hx + half && p.1 >= hy - half && p.1 <= hy + half
            })
    }

    /// Frame-pixel hit test for hover-snap. The window list is in frame
    /// (physical) pixels; the cursor arrives logical.
    fn window_at(&self, cx: f32, cy: f32, sf: f32, sx: f32, sy: f32) -> Option<WinRect> {
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
    fn to_logical(
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

    fn scale(window: &Window, view: (u32, u32)) -> (f32, f32) {
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
    fn rect_to_frame(
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
    fn frame_pos(&self, sf: f32, sx: f32, sy: f32, lx: f32, ly: f32) -> (i64, i64) {
        (
            self.origin.0 as i64 + (lx * sf * sx) as i64,
            self.origin.1 as i64 + (ly * sf * sy) as i64,
        )
    }
    /// Rebuild the loupe for frame pixel (fx, fy). The scratch buffer
    /// is reused across rebuilds so a drag does not alloc/free a 92KB
    /// image per mousemove.
    fn update_loupe(&mut self, fx: i64, fy: i64, cx: &mut Context<Self>) {
        if self.loupe_at == Some((fx, fy)) {
            return;
        }
        self.loupe_at = Some((fx, fy));
        if let (Some(img), Some((w, h))) = (&self.frame_img, self.frame_size) {
            let img = img.clone();
            let (w, h) = (w, h);
            let built = loupe_image(&img, w, h, fx, fy, &mut self.loupe_scratch);
            // The previous loupe's atlas tile goes with it: a drag
            // mints one per mousemove.
            let old = self.loupe.replace(built);
            if let Some((img, _)) = old {
                crate::widgets::release_render(&img, cx);
            }
        }
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            // recording chip takes over the visual state.
            let _ = crate::daemon::dispatch(
                cx,
                &crate::daemon::Command::RecordRegion {
                    x: region.x as i32,
                    y: region.y as i32,
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
                return;
            }
        };
        pipeline::play_shutter_sound();
        // Every other monitor's overlay leaves with the commit; only
        // this window stays for the flight.
        close_other_overlays(cx, Some(window.window_handle()));
        let (cw, ch) = (crop.width(), crop.height());
        // The flight needs pixels now, not after an encode/decode
        // round trip: a straight swizzle of the crop into a
        // RenderImage. The PNG encode, thumbnail and disk write run
        // behind the flight and land mid-animation.
        let flight_img = crate::widgets::render_image_from_rgba(cw, ch, crop.as_raw());
        let finalize = cx
            .background_executor()
            .spawn(async move { pipeline::finalize(crop) });
        cx.spawn(async move |this, cx| {
            let result = finalize.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((path, entry)) => {
                    this.landed = Some((path, entry.thumb, entry.width, entry.height));
                    cx.notify();
                }
                Err(e) => {
                    // A failed save (full disk, unwritable dir) loses
                    // the capture; the daemon and the overlay recover.
                    iris_lib::ilog!("iris: capture: {e}");
                    this.finalize_failed = true;
                    cx.notify();
                }
            });
        })
        .detach();
        let show_toast = self.cfg.show_toast_after_capture;
        if show_toast {
            // The toast lands on the monitor under the selection's
            // center, in that monitor's own bottom-right corner.
            let (fcx, fcy) = (
                region.x as f32 + region.width as f32 / 2.0,
                region.y as f32 + region.height as f32 / 2.0,
            );
            let sf = window.scale_factor();
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
    }    /// Release this window's painted images: with the window pooled
    /// across sessions, an unreleased tile would outlive its session.
    fn release_assets(&mut self, cx: &mut App) {
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
    /// fight it, and the GPU surface is freed while iconic. Wayland has
    /// no unminimize, so the window is destroyed there instead.
    fn park(&mut self, window: &mut Window, cx: &mut App) {
        self.release_assets(cx);
        self.hidden = true;
        if wayland() {
            window.remove_window();
        } else {
            window.minimize_window();
        }
    }

    pub(crate) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_finish = false;
        close_other_overlays(cx, Some(window.window_handle()));
        self.park(window, cx);
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
        let opened = *self.opened.get_or_insert_with(Instant::now);
        let dim_t = (opened.elapsed().as_secs_f32()
            / crate::motion::tempo(DIM_FADE).as_secs_f32())
        .min(1.0);
        let dim = 0.32 * crate::motion::ease_out(dim_t);
        if dim_t < 1.0 {
            window.request_animation_frame();
        }

        let mut root = div()
            .id("overlay")
            .size_full()
            .font_family(theme::FONT)
            .cursor_crosshair()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                let shift = ev.keystroke.modifiers.shift;
                let dir = match key {
                    "left" => Some((-1.0, 0.0)),
                    "right" => Some((1.0, 0.0)),
                    "up" => Some((0.0, -1.0)),
                    "down" => Some((0.0, 1.0)),
                    _ => None,
                };
                if let (Some((dx, dy)), Some((x, y, w, h))) = (dir, this.current) {
                    let step = if shift { 10.0 } else { 1.0 };
                    let size = window.bounds().size;
                    let nx = (x + dx * step).clamp(0.0, f32::from(size.width) - w);
                    let ny = (y + dy * step).clamp(0.0, f32::from(size.height) - h);
                    this.current = Some((nx, ny, w, h));
                    cx.notify();
                    return;
                }
                // Number keys snap the selection to that monitor.
                if let Ok(d) = key.parse::<usize>() {
                    if d >= 1 && d <= this.monitors.len() {
                        let m = this.monitors[d - 1];
                        let sf = window.scale_factor();
                        let (sx, sy) = Self::scale(window, this.view);
                        this.current = Some(Self::to_logical(this.origin, &m, sf, sx, sy));
                        cx.notify();
                        return;
                    }
                }
                // 'c' copies the loupe's center hex while dragging.
                if key == "c" {
                    if let Some((_, info)) = &this.loupe {
                        if let Some(hex) = info.split('#').nth(1) {
                            let text = format!("#{hex}");
                            let _ = pipeline::with_clipboard(|c| {
                                let _ = iris_lib::dragcopy::clipboard_set_text(c, &text);
                            });
                        }
                    }
                    cx.notify();
                    return;
                }
                let cfg = &this.cfg;
                let m = &ev.keystroke.modifiers;
                if iris_lib::config::keybind_matches(
                    &cfg.cancel_keybind, key, m.control, m.shift, m.alt, m.platform,
                ) {
                    this.cancel(window, cx);
                } else if iris_lib::config::keybind_matches(
                    &cfg.confirm_keybind, key, m.control, m.shift, m.alt, m.platform,
                ) {
                    this.finish(window, cx);
                }
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
                    let (mut x0, mut y0, mut x1, mut y1) =
                        (r0.0, r0.1, r0.0 + r0.2, r0.1 + r0.3);
                    match h {
                        0 => { x0 = mx; y0 = my; }
                        1 => { x1 = mx; y0 = my; }
                        2 => { x0 = mx; y1 = my; }
                        3 => { x1 = mx; y1 = my; }
                        4 => { y0 = my; }
                        5 => { y1 = my; }
                        6 => { x0 = mx; }
                        _ => { x1 = mx; }
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
                    let mut region = Region::from_corners(
                        this.anchor,
                        (mx, my),
                        window.bounds().size.width.into(),
                        window.bounds().size.height.into(),
                    );
                    // Shift locks the drag to a square, like macOS.
                    if ev.modifiers.shift {
                        let side = region.width.max(region.height);
                        // Grow away from the anchor in the drag's
                        // direction, clamped to the window.
                        let size = window.bounds().size;
                        let (wmax, hmax) =
                            (f32::from(size.width), f32::from(size.height));
                        let right = mx >= this.anchor.0;
                        let down = my >= this.anchor.1;
                        let side = (side as f32)
                            .min(if right { wmax - this.anchor.0 } else { this.anchor.0 })
                            .min(if down { hmax - this.anchor.1 } else { this.anchor.1 });
                        region = Region {
                            x: if right { this.anchor.0 } else { this.anchor.0 - side }
                                .max(0.0) as u32,
                            y: if down { this.anchor.1 } else { this.anchor.1 - side }
                                .max(0.0) as u32,
                            width: side as u32,
                            height: side as u32,
                        };
                    }
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
                    // A handle or interior release ends the reshape or
                    // move; the rect stays armed for Enter.
                    if this.resize.take().is_some() || this.moving.take().is_some() {
                        if let Some((img, _)) = this.loupe.take() {
                            this.loupe_at = None;
                            crate::widgets::release_render(&img, cx);
                        }
                        cx.notify();
                        return;
                    }
                    if !this.dragging {
                        return;
                    }
                    this.dragging = false;
                    if let Some((img, _)) = this.loupe.take() {
                        this.loupe_at = None;
                        crate::widgets::release_render(&img, cx);
                    }
                    let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                    let moved =
                        ((mx - this.anchor.0).powi(2) + (my - this.anchor.1).powi(2)).sqrt();
                    if moved < 4.0 {
                        // A click (no drag) on a window captures that window.
                        let sf = window.scale_factor();
                        let (sx, sy) = Self::scale(window, this.view);
                        if let Some(w) = this.window_at(mx, my, sf, sx, sy) {
                            this.current = Some(Self::to_logical(this.origin, &w, sf, sx, sy));
                            this.hovered = None;
                            this.finish(window, cx);
                        } else {
                            this.current = None;
                        }
                        cx.notify();
                        return;
                    }
                    let size = window.bounds().size;
                    let region = Region::from_corners(
                        this.anchor,
                        (mx, my),
                        size.width.into(),
                        size.height.into(),
                    );
                    if (region.width as f32) < MIN_SIZE || (region.height as f32) < MIN_SIZE {
                        this.current = None;
                    } else {
                        this.current = Some((
                            region.x as f32,
                            region.y as f32,
                            region.width as f32,
                            region.height as f32,
                        ));
                    }
                    cx.notify();
                }),
            )
            // The frozen frame, dimmed. Before it lands the window
            // is transparent and the dim lies over the live desktop.
            .children(
                self.frame_img
                    .clone()
                    .map(|i| img(ImageSource::Render(i)).size_full().object_fit(ObjectFit::Fill)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(theme::alpha(theme::BG, dim)),
            );

        // Idle crosshair coordinates, like macOS region capture:
        // physical pixels next to the cursor, flipping at the edges.
        if !self.dragging && self.current.is_none() && self.hovered.is_none() {
            let sf = window.scale_factor();
            let win = window.bounds().size;
            let (fx, fy) = self.frame_pos(sf, sx, sy, self.cursor.0, self.cursor.1);
            if self.coord_at != Some((fx, fy)) {
                self.coord_at = Some((fx, fy));
                self.coord_label = SharedString::from(format!("{fx}, {fy}"));
            }
            let coord_label = self.coord_label.clone();
            let lx = if self.cursor.0 + 90.0 > f32::from(win.width) {
                self.cursor.0 - 82.0
            } else {
                self.cursor.0 + 18.0
            };
            let ly = if self.cursor.1 + 40.0 > f32::from(win.height) {
                self.cursor.1 - 30.0
            } else {
                self.cursor.1 + 16.0
            };
            root = root.child(
                div()
                    .absolute()
                    .left(px(lx))
                    .top(px(ly))
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .bg(theme::alpha(theme::BG_ELEV, 0.9))
                    .text_xs()
                    .child(coord_label),
            );
        }

        // Hover-snap highlight: un-dimmed reveal of the window's
        // rect, fading in; the window just left fades out behind it.
        // Before the frame lands there is nothing to reveal.
        if !self.dragging && self.frame_img.is_some() {
            if let Some((old, since)) = self.hover_out {
                let t = (since.elapsed().as_secs_f32()
                    / crate::motion::tempo(HOVER_FADE).as_secs_f32())
                .min(1.0);
                if t >= 1.0 {
                    self.hover_out = None;
                } else {
                    let sf = window.scale_factor();
                    let (rx, ry, rw, rh) = Self::to_logical(self.origin, &old, sf, sx, sy);
                    root = root.child(
                        reveal(self.frame_img.clone().expect("checked"), rx, ry, rw, rh, window)
                            .opacity(1.0 - t),
                    );
                    window.request_animation_frame();
                }
            }
            if let Some(w) = self.hovered {
                let sf = window.scale_factor();
                let (rx, ry, rw, rh) = Self::to_logical(self.origin, &w, sf, sx, sy);
                let mut alpha = 1.0f32;
                if let Some(since) = self.hover_in {
                    let t = (since.elapsed().as_secs_f32()
                        / crate::motion::tempo(HOVER_FADE).as_secs_f32())
                    .min(1.0);
                    alpha = t;
                    if t < 1.0 {
                        window.request_animation_frame();
                    }
                }
                root = root.child(
                    reveal(self.frame_img.clone().expect("checked"), rx, ry, rw, rh, window).opacity(alpha),
                );
            }
        }

        // Committed or in-progress selection.
        if let Some((x, y, w, h)) = self.current {
            let sf = window.scale_factor();
            let phys = (
                (w * sf * sx).round() as u32,
                (h * sf * sy).round() as u32,
            );
            if self.size_at != Some(phys) {
                self.size_at = Some(phys);
                self.size_label = SharedString::from(format!("{} × {}", phys.0, phys.1));
            }
            let size_label = self.size_label.clone();
            if let Some(fi) = self.frame_img.clone() {
                root = root.child(reveal(fi, x, y, w, h, window));
            }
            root = root.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(if y < 44.0 { y + 4.0 } else { y - 28.0 }))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(6.))
                    .bg(theme::alpha(theme::BG_ELEV, 0.9))
                    .text_xs()
                    .text_color(theme::FG)
                    .child(size_label),
            );
            // A committed selection (drag released, not yet captured)
            // shows what confirms and what cancels. While still
            // dragging the loupe is the feedback; the hint would
            // flicker under it.
            if !self.dragging {
                let hint = self.hint.clone();
                root = root.child(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y + h + 6.0))
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(px(6.))
                        .bg(theme::alpha(theme::BG_ELEV, 0.9))
                        .text_xs()
                        .text_color(theme::FG_DIM)
                        .child(hint),
                );
            }
            // Resize handles on a committed selection: 8 grab points
            // the cursor can pull to reshape the rect before capture.
            if !self.dragging {
                for (hx, hy) in Self::handles(x, y, w, h) {
                    root = root.child(
                        div()
                            .absolute()
                            .left(px(hx - HANDLE_PX / 2.0))
                            .top(px(hy - HANDLE_PX / 2.0))
                            .w(px(HANDLE_PX))
                            .h(px(HANDLE_PX))
                            .rounded(px(2.))
                            .bg(theme::FG)
                            .border_1()
                            .border_color(theme::alpha(theme::BG, 0.6))
                            .shadow(theme::shadow_float()),
                    );
                }
            }
        }

        // Loupe while dragging or resizing: the window-snap highlight
        // is the hover feedback; no circle chasing the cursor. Flips
        // to the other side of the cursor near the screen edges.
        if self.dragging || self.resize.is_some() {
            if let Some((loupe, info)) = &self.loupe {
                let win = window.bounds().size;
                let span = LOUPE_PX as f32 + 30.0;
                let lx = if self.cursor.0 + 24.0 + span > f32::from(win.width) {
                    self.cursor.0 - 24.0 - LOUPE_PX as f32
                } else {
                    self.cursor.0 + 24.0
                };
                let ly = if self.cursor.1 + 24.0 + span > f32::from(win.height) {
                    self.cursor.1 - 24.0 - LOUPE_PX as f32
                } else {
                    self.cursor.1 + 24.0
                };
                root = root.child(
                    div()
                        .absolute()
                        .left(px(lx))
                        .top(px(ly))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .w(px(LOUPE_PX as f32))
                                .h(px(LOUPE_PX as f32))
                                .rounded(px(theme::RADIUS_SM))
                                .overflow_hidden()
                                .border_1()
                                .border_color(theme::HAIRLINE)
                                // Lift the loupe off the dimmed desktop so
                                // it reads as floating chrome, like the
                                // toast and chip.
                                .shadow(theme::shadow_float())
                                .child(img(ImageSource::Render(loupe.clone())).size_full()),
                        )
                        .child(
                            div()
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(6.))
                                .bg(theme::alpha(theme::BG_ELEV, 0.9))
                                .text_xs()
                                .text_color(theme::FG_DIM)
                                .child(info.clone()),
                        ),
                );
            }
        }

        root.into_any_element()
    }
}

impl Overlay {
    /// The capture flight: the committed region springs from the
    /// selection rect to the toast's corner rect while the dim lifts,
    /// then the toast appears underneath and the overlay closes.
    fn render_flight(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        const FLIGHT: Duration = Duration::from_millis(500);
        let f = self.flight.clone().expect("flight checked");
        let t = (f.started.elapsed().as_secs_f32()
            / crate::motion::tempo(FLIGHT).as_secs_f32())
        .min(1.0);
        if t >= 1.0 && self.landed.is_some() {
            self.flight = None;
            // The flight image's atlas tile must go before the park:
            // release_assets only looks at live fields, and the window
            // now outlives the session.
            crate::widgets::release_render(&f.img, cx);
            // Window ops from inside a render are dropped by GPUI's
            // effect queue; defer the toast to after this frame.
            let (path, thumb, w, h) = self.landed.take().expect("landed checked");
            let screen = f.screen;
            cx.defer(move |cx| {
                // A toast that cannot open loses the notification,
                // not the daemon: the capture is already on disk.
                if let Err(e) = stage::show_toast_landed(cx, &path, &thumb, w, h, Some(screen)) {
                    iris_lib::ilog!("iris: toast: {e}");
                }
            });
            self.park(window, cx);
        } else {
            // Still flying, or parked at rest until the background
            // finalize lands: a slow disk must not strand the card.
            window.request_animation_frame();
        }
        let e = crate::motion::spring(t);
        let (fx, fy, fw, fh) = f.from;
        let (tx, ty, tw, th) = f.to;
        let (x, y, w, h) = (
            fx + (tx - fx) * e,
            fy + (ty - fy) * e,
            fw + (tw - fw) * e,
            fh + (th - fh) * e,
        );
        div()
            .size_full()
            // The undimmed frozen frame: the desktop as it was.
            .children(
                self.frame_img
                    .clone()
                    .map(|i| img(ImageSource::Render(i)).size_full().object_fit(ObjectFit::Fill)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w.max(1.0)))
                    .h(px(h.max(1.0)))
                    .rounded(px(12.))
                    .overflow_hidden()
                    .shadow(theme::shadow_float())
                    .child(img(ImageSource::Render(f.img.clone())).size_full().object_fit(ObjectFit::Fill)),
            )
    }
}

/// The un-dimmed frame reveal inside a rect: the same full-window image
/// drawn again inside a clipped, bordered rect, offset to align.
fn reveal(
    frame: Arc<RenderImage>,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    window: &Window,
) -> Div {
    let win = window.bounds().size;
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .overflow_hidden()
        .border_1()
        .border_color(theme::ACCENT)
        .child(
            div()
                .absolute()
                .left(px(-x))
                .top(px(-y))
                .w(win.width)
                .h(win.height)
                .child(img(ImageSource::Render(frame)).size_full().object_fit(ObjectFit::Fill)),
        )
}

// WHY: the committed-selection handle layout is a fixed contract —
// 4 corners then 4 edges, each centered on the point it drags. A
// reordered or off-center handle breaks resize hit-testing. Not
// covered: the GPU paint of the handles.
#[cfg(test)]
mod tests {
    use super::Overlay;

    #[test]
    fn handles_are_corners_then_edges() {
        let h = Overlay::handles(10.0, 20.0, 100.0, 50.0);
        // Corners NW NE SW SE.
        assert_eq!(h[0], (10.0, 20.0));
        assert_eq!(h[1], (110.0, 20.0));
        assert_eq!(h[2], (10.0, 70.0));
        assert_eq!(h[3], (110.0, 70.0));
        // Edges N S W E, centered on the midpoint.
        assert_eq!(h[4], (60.0, 20.0));
        assert_eq!(h[5], (60.0, 70.0));
        assert_eq!(h[6], (10.0, 45.0));
        assert_eq!(h[7], (110.0, 45.0));
    }
}
