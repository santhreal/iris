#[cfg(target_os = "linux")]
pub mod x11;

#[cfg(target_os = "linux")]
pub mod wayland;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

/// One grabbed frame: RGBA8, row-major, tightly packed, origin at the
/// top-left of the virtual screen.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Root-space rectangle of a top-level window, used by overlay hover-snap.
#[derive(Clone, Copy, serde::Serialize)]
pub struct WinRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WinRect {
    /// The monitor among `monitors` (primary first) that holds this
    /// rect's center, or the primary when none does.
    pub fn host<'a>(&self, monitors: &'a [WinRect]) -> Option<&'a WinRect> {
        let cx = i64::from(self.x) + i64::from(self.width) / 2;
        let cy = i64::from(self.y) + i64::from(self.height) / 2;
        monitors
            .iter()
            .find(|m| {
                let (x, y) = (i64::from(m.x), i64::from(m.y));
                (x..x + i64::from(m.width)).contains(&cx)
                    && (y..y + i64::from(m.height)).contains(&cy)
            })
            .or(monitors.first())
    }
}

pub trait CaptureBackend {
    /// Grab the whole virtual screen as one frame.
    fn grab_screen(&self) -> Result<Frame, String>;
}

#[cfg(target_os = "macos")]
use self::macos as native;
/// The Windows or macOS capture module: one native backend per OS, so
/// each entry point below dispatches once instead of per platform.
#[cfg(windows)]
use self::windows as native;

/// Run an X11 query, or fail on a Wayland-only session: Wayland exposes
/// no monitor geometry, window list, or root grab to clients.
#[cfg(target_os = "linux")]
fn x11_only<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if !crate::session::wayland() {
        f()
    } else {
        Err(format!("{what} is not available on Wayland"))
    }
}

/// Pick the capture backend for the current session. Wayland is checked
/// before X11 on Linux: under Wayland the X11 backend would see only the
/// XWayland root, not the real session.
pub fn backend() -> Result<Box<dyn CaptureBackend>, String> {
    #[cfg(target_os = "linux")]
    {
        if crate::session::wayland() {
            return wayland::WaylandBackend::new().map(|b| Box::new(b) as Box<dyn CaptureBackend>);
        }
        if crate::session::x11() {
            return x11::X11Backend::new().map(|b| Box::new(b) as Box<dyn CaptureBackend>);
        }
        Err("no display: neither WAYLAND_DISPLAY nor DISPLAY is set".to_string())
    }
    #[cfg(not(target_os = "linux"))]
    Ok(Box::new(native::Backend))
}

/// Root-space pixels per logical window pixel: macOS points and Windows
/// DIPs at the system DPI. Root space (monitors, window rects, frames)
/// is physical. X11 defines no logical unit; the UI toolkit picks its
/// own scale there.
#[cfg(not(target_os = "linux"))]
pub fn root_scale() -> f32 {
    native::root_scale()
}

/// Per-monitor rectangles in root space, primary first. Used to slice
/// the frozen frame per monitor and to place windows on the right
/// display.
pub fn monitors() -> Result<Vec<WinRect>, String> {
    #[cfg(target_os = "linux")]
    return x11_only("monitor enumeration", x11::monitors);
    #[cfg(not(target_os = "linux"))]
    native::monitors()
}

/// Monitors plus top-level windows for the overlay's hover-snap.
pub fn layout() -> Result<(Vec<WinRect>, Vec<WinRect>), String> {
    #[cfg(target_os = "linux")]
    return x11_only("the window layout", x11::layout);
    #[cfg(not(target_os = "linux"))]
    native::layout()
}

/// The focused window's rect in root space, decorations included.
/// X11 reads _NET_ACTIVE_WINDOW; Windows uses GetForegroundWindow;
/// macOS takes the frontmost on-screen window.
pub fn active_window_rect() -> Result<WinRect, String> {
    #[cfg(target_os = "linux")]
    return x11_only("focused-window capture", x11::active_window_rect);
    #[cfg(not(target_os = "linux"))]
    native::active_window_rect()
}

/// Grab one rect of the screen into a fresh RGBA frame. The window
/// capture path reads only the target's pixels instead of grabbing the
/// whole screen and cropping.
pub fn grab_rect(rect: WinRect) -> Result<Frame, String> {
    #[cfg(target_os = "linux")]
    return x11_only("region grab", || x11::grab_root_rect(rect));
    #[cfg(not(target_os = "linux"))]
    native::grab_rect(rect)
}

/// Grab the whole screen into a BGRA buffer: the overlay's GPU-bound
/// frame consumes BGRA. X11 and Windows read BGRA natively; Wayland and
/// macOS grab RGBA and are swizzled here.
pub fn grab_screen_bgra() -> Result<(u32, u32, Vec<u8>), String> {
    #[cfg(target_os = "linux")]
    if !crate::session::wayland() {
        return x11::grab_screen_bgra();
    }
    #[cfg(windows)]
    return windows::grab_screen_bgra();
    #[cfg(not(windows))]
    {
        // Banded across cores: a 4K frame is 33MB of channel swaps.
        let frame = backend()?.grab_screen()?;
        let mut bgra = frame.rgba;
        let row = frame.width as usize * 4;
        crate::par::par_bands_mut(&mut bgra, row, |band, _| {
            crate::pixel::swap_rb_in_place(band)
        });
        Ok((frame.width, frame.height, bgra))
    }
}

// WHY: the class closed here is "a window placed for a rect lands on
// the wrong monitor": the recording chip picks the rect's monitor
// through `host`. Not covered: monitor enumeration itself, which the
// per-platform backends own.
#[cfg(test)]
mod tests {
    use super::WinRect;

    const fn rect(x: i32, y: i32, width: u32, height: u32) -> WinRect {
        WinRect {
            x,
            y,
            width,
            height,
        }
    }

    /// Primary 1000x1000 at the origin; a second monitor to its left at
    /// negative x, as Windows and macOS lay out a monitor placed there.
    const MONITORS: [WinRect; 2] = [rect(0, 0, 1000, 1000), rect(-800, 200, 800, 600)];

    fn host_x(r: WinRect, monitors: &[WinRect]) -> Option<i32> {
        r.host(monitors).map(|m| m.x)
    }

    #[test]
    fn host_is_the_monitor_holding_the_center() {
        assert_eq!(host_x(rect(-700, 300, 300, 200), &MONITORS), Some(-800));
        // Starts on the left monitor, centered on the primary: the
        // center decides, not the origin.
        assert_eq!(host_x(rect(-100, 300, 600, 200), &MONITORS), Some(0));
    }

    #[test]
    fn host_edges_are_half_open() {
        let side_by_side = [rect(0, 0, 1000, 1000), rect(1000, 0, 500, 500)];
        // Center at x=1000: past the primary's last column.
        assert_eq!(host_x(rect(999, 0, 2, 2), &side_by_side), Some(1000));
        assert_eq!(host_x(rect(998, 0, 2, 2), &side_by_side), Some(0));
    }

    #[test]
    fn host_falls_back_to_the_primary() {
        let nowhere = rect(-800, -500, 100, 100);
        assert_eq!(host_x(nowhere, &MONITORS), Some(0));
        assert_eq!(host_x(nowhere, &[]), None);
    }
}
