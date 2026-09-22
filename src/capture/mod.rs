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

/// Pick the capture backend for the current session. Wayland is checked
/// before X11 on Linux; Windows and macOS use the ffmpeg desktop grab.
pub fn backend() -> Result<Box<dyn CaptureBackend>, String> {
    // Wayland first: under Wayland the X11 backend would see only the
    // XWayland root, not the real session.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return wayland::WaylandBackend::new().map(|b| Box::new(b) as Box<dyn CaptureBackend>);
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some() {
        return x11::X11Backend::new().map(|b| Box::new(b) as Box<dyn CaptureBackend>);
    }
    #[cfg(windows)]
    {
        return Ok(Box::new(windows::WindowsBackend));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(Box::new(macos::MacosBackend));
    }
    #[allow(unreachable_code)]
    Err("no usable capture backend for this session".to_string())
}

/// Root-space pixels per logical window pixel. Root space (monitors,
/// window rects, frames) is physical; GPUI window bounds are logical.
/// X11 root space is already what GPUI's X11 backend uses.
pub fn root_scale() -> f32 {
    #[cfg(windows)]
    {
        return windows::root_scale();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::root_scale();
    }
    #[allow(unreachable_code)]
    1.0
}

/// Per-monitor rectangles in root space, primary first. Used to slice
/// the frozen frame per monitor and to place windows on the right
/// display. Empty when the platform cannot enumerate monitors.
pub fn monitors() -> Result<Vec<WinRect>, String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return x11::monitors();
    }
    #[cfg(windows)]
    {
        return windows::monitors();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::monitors();
    }
    #[allow(unreachable_code)]
    Err("monitor enumeration is not supported on this platform".to_string())
}

/// Monitors plus top-level windows for the overlay's hover-snap. The
/// second vec is empty on platforms without a window list.
pub fn layout() -> Result<(Vec<WinRect>, Vec<WinRect>), String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return x11::layout();
    }
    #[cfg(windows)]
    {
        return windows::layout();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::layout();
    }
    #[allow(unreachable_code)]
    Err("layout is not supported on this platform".to_string())
}

/// The focused window's rect in root space, decorations included.
/// X11 reads _NET_ACTIVE_WINDOW; Windows uses GetForegroundWindow;
/// macOS takes the frontmost on-screen window.
pub fn active_window_rect() -> Result<WinRect, String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return x11::active_window_rect();
    }
    #[cfg(windows)]
    {
        return windows::active_window_rect();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::active_window_rect();
    }
    #[allow(unreachable_code)]
    Err("focused-window capture is not supported on this platform".to_string())
}

/// Grab one rect of the screen into a fresh RGBA frame. The window
/// capture path reads only the target's pixels instead of grabbing the
/// whole screen and cropping.
pub fn grab_rect(rect: WinRect) -> Result<Frame, String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return x11::grab_root_rect(rect);
    }
    #[cfg(windows)]
    {
        return windows::grab_rect(rect);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::grab_rect(rect);
    }
    #[allow(unreachable_code)]
    Err("region grab is not supported on this platform".to_string())
}

/// Grab the whole screen into a BGRA buffer: the overlay's GPU-bound
/// frame consumes BGRA, so this skips the RGBA intermediate. Platforms
/// without a native BGRA path swizzle the RGBA frame.
pub fn grab_screen_bgra() -> Result<(u32, u32, Vec<u8>), String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return x11::grab_screen_bgra();
    }
    // Wayland, Windows, and macOS grab RGBA; swizzle to BGRA here so the
    // caller's GPU path is uniform. Banded across cores: a 4K frame is
    // 33MB of channel swaps.
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
