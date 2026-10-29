// Windows capture through GDI: BitBlt from the screen DC into a
// top-down 32-bit DIB. The process is per-monitor DPI-aware, so every
// rect is in physical desktop pixels.
#![cfg(windows)]

use crate::capture::{Frame, WinRect};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetCursorPos, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible,
};

/// Root pixels per GPUI logical pixel: the system DPI over 96. GPUI
/// runs DPI-aware, so its window coordinates are logical.
pub fn root_scale() -> f32 {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForSystem() };
    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

/// The whole virtual screen (every monitor).
pub fn capture_full_frame() -> Result<Frame, String> {
    let (x, y, w, h) = virtual_screen();
    if w <= 0 || h <= 0 {
        return Err("no virtual screen (is a desktop session attached?)".to_string());
    }
    grab_rect(WinRect {
        x,
        y,
        width: w as u32,
        height: h as u32,
    })
}

pub struct Backend;

impl crate::capture::CaptureBackend for Backend {
    fn grab_screen(&self) -> Result<Frame, String> {
        capture_full_frame()
    }
}

// ---- geometry ------------------------------------------------------
// Rects are desktop coordinates, which go negative for a monitor left
// of or above the primary. The full frame starts at the virtual
// screen's top-left, the union of the monitor rects; the overlay maps
// rects into the frame relative to that union.

/// Desktop position and size of the virtual screen (every monitor).
fn virtual_screen() -> (i32, i32, i32, i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

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

/// `monitor` less the taskbar and the app bars docked on its edges: the
/// work area of the monitor nearest `monitor`, from `GetMonitorInfoW`.
/// `monitor` itself where Windows reports none.
pub fn work_area(monitor: WinRect) -> WinRect {
    use windows_sys::Win32::Graphics::Gdi::{
        MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    let rect = RECT {
        left: monitor.x,
        top: monitor.y,
        right: monitor.x.saturating_add_unsigned(monitor.width),
        bottom: monitor.y.saturating_add_unsigned(monitor.height),
    };
    // SAFETY: `rect` and `info` outlive both calls, and `info.cbSize`
    // states the struct GetMonitorInfoW fills.
    unsafe {
        let handle = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if handle.is_null() || GetMonitorInfoW(handle, &mut info) != TRUE {
            return monitor;
        }
        monitor
            .intersect(&rect_to_winrect(info.rcWork))
            .unwrap_or(monitor)
    }
}

/// The rect a window draws on screen, or None for a window that shows
/// nothing: one cloaked by DWM (on another virtual desktop, or a
/// suspended store app) or one with an empty frame. DWM's extended
/// frame bounds leave out the invisible resize borders GetWindowRect
/// includes, 7 px on each side of a Windows 10 or 11 window.
unsafe fn shown_rect(hwnd: HWND) -> Option<WinRect> {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    let mut cloaked: u32 = 0;
    let read = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED as u32,
        &mut cloaked as *mut u32 as *mut core::ffi::c_void,
        std::mem::size_of::<u32>() as u32,
    );
    if read == 0 && cloaked != 0 {
        return None;
    }
    let mut r: RECT = std::mem::zeroed();
    let framed = DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS as u32,
        &mut r as *mut RECT as *mut core::ffi::c_void,
        std::mem::size_of::<RECT>() as u32,
    ) == 0;
    if !framed && GetWindowRect(hwnd, &mut r) != TRUE {
        return None;
    }
    let rect = rect_to_winrect(r);
    (rect.width > 0 && rect.height > 0).then_some(rect)
}

/// Monitors plus the shown top-level windows of other processes for
/// hover-snap, bottom to top. EnumWindows visits the top of the Z order
/// first, so the list is reversed. Minimized and cloaked windows show
/// nothing and are left out.
pub fn layout() -> Result<(Vec<WinRect>, Vec<WinRect>), String> {
    let monitors = monitors()?;
    unsafe extern "system" fn cb(hwnd: HWND, data: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) != TRUE || IsIconic(hwnd) == TRUE {
            return TRUE;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == windows_sys::Win32::System::Threading::GetCurrentProcessId() {
            return TRUE;
        }
        if let Some(rect) = shown_rect(hwnd) {
            (*(data as *mut Vec<WinRect>)).push(rect);
        }
        TRUE
    }
    let mut windows: Vec<WinRect> = Vec::new();
    unsafe {
        EnumWindows(Some(cb), &mut windows as *mut _ as LPARAM);
    }
    windows.reverse();
    Ok((monitors, windows))
}

/// The rect the foreground window draws, decorations included.
pub fn active_window_rect() -> Result<WinRect, String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return Err("no foreground window".to_string());
        }
        shown_rect(hwnd).ok_or_else(|| "the foreground window shows nothing".to_string())
    }
}

/// The pointer in physical desktop pixels.
pub fn pointer() -> Result<(i32, i32), String> {
    let mut p = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut p) } == 0 {
        return Err("GetCursorPos failed".to_string());
    }
    Ok((p.x, p.y))
}

/// The whole virtual screen as BGRA, the layout GDI produces: the
/// overlay's frame consumes BGRA, so no swizzle runs.
pub fn grab_screen_bgra() -> Result<(u32, u32, Vec<u8>), String> {
    let (x, y, w, h) = virtual_screen();
    if w <= 0 || h <= 0 {
        return Err("no virtual screen (is a desktop session attached?)".to_string());
    }
    let (w, h) = (w as u32, h as u32);
    let bgra = grab(
        WinRect {
            x,
            y,
            width: w,
            height: h,
        },
        crate::pixel::opaque,
    )?;
    Ok((w, h, bgra))
}

/// Pixels of `rect` (desktop coordinates) as RGBA.
pub fn grab_rect(rect: WinRect) -> Result<Frame, String> {
    let rgba = grab(rect, crate::pixel::swap_rb_opaque)?;
    Ok(Frame {
        width: rect.width,
        height: rect.height,
        rgba,
    })
}

/// Pixels of `rect` (desktop coordinates), each GDI BGRX pixel through
/// `px` in the one pass that copies it out of the DIB. `BI_RGB` leaves
/// the fourth byte undefined (often 0), so `px` sets alpha. CAPTUREBLT
/// includes layered windows (tooltips, translucent UI).
fn grab(rect: WinRect, px: impl Fn([u8; 4]) -> [u8; 4]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
    };
    let (w, h) = (rect.width as i32, rect.height as i32);
    if w <= 0 || h <= 0 {
        return Err(format!("empty capture rect {}x{}", rect.width, rect.height));
    }
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err("GetDC(screen) failed (is a desktop session attached?)".to_string());
        }
        let mem = CreateCompatibleDC(screen);
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // negative: top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(
            mem,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        let result = if mem.is_null() || dib.is_null() || bits.is_null() {
            Err("CreateDIBSection failed".to_string())
        } else {
            let old = SelectObject(mem, dib);
            let ok = BitBlt(
                mem,
                0,
                0,
                w,
                h,
                screen,
                rect.x,
                rect.y,
                SRCCOPY | CAPTUREBLT,
            ) != 0;
            SelectObject(mem, old);
            if ok {
                let len = w as usize * h as usize * 4;
                let pixels = std::slice::from_raw_parts(bits as *const u8, len);
                Ok(crate::pixel::map_to_vec(pixels, px))
            } else {
                Err("BitBlt from the screen failed".to_string())
            }
        };
        if !dib.is_null() {
            DeleteObject(dib);
        }
        if !mem.is_null() {
            DeleteDC(mem);
        }
        ReleaseDC(std::ptr::null_mut(), screen);
        result
    }
}

#[cfg(test)]
mod tests {
    // WHY: the class closed here is "the screenshot is not the screen":
    // a frame whose size differs from the monitor union (wrong origin or
    // DPI space), or an all-black frame (a capture from a session with no
    // desktop). Needs an interactive desktop, so ignored by default:
    // `cargo test -- --ignored gdi_` from the logged-in session.
    use super::*;

    #[test]
    #[ignore = "needs an interactive desktop session"]
    fn gdi_full_frame_spans_the_monitor_union_with_real_pixels() {
        let mons = monitors().unwrap();
        let x0 = mons.iter().map(|m| m.x).min().unwrap();
        let y0 = mons.iter().map(|m| m.y).min().unwrap();
        let x1 = mons.iter().map(|m| m.x + m.width as i32).max().unwrap();
        let y1 = mons.iter().map(|m| m.y + m.height as i32).max().unwrap();
        let f = capture_full_frame().unwrap();
        assert_eq!((f.width as i32, f.height as i32), (x1 - x0, y1 - y0));
        assert_eq!(f.rgba.len(), (f.width * f.height * 4) as usize);
        let pixels = f.rgba.as_chunks::<4>().0;
        assert!(pixels.iter().all(|p| p[3] == 255));
        assert!(
            pixels.iter().any(|p| p[0] | p[1] | p[2] != 0),
            "all-black frame"
        );

        // A rect grab reads the same pixels as the full frame there.
        let r = WinRect {
            x: mons[0].x + 10,
            y: mons[0].y + 10,
            width: 64,
            height: 32,
        };
        let g = grab_rect(r).unwrap();
        let (ox, oy) = ((r.x - x0) as u32, (r.y - y0) as u32);
        let row = |fr: &Frame, x: u32, y: u32| {
            let i = ((y * fr.width + x) * 4) as usize;
            fr.rgba[i..i + 64 * 4].to_vec()
        };
        assert_eq!(row(&g, 0, 5), row(&f, ox, oy + 5));

        // The overlay's BGRA grab is the same frame, channels swapped.
        let (bw, bh, bgra) = grab_screen_bgra().unwrap();
        assert_eq!((bw, bh), (f.width, f.height));
        let i = ((oy * bw + ox) * 4) as usize;
        assert_eq!(
            [bgra[i + 2], bgra[i + 1], bgra[i], bgra[i + 3]],
            [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], 255]
        );
    }
}
