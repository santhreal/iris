//! Windows: Explorer folder windows, window rects from `EnumWindows`,
//! and pointer input through `SendInput`.

use super::Rect;
use std::path::Path;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, MOUSE_EVENT_FLAGS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetSystemMetrics, GetWindowRect, GetWindowTextW, IsWindowVisible,
    PostMessageW, SetWindowPos, HWND_TOP, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_SHOWWINDOW, WM_CLOSE,
};

/// Window rects and input coordinates in physical pixels on any
/// display scale.
pub fn init() {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Every visible top-level window: handle, class, title.
fn windows() -> Vec<(HWND, String, String)> {
    unsafe extern "system" fn each(hwnd: HWND, out: LPARAM) -> BOOL {
        let out = &mut *(out as *mut Vec<(HWND, String, String)>);
        if IsWindowVisible(hwnd) != 0 {
            let class = text(|b, n| GetClassNameW(hwnd, b, n));
            let title = text(|b, n| GetWindowTextW(hwnd, b, n));
            out.push((hwnd, class, title));
        }
        1
    }
    let mut out: Vec<(HWND, String, String)> = Vec::new();
    unsafe { EnumWindows(Some(each), &mut out as *mut _ as LPARAM) };
    out
}

fn text(read: impl FnOnce(*mut u16, i32) -> i32) -> String {
    let mut buf = [0u16; 512];
    let n = read(buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn rect(hwnd: HWND) -> Rect {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd, &mut r) };
    Rect {
        x: f64::from(r.left),
        y: f64::from(r.top),
        w: f64::from(r.right - r.left),
        h: f64::from(r.bottom - r.top),
    }
}

pub fn describe() -> String {
    windows()
        .into_iter()
        .map(|(h, class, title)| format!("  {class} {title:?} {:?}\n", rect(h)))
        .collect()
}

pub fn toast() -> Option<Rect> {
    windows()
        .into_iter()
        .find(|(_, _, title)| title == super::TOAST)
        .map(|(h, _, _)| rect(h))
}

/// An Explorer window on a folder; closed on drop.
pub struct Folder {
    hwnd: HWND,
    pub rect: Rect,
}

/// Open `dir` in Explorer at the top left of the primary display.
pub fn open_folder(dir: &Path) -> Folder {
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    // explorer.exe hands the folder to the running shell and exits
    // with status 1 whether or not a window opens.
    let _ = std::process::Command::new("explorer.exe").arg(dir).status();
    let deadline = Instant::now() + Duration::from_secs(15);
    let hwnd = loop {
        let found = windows().into_iter().find(|(_, class, title)| {
            class == "CabinetWClass"
                && (*title == name
                    || title.starts_with(&format!("{name} - "))
                    || title.ends_with(&format!("\\{name}")))
        });
        if let Some((hwnd, _, _)) = found {
            break hwnd;
        }
        assert!(
            Instant::now() < deadline,
            "no Explorer window on {}; windows on screen:\n{}",
            dir.display(),
            describe()
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    unsafe { SetWindowPos(hwnd, HWND_TOP, 40, 40, 640, 480, SWP_SHOWWINDOW) };
    // Explorer lays out its panes after the resize.
    std::thread::sleep(Duration::from_millis(800));
    Folder {
        hwnd,
        rect: rect(hwnd),
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        unsafe { PostMessageW(self.hwnd, WM_CLOSE, 0, 0) };
    }
}

/// One mouse event at `(x, y)` on the virtual desktop.
fn send(flags: MOUSE_EVENT_FLAGS, (x, y): (f64, f64)) {
    let metric = |i| unsafe { GetSystemMetrics(i) };
    let (vx, vy) = (metric(SM_XVIRTUALSCREEN), metric(SM_YVIRTUALSCREEN));
    let (vw, vh) = (metric(SM_CXVIRTUALSCREEN), metric(SM_CYVIRTUALSCREEN));
    // Absolute input spans 0..=65535 across the virtual desktop.
    let norm = |v: f64, origin: i32, len: i32| {
        ((v - f64::from(origin)) * 65535.0 / f64::from((len - 1).max(1))).round() as i32
    };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: norm(x, vx, vw),
                dy: norm(y, vy, vh),
                mouseData: 0,
                dwFlags: flags | MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        eprintln!("SendInput: {}", std::io::Error::last_os_error());
    }
}

pub fn hover(at: (f64, f64)) {
    send(0, at);
}

pub fn press(at: (f64, f64)) {
    send(MOUSEEVENTF_LEFTDOWN, at);
}

pub fn drag_to(at: (f64, f64)) {
    send(0, at);
}

pub fn release(at: (f64, f64)) {
    send(MOUSEEVENTF_LEFTUP, at);
}
