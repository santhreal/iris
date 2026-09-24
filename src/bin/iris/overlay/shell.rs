use std::{sync::Arc, time::Duration};

use gpui::*;
use iris_lib::capture::WinRect;

use super::{Overlay, POOL};
use crate::theme;

/// The monitor layout and window list for an interactive overlay
/// session, queried before the grab so the shell can map at once.
pub struct ShellLayout {
    pub monitors: Vec<WinRect>,
    pub windows: Vec<WinRect>,
    /// The virtual screen: every monitor's union, the shell's rect
    /// and the frozen frame's own extents.
    pub union: WinRect,
}

pub fn layout() -> ShellLayout {
    // Linux Wayland has no window list or client placement: the capture
    // layer reports an error there and the shell opens fullscreen.
    let (monitors, windows) = iris_lib::capture::layout().unwrap_or((Vec::new(), Vec::new()));
    let union = monitors.iter().skip(1).fold(
        monitors.first().copied().unwrap_or(WinRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }),
        |u, m| {
            let x0 = u.x.min(m.x);
            let y0 = u.y.min(m.y);
            let x1 = (u.x + u.width as i32).max(m.x + m.width as i32);
            let y1 = (u.y + u.height as i32).max(m.y + m.height as i32);
            WinRect {
                x: x0,
                y: y0,
                width: (x1 - x0) as u32,
                height: (y1 - y0) as u32,
            }
        },
    );
    ShellLayout {
        monitors,
        windows,
        union,
    }
}

/// The frozen frame as one display image: the frame's BGRA buffer is
/// moved into the RenderImage with no swizzle (the X11 capture path
/// already produced BGRA), so the frame's only CPU copy IS the
/// GPU-bound buffer. Crop and loupe read pixels back out of it (the
/// swizzle is symmetric). Callers run this off the main thread.
pub fn slice_frame_bgra(width: u32, height: u32, bgra: Vec<u8>) -> Arc<RenderImage> {
    crate::widgets::render_image_from_bgra_owned(width, height, bgra)
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
    if !iris_lib::session::wayland() && (layout.union.width == 0 || layout.union.height == 0) {
        return Err("no monitor layout".into());
    }
    let focus = cx.focus_handle();
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
    // spanning the virtual screen - _NET_WM_STATE_FULLSCREEN would pin
    // it to a single monitor.
    let bounds = {
        // Root space is physical; window bounds are logical.
        let s = crate::sys::window::root_scale(cx);
        let (w, h) = if iris_lib::session::wayland() {
            cx.primary_display()
                .map(|d| (d.bounds().size.width, d.bounds().size.height))
                .unwrap_or((px(1280.0), px(800.0)))
        } else {
            (px(uw as f32 / s), px(uh as f32 / s))
        };
        WindowBounds::Windowed(Bounds {
            origin: point(px(ux as f32 / s), px(uy as f32 / s)),
            size: size(w, h),
        })
    };
    let handle = crate::widgets::open_window(
        cx,
        "Capture - iris",
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
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        },
        |window, cx| {
            crate::sys::window::span_after_map(window, ux, uy, uw, uh);
            cx.new(|_| {
                let cfg = iris_lib::config::Config::load();
                let hint = SharedString::from(format!(
                    "{} capture   {} cancel",
                    cfg.confirm_keybind, cfg.cancel_keybind
                ));
                Overlay {
                    hidden: false,
                    mode: super::OverlayMode::Capture,
                    frame_size: None,
                    origin: (ux, uy),
                    view: (uw, uh),
                    monitors,
                    frame_img: None,
                    windows,
                    focus,
                    dragging: false,
                    anchor: (0.0, 0.0),
                    resize: None,
                    moving: None,
                    current: None,
                    hovered: None,
                    cursor: (0.0, 0.0),
                    loupe: None,
                    loupe_at: None,
                    loupe_scratch: Vec::new(),
                    finishing: false,
                    pending_finish: false,
                    opened: None,
                    hover_in: None,
                    hover_out: None,
                    flight: None,
                    finalize_failed: false,
                    landed: None,
                    cfg,
                    hint,
                    coord_at: None,
                    coord_label: SharedString::default(),
                    size_at: None,
                    size_label: SharedString::default(),
                }
            })
        },
    )
    .map_err(|e| format!("open overlay window: {e}"))?;
    // Pooling needs minimize+restore; elsewhere the window is destroyed
    // on park instead.
    if poolable() {
        *POOL.lock() = Some(handle);
    }
    Ok(handle)
}

/// True where a parked (minimized) overlay can be restored: X11, where
/// `sys::window::unpark_span` maps and re-spans it. Wayland has no
/// unminimize, and Windows and macOS have no restore step.
pub(super) fn poolable() -> bool {
    iris_lib::session::x11()
}

/// Pre-create the overlay at daemon start and park it into the pool,
/// so the first capture reuses a live window instead of paying GPUI's
/// ~130ms renderer init on the hotkey press. The shell maps unfocused
/// and transparent, then minimizes in the same tick: invisible, and it
/// never takes input focus. Only where [`poolable`].
pub fn warmup(cx: &mut App) {
    if !poolable() {
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
pub(super) fn close_other_overlays(cx: &mut App, keep: Option<AnyWindowHandle>) {
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

/// The un-dimmed frame reveal inside a rect: the same full-window image
/// drawn again inside a clipped, bordered rect, offset to align.
pub(super) fn reveal(
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
                .child(
                    img(ImageSource::Render(frame))
                        .size_full()
                        .object_fit(ObjectFit::Fill),
                ),
        )
}
