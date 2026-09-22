// Windows capture via ffmpeg gdigrab. ffmpeg is already the encoder
// dependency, so this backend adds no crates. It records the primary
// desktop: gdigrab has no interactive window picker, so the target is
// the whole desktop and the stop path is the tray, the --stop-recording
// CLI flag, or the record hotkey.
#![cfg(windows)]

use crate::capture::{Frame, WinRect};
use std::process::Command;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowRect, IsWindowVisible,
};

pub fn capture_args() -> Vec<String> {
    vec!["-f".into(), "gdigrab".into(), "-i".into(), "desktop".into()]
}

pub fn capture_full_frame() -> Result<Frame, String> {
    let out = std::env::temp_dir().join("iris-grab.png");
    let _ = std::fs::remove_file(&out);
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error"])
        .args(capture_args())
        .args(["-frames:v", "1", "-y"])
        .arg(&out)
        .status()
        .map_err(|e| format!("ffmpeg gdigrab failed to start (is ffmpeg on PATH?): {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg gdigrab exited {status}"));
    }
    let png = std::fs::read(&out).map_err(|e| format!("read grab: {e}"))?;
    let _ = std::fs::remove_file(&out);
    let img = image::load_from_memory(&png)
        .map_err(|e| format!("decode gdigrab frame: {e}"))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Ok(Frame {
        width,
        height,
        rgba: img.into_raw(),
    })
}

pub struct WindowsBackend;

impl crate::capture::CaptureBackend for WindowsBackend {
    fn grab_screen(&self) -> Result<Frame, String> {
        capture_full_frame()
    }
}

// ---- geometry and region grabs --------------------------------------
// gdigrab captures the whole desktop; the monitor list, the focused
// window's rect, and per-region grabs come from Win32 so the overlay
// and window capture work the same as on X11.

fn rect_to_winrect(r: RECT) -> WinRect {
    WinRect {
        x: r.left,
        y: r.top,
        width: (r.right - r.left).max(0) as u32,
        height: (r.bottom - r.top).max(0) as u32,
    }
}

/// Per-monitor rectangles, primary first. EnumDisplayMonitors visits
/// every display; MONITORINFOF_PRIMARY marks the primary, which the
/// overlay expects at index 0.
pub fn monitors() -> Result<Vec<WinRect>, String> {
    unsafe extern "system" fn cb(mon: HMONITOR, _hdc: HDC, _rect: *mut RECT, data: LPARAM) -> BOOL {
        let out = &mut *(data as *mut Vec<(WinRect, bool)>);
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // MONITORINFOEXW extends MONITORINFO: the EX struct's first
        // field is the base, so the pointer cast is the documented call.
        if GetMonitorInfoW(mon, &mut info as *mut MONITORINFOEXW as *mut _) == TRUE {
            let primary = info.monitorInfo.dwFlags & 1 != 0; // MONITORINFOF_PRIMARY
            out.push((rect_to_winrect(info.monitorInfo.rcMonitor), primary));
        }
        TRUE
    }
    let mut found: Vec<(WinRect, bool)> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(cb),
            &mut found as *mut _ as LPARAM,
        );
    }
    if found.is_empty() {
        return Err("EnumDisplayMonitors found no monitors".to_string());
    }
    // Primary first, matching the X11 randr ordering the overlay relies on.
    found.sort_by_key(|(_, primary)| !*primary);
    Ok(found.into_iter().map(|(r, _)| r).collect())
}

/// Monitors plus the visible top-level windows for hover-snap.
pub fn layout() -> Result<(Vec<WinRect>, Vec<WinRect>), String> {
    let monitors = monitors()?;
    unsafe extern "system" fn cb(hwnd: HWND, data: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) == TRUE {
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut r) == TRUE {
                let out = &mut *(data as *mut Vec<WinRect>);
                out.push(rect_to_winrect(r));
            }
        }
        TRUE
    }
    let mut windows: Vec<WinRect> = Vec::new();
    unsafe {
        EnumWindows(Some(cb), &mut windows as *mut _ as LPARAM);
    }
    Ok((monitors, windows))
}

/// The foreground window's rect, decorations included.
pub fn active_window_rect() -> Result<WinRect, String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return Err("no foreground window".to_string());
        }
        let mut r: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut r) != TRUE {
            return Err("GetWindowRect failed".to_string());
        }
        Ok(rect_to_winrect(r))
    }
}

/// Grab one screen rect. gdigrab reads a region with -offset_x/-offset_y
/// and -video_size, so a window capture does not decode the whole
/// desktop and crop.
pub fn grab_rect(rect: WinRect) -> Result<Frame, String> {
    let out = std::env::temp_dir().join("iris-grab-rect.png");
    let _ = std::fs::remove_file(&out);
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error"])
        .args(capture_args())
        .args([
            "-offset_x".into(),
            rect.x.to_string(),
            "-offset_y".into(),
            rect.y.to_string(),
            "-video_size".into(),
            format!("{}x{}", rect.width, rect.height),
        ])
        .args(["-frames:v", "1", "-y"])
        .arg(&out)
        .status()
        .map_err(|e| format!("ffmpeg gdigrab failed to start: {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg gdigrab exited {status}"));
    }
    let png = std::fs::read(&out).map_err(|e| format!("read grab: {e}"))?;
    let _ = std::fs::remove_file(&out);
    let img = image::load_from_memory(&png)
        .map_err(|e| format!("decode gdigrab frame: {e}"))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Ok(Frame {
        width,
        height,
        rgba: img.into_raw(),
    })
}
