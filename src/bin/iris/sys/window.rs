//! Window placement and post-map fixups behind one facade.
//!
//! GPUI opens a window at a requested origin, but every platform's
//! window manager can override that at map time: X11 WMs apply their
//! own placement, Windows snaps to its own rules, macOS centers or
//! cascades. The fixup layer reasserts the intended geometry after the
//! map. The cross-platform helpers (unique window ids, monitor-aware
//! centering) live here; the per-OS fixup lives in a submodule.

/// A process-unique window class: surfaces that can have several live
/// instances (toasts replacing each other, editors) must be
/// distinguishable for the post-map fixup to find THE window it belongs
/// to, not whichever sibling happens to be newest.
pub fn unique_id(base: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{base}.{:x}.{}",
        nanos,
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// The primary monitor's rect in logical pixels, falling back to
/// GPUI's primary display. On X11 randr reports the real per-monitor
/// geometry where GPUI's primary_display() can span the whole virtual
/// screen; elsewhere GPUI's answer is already correct.
pub fn primary_monitor_rect(cx: &gpui::App) -> Option<(f32, f32, f32, f32)> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        if let Ok(monitors) = iris_lib::capture::x11::monitors() {
            if let Some(m) = monitors.first() {
                return Some((m.x as f32, m.y as f32, m.width as f32, m.height as f32));
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

// ---- post-map fixups -------------------------------------------------
// X11 WMs reposition and restyle a window at map time, so the X11
// implementation re-asserts geometry, decorations, and stacking after
// the map. Windows and macOS honor the bounds GPUI requests at window
// creation, so their fixups do nothing.

#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "linux")]
pub use x11::{
    always_on_top_after_map, find_xid_by_class_on, place_after_map, place_after_map_kind,
    span_after_map, suppress_decorations_on, unpark_span,
};
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// Hand a left-button drag that began at root `(root_x, root_y)` to the
/// window manager so it moves `window`. Call on the press (or the first
/// move while pressed).
pub fn begin_wm_move(window: &gpui::Window, class: String, root_x: i32, root_y: i32) {
    #[cfg(target_os = "linux")]
    {
        // Wayland: the compositor moves the window (xdg_toplevel.move);
        // X11 slides it from the root pointer, since _NET_WM_MOVERESIZE
        // loses the grab to GPUI's click grab.
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            window.start_window_move();
        } else {
            x11::begin_wm_move(class, root_x, root_y);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (class, root_x, root_y);
        #[cfg(windows)]
        windows::begin_move(window);
        #[cfg(target_os = "macos")]
        macos::begin_move(window);
    }
}

/// Keep `window` out of screen captures and recordings (the recording
/// chip over a recorded region). X11 has no such flag; the chip there
/// sits outside the recorded window instead.
#[cfg(not(target_os = "linux"))]
pub fn exclude_from_capture(window: &gpui::Window) {
    #[cfg(windows)]
    windows::exclude_from_capture(window);
    #[cfg(target_os = "macos")]
    macos::exclude_from_capture(window);
}

#[cfg(not(target_os = "linux"))]
mod other {
    pub fn place_after_map(_class: String, _x: f32, _y: f32) {}
    pub fn place_after_map_kind(_class: String, _x: f32, _y: f32, _notification: bool) {}
    pub fn always_on_top_after_map(_class: String) {}
    pub fn span_after_map(_class: String, _x: i32, _y: i32, _w: u32, _h: u32) {}
    pub fn unpark_span(_class: String, _x: i32, _y: i32, _w: u32, _h: u32) {}
}
#[cfg(not(target_os = "linux"))]
pub use other::*;
