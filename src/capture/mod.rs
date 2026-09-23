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
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
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
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return wayland::WaylandBackend::new().map(|b| Box::new(b) as Box<dyn CaptureBackend>);
        }
        if std::env::var_os("DISPLAY").is_some() {
            return x11::X11Backend::new().map(|b| Box::new(b) as Box<dyn CaptureBackend>);
        }
        Err("no display: neither WAYLAND_DISPLAY nor DISPLAY is set".to_string())
    }
    #[cfg(not(target_os = "linux"))]
    Ok(Box::new(native::Backend))
}

/// Root-space pixels per logical window pixel. Root space (monitors,
/// window rects, frames) is physical; GPUI window bounds are logical.
/// X11 root space is already what GPUI's X11 backend uses.
pub fn root_scale() -> f32 {
    #[cfg(target_os = "linux")]
    return 1.0;
    #[cfg(not(target_os = "linux"))]
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
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
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
            for px in band.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        });
        Ok((frame.width, frame.height, bgra))
    }
}
