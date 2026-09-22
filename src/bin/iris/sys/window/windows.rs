//! Win32 window operations: capture exclusion through display affinity,
//! and moves through the caption-drag handoff. Releasing the capture
//! and posting `WM_NCLBUTTONDOWN(HTCAPTION)` enters the system's own
//! move loop, which follows the held button until release.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SetWindowDisplayAffinity, HTCAPTION, WDA_EXCLUDEFROMCAPTURE, WM_NCLBUTTONDOWN,
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
