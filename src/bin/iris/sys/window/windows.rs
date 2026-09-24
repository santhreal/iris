//! Win32 window operations: capture exclusion through display affinity,
//! moves through the caption-drag handoff, and the caption double-click
//! maximize toggle. Releasing the capture and posting
//! `WM_NCLBUTTONDOWN(HTCAPTION)` enters the system's own move loop,
//! which follows the held button until release.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsZoomed, PostMessageW, SetWindowDisplayAffinity, ShowWindowAsync, HTCAPTION, SW_MAXIMIZE,
    SW_RESTORE, WDA_EXCLUDEFROMCAPTURE, WM_NCLBUTTONDOWN,
};

fn hwnd(window: &gpui::Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as HWND),
        _ => None,
    }
}

/// Screen capture (BitBlt, gdigrab, Graphics Capture) skips the window.
pub fn exclude_from_capture(window: &gpui::Window) {
    if let Some(hwnd) = hwnd(window) {
        unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) };
    }
}

pub fn begin_move(window: &gpui::Window) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    unsafe {
        ReleaseCapture();
        // Posted, not sent: the move loop runs after this handler
        // returns, outside GPUI's event dispatch.
        PostMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
    }
}

/// Maximize the window, or restore it when it is maximized. GPUI's
/// `zoom_window` only maximizes on Windows.
pub fn toggle_maximize(window: &gpui::Window) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    unsafe {
        let cmd = if IsZoomed(hwnd) != 0 {
            SW_RESTORE
        } else {
            SW_MAXIMIZE
        };
        ShowWindowAsync(hwnd, cmd);
    }
}
