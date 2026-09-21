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
// X11 WMs reposition and restyle a window at map time, so the fixups
// below re-assert geometry, decorations, and stacking after the map.
// Other platforms honor the requested bounds at creation; the fixups
// are no-ops there.

#[cfg(target_os = "linux")]
mod imp {
    pub(crate) use crate::xwin::{
        always_on_top_after_map, begin_wm_move, place_after_map, place_after_map_kind,
        span_after_map, unpark_span,
    };
}

/// Reassert `(x, y)` after the WM maps the window named by `class`.
#[cfg(target_os = "linux")]
pub fn place_after_map(class: String, x: f32, y: f32) {
    imp::place_after_map(class, x, y);
}

/// `place_after_map` for a notification-style window.
#[cfg(target_os = "linux")]
pub fn place_after_map_kind(class: String, x: f32, y: f32, notification: bool) {
    imp::place_after_map_kind(class, x, y, notification);
}

/// Keep the window above its siblings after the map.
#[cfg(target_os = "linux")]
pub fn always_on_top_after_map(class: String) {
    imp::always_on_top_after_map(class);
}

/// Span the window across the given root rect after the map.
#[cfg(target_os = "linux")]
pub fn span_after_map(class: String, x: i32, y: i32, w: u32, h: u32) {
    imp::span_after_map(class, x, y, w, h);
}

/// Restore a parked window's span when it is reused.
#[cfg(target_os = "linux")]
pub fn unpark_span(class: String, x: i32, y: i32, w: u32, h: u32) {
    imp::unpark_span(class, x, y, w, h);
}

/// Begin a WM-driven move drag at the root coordinates.
#[cfg(target_os = "linux")]
pub fn begin_wm_move(class: String, root_x: i32, root_y: i32) {
    imp::begin_wm_move(class, root_x, root_y);
}

// Non-Linux platforms honor the bounds GPUI requests at window
// creation, so the post-map fixups are no-ops.
#[cfg(not(target_os = "linux"))]
pub fn place_after_map(_class: String, _x: f32, _y: f32) {}
#[cfg(not(target_os = "linux"))]
pub fn place_after_map_kind(_class: String, _x: f32, _y: f32, _notification: bool) {}
#[cfg(not(target_os = "linux"))]
pub fn always_on_top_after_map(_class: String) {}
#[cfg(not(target_os = "linux"))]
pub fn span_after_map(_class: String, _x: i32, _y: i32, _w: u32, _h: u32) {}
#[cfg(not(target_os = "linux"))]
pub fn unpark_span(_class: String, _x: i32, _y: i32, _w: u32, _h: u32) {}
#[cfg(not(target_os = "linux"))]
pub fn begin_wm_move(_class: String, _root_x: i32, _root_y: i32) {}
