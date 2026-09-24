//! Window placement, stacking, and window manager moves behind one
//! facade.
//!
//! GPUI sets a window's origin, frame, and pop-up stacking before the
//! map, where every platform's window manager reads them. An X11 window
//! manager applies two requests only to a window it manages: a rect
//! over several monitors and stacking above for a normal window; the
//! X11 submodule sends them once the window is managed. The
//! cross-platform helpers (root scale, monitor-aware centering) are
//! defined here; the per-OS parts are in submodules.

/// Root pixels per GPUI logical pixel. Root space (randr monitors,
/// window rects, captured frames) is physical; GPUI window bounds are
/// logical. X11 defines no logical unit: GPUI's X11 backend picks one
/// scale for every window and reports it on its display. macOS points
/// and Windows DIPs are platform units the capture layer reads. Under
/// Wayland the compositor places windows and root space is unread.
pub fn root_scale(cx: &gpui::App) -> f32 {
    #[cfg(target_os = "linux")]
    {
        if iris_lib::session::wayland() {
            return 1.0;
        }
        cx.primary_display()
            .and_then(|d| d.scale_factor())
            .unwrap_or(1.0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cx;
        iris_lib::capture::root_scale()
    }
}

/// The primary monitor's rect in logical pixels, falling back to
/// GPUI's primary display. On X11 randr reports the real per-monitor
/// geometry where GPUI's primary_display() spans the whole virtual
/// screen; elsewhere GPUI's answer is already correct.
pub fn primary_monitor_rect(cx: &gpui::App) -> Option<(f32, f32, f32, f32)> {
    #[cfg(target_os = "linux")]
    if !iris_lib::session::wayland() {
        if let Ok(monitors) = iris_lib::capture::x11::monitors() {
            if let Some(m) = monitors.first() {
                let s = root_scale(cx);
                return Some((
                    m.x as f32 / s,
                    m.y as f32 / s,
                    m.width as f32 / s,
                    m.height as f32 / s,
                ));
            }
        }
    }
    cx.primary_display().map(|d| {
        let b = d.bounds();
        (
            b.origin.x.into(),
            b.origin.y.into(),
            b.size.width.into(),
            b.size.height.into(),
        )
    })
}

/// Window origin that centers a `w`x`h` window on the primary display,
/// or `fallback` when no display information is available.
pub fn centered_origin(cx: &gpui::App, w: f32, h: f32, fallback: (f32, f32)) -> (f32, f32) {
    let Some((bx, by, sw, sh)) = primary_monitor_rect(cx) else {
        return fallback;
    };
    (bx + (sw - w) / 2.0, by + (sh - h) / 2.0)
}

// ---- window manager state ---------------------------------------------

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
mod x11;

/// Keep `window` above other windows once the X11 window manager
/// manages it (_NET_WM_STATE_ABOVE). A pop-up is above from its map.
pub fn keep_above(window: &gpui::Window) {
    #[cfg(target_os = "linux")]
    if let Some(xid) = x11::xid(window) {
        x11::always_on_top_after_map(xid);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = window;
}

/// Span the X11 virtual screen with `window` once the window manager
/// manages it: the union rect (`x`, `y`, `w`, `h`, root pixels), above
/// other windows. A window manager maps a window within one monitor;
/// _NET_WM_STATE_FULLSCREEN also pins it to one.
pub fn span_after_map(window: &gpui::Window, x: i32, y: i32, w: u32, h: u32) {
    #[cfg(target_os = "linux")]
    if let Some(xid) = x11::xid(window) {
        x11::span_after_map(xid, x, y, w, h);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, x, y, w, h);
}

/// Bring the parked (minimized) X11 `window` back over the virtual
/// screen: the union rect (root pixels), mapped, above, focused.
pub fn unpark_span(window: &gpui::Window, x: i32, y: i32, w: u32, h: u32) {
    #[cfg(target_os = "linux")]
    if let Some(xid) = x11::xid(window) {
        x11::unpark_span(xid, x, y, w, h);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, x, y, w, h);
}

/// Hand a left-button drag to the window manager so it moves `window`.
/// `grab` is the press's window-local position. Call while the button
/// is held: on the press, or on a motion after it.
pub fn begin_wm_move(window: &gpui::Window, grab: gpui::Point<gpui::Pixels>) {
    #[cfg(target_os = "linux")]
    {
        // Wayland: the compositor moves the window (xdg_toplevel.move).
        // X11: the window manager moves it on _NET_WM_MOVERESIZE, with
        // its snapping and tiling; under one that does not handle that,
        // the window follows the root pointer.
        if iris_lib::session::wayland() || x11::wm_moveresize() {
            window.start_window_move();
        } else if let Some(xid) = x11::xid(window) {
            let s = window.scale_factor();
            x11::begin_wm_move(
                xid,
                (
                    (f32::from(grab.x) * s).round() as i32,
                    (f32::from(grab.y) * s).round() as i32,
                ),
            );
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = grab;
        #[cfg(windows)]
        windows::begin_move(window);
        #[cfg(target_os = "macos")]
        macos::begin_move(window);
    }
}

/// Whether a resizable window draws its own resize edges. Linux windows
/// use client-side decorations and have no frame to grab; Windows
/// resizes through the frame's hit test and macOS through AppKit.
pub const CLIENT_RESIZE: bool = cfg!(target_os = "linux");

/// Hand a left-button press on a window edge to a resize from `edge`.
/// `grab` is the press's window-local position; `min` is the window's
/// minimum logical size, where the X11 resize stops.
pub fn begin_wm_resize(
    window: &gpui::Window,
    edge: gpui::ResizeEdge,
    grab: gpui::Point<gpui::Pixels>,
    min: gpui::Size<gpui::Pixels>,
) {
    #[cfg(target_os = "linux")]
    {
        // Wayland: the compositor resizes (xdg_toplevel.resize) and
        // enforces the minimum from xdg_toplevel.set_min_size. X11: the
        // window manager resizes on _NET_WM_MOVERESIZE and enforces the
        // minimum from WM_NORMAL_HINTS; under one that does not handle
        // that, the window follows the root pointer and stops at `min`.
        if iris_lib::session::wayland() || x11::wm_moveresize() {
            window.start_window_resize(edge);
        } else if let Some(xid) = x11::xid(window) {
            let s = window.scale_factor();
            let root = |v: gpui::Pixels| (f32::from(v) * s).round() as i32;
            x11::begin_wm_resize(
                xid,
                (root(grab.x), root(grab.y)),
                edge,
                (root(min.width), root(min.height)),
            );
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, edge, grab, min);
}

/// A double-click on a window's title bar runs the platform's action:
/// on macOS the one set in Desktop & Dock (zoom, minimize, or none), on
/// Windows and Linux a maximize toggle.
pub fn title_double_click(window: &gpui::Window) {
    #[cfg(target_os = "macos")]
    window.titlebar_double_click();
    #[cfg(windows)]
    windows::toggle_maximize(window);
    #[cfg(target_os = "linux")]
    window.zoom_window();
}

/// The zoom (green) window control: fullscreen on macOS, where that is
/// the button's action; the maximize toggle elsewhere.
pub fn zoom_control(window: &gpui::Window) {
    #[cfg(target_os = "macos")]
    window.toggle_fullscreen();
    #[cfg(not(target_os = "macos"))]
    title_double_click(window);
}

/// Drag `paths` out of `window` as files, from the press in progress.
/// macOS begins the session from the window's view and current event;
/// X11 and Windows track the pointer themselves. `icon` is the X11 drag
/// image; the Windows and macOS shells draw their own.
pub fn start_file_drag(
    window: &gpui::Window,
    paths: Vec<std::path::PathBuf>,
    icon: Option<iris_lib::dragcopy::DragIcon>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = icon;
        let view = macos::ns_view(window).ok_or("drag: no window view")?;
        iris_lib::dragcopy::start_file_drag_from_view(view, paths)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        iris_lib::dragcopy::start_file_drag_at_cursor(paths, icon)
    }
}

/// Finish opening the recording chip `handle`, and return its X11 id (0
/// elsewhere). Windows and macOS keep the chip out of screen captures,
/// so a chip over a recorded region is not in the recording. X11 has no
/// such flag; the chip there sits outside the recorded rect instead.
pub fn prepare_chip(cx: &mut gpui::App, handle: gpui::AnyWindowHandle) -> u32 {
    #[cfg(target_os = "linux")]
    let setup = |window: &gpui::Window| x11::xid(window).unwrap_or(0);
    #[cfg(windows)]
    let setup = |window: &gpui::Window| {
        windows::exclude_from_capture(window);
        0
    };
    #[cfg(target_os = "macos")]
    let setup = |window: &gpui::Window| {
        macos::exclude_from_capture(window);
        0
    };
    handle.update(cx, |_, window, _| setup(window)).unwrap_or(0)
}

// ---- present report ---------------------------------------------------

/// A report of the next frame a window presents, for a handoff that
/// must not uncover the screen behind a window before its pixels are
/// there. GPUI paces X11 frames with a timer, and a present reaches the
/// X server after the tick that issued it, so the X11 watch reports the
/// window's damage instead. Wayland frame callbacks, the macOS display
/// link, and DWM's vblank start a frame tick after the compositor's
/// last composition; there `open` returns None and a caller waits one
/// more frame tick.
pub struct PresentWatch(PresentImpl);

#[cfg(target_os = "linux")]
type PresentImpl = x11::PresentWatch;
#[cfg(not(target_os = "linux"))]
type PresentImpl = std::convert::Infallible;

impl PresentWatch {
    /// Prepare a report on `window` that lapses at `until`. Opening
    /// connects to the X server; open before the frame callback that
    /// arms the watch.
    pub fn open(window: &gpui::Window, until: std::time::Instant) -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            x11::PresentWatch::open(x11::xid(window)?, until).map(Self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (window, until);
            None
        }
    }

    /// Arm the watch inside a frame callback, before the frame to watch
    /// draws. The receiver resolves once that frame, or a later one, is
    /// on the window, and is canceled when the watch lapses.
    pub fn arm(self) -> Option<futures::channel::oneshot::Receiver<()>> {
        #[cfg(target_os = "linux")]
        {
            self.0.arm()
        }
        #[cfg(not(target_os = "linux"))]
        match self.0 {}
    }
}
