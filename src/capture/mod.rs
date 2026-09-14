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
