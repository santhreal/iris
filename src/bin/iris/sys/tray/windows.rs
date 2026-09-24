//! The system-tray icon on Windows via `Shell_NotifyIconW`.
//!
//! A tray icon needs a window to receive its callback and menu
//! commands, so one thread owns a message-only window (`HWND_MESSAGE`
//! parent) and pumps it. The icon's callback message arrives on a
//! right-click and shows a popup menu; the chosen item comes back as
//! `WM_COMMAND` and is pushed onto the daemon channel. That mirrors
//! the Linux `ksni` tray: same menu, same `Command`s, same channel.

use futures::channel::mpsc::UnboundedSender;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, LoadCursorW,
    PostQuitMessage, RegisterClassExW, SetForegroundWindow, TrackPopupMenu, TranslateMessage,
    CW_USEDEFAULT, HICON, HMENU, HWND_MESSAGE, IDC_ARROW, MF_SEPARATOR, MF_STRING, MSG,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_APP, WM_COMMAND, WM_CONTEXTMENU,
    WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSEXW,
};

use crate::daemon::Command;

/// The tray icon's callback message id (distinct from the hotkey
/// thread's `WM_APP` offsets; they live on different queues anyway).
const WM_TRAYICON: u32 = WM_APP + 2;

// Menu item ids, reported in WM_COMMAND's low wParam word.
const IDM_CAPTURE: usize = 1;
const IDM_RECORD: usize = 2;
const IDM_LIBRARY: usize = 3;
const IDM_SETTINGS: usize = 4;
const IDM_QUIT: usize = 5;

/// The daemon channel, published once by `spawn` so the plain-fn
/// window procedure can reach it. `WNDPROC` cannot capture.
static TX: OnceLock<UnboundedSender<Command>> = OnceLock::new();

/// A 16x16 tray icon: a white aperture ring with a center dot on
/// transparency, matching the Linux tray glyph. `CreateIcon` takes the
/// AND mask (1bpp, 4-byte-aligned rows, 1 = transparent) and the XOR
/// color bits (32bpp BGRA, bottom-up) directly.
fn build_icon() -> HICON {
    const N: usize = 16;
    let stride = N.div_ceil(32) * 4;
    let mut and_mask = vec![0u8; stride * N];
    let mut xor = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let dx = x as i32 - 8;
            let dy = y as i32 - 8;
            let r2 = dx * dx + dy * dy;
            let opaque = (20..=40).contains(&r2) || r2 <= 5;
            if opaque {
                // XOR rows are bottom-up.
                let i = ((N - 1 - y) * N + x) * 4;
                xor[i..i + 4].copy_from_slice(&[244, 244, 246, 255]);
            } else {
                and_mask[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    unsafe {
        CreateIcon(
            GetModuleHandleW(core::ptr::null()),
            N as i32,
            N as i32,
            1,
            32,
            and_mask.as_ptr(),
            xor.as_ptr(),
        )
    }
}

/// Show the tray menu at the cursor and let `TrackPopupMenu` deliver
/// the pick back here as `WM_COMMAND`. `SetForegroundWindow` first so
/// the menu dismisses on a click elsewhere.
unsafe fn show_menu(hwnd: HWND) {
    let mut pt: POINT = core::mem::zeroed();
    if GetCursorPos(&mut pt) == 0 {
        return;
    }
    let menu: HMENU = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    for (id, label) in [
        (IDM_CAPTURE, "Capture"),
        (IDM_RECORD, "Record window"),
        (IDM_LIBRARY, "Library"),
        (IDM_SETTINGS, "Settings"),
        (0, ""),
        (IDM_QUIT, "Quit"),
    ] {
        if id == 0 {
            AppendMenuW(menu, MF_SEPARATOR, 0, core::ptr::null());
        } else {
            let wide: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
            AppendMenuW(menu, MF_STRING, id, wide.as_ptr());
        }
    }
    SetForegroundWindow(hwnd);
    TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        0,
        hwnd,
        core::ptr::null(),
    );
    DestroyMenu(menu);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            // Version-0 callback: lParam is the mouse message.
            let mouse = lparam as u32;
            if mouse == WM_RBUTTONUP || mouse == WM_CONTEXTMENU || mouse == WM_LBUTTONUP {
                show_menu(hwnd);
            }
            0
        }
        WM_COMMAND => {
            let cmd = match wparam & 0xFFFF {
                x if x == IDM_CAPTURE => Some(Command::Capture),
                x if x == IDM_RECORD => Some(Command::RecordToggle),
                x if x == IDM_LIBRARY => Some(Command::Library),
                x if x == IDM_SETTINGS => Some(Command::Settings),
                x if x == IDM_QUIT => Some(Command::Quit),
                _ => None,
            };
            if let (Some(cmd), Some(tx)) = (cmd, TX.get()) {
                let _ = tx.unbounded_send(cmd);
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Spawn the tray on its own thread: register the window class, create
/// the message-only window, add the icon, and pump messages for the
/// process lifetime. A failure degrades to a log line, never a crash.
pub(super) fn spawn(tx: UnboundedSender<Command>) {
    let _ = TX.set(tx);
    std::thread::spawn(|| unsafe {
        let hinst = GetModuleHandleW(core::ptr::null());
        let class: Vec<u16> = "iris-tray\0".encode_utf16().collect();
        let mut wc: WNDCLASSEXW = core::mem::zeroed();
        wc.cbSize = core::mem::size_of::<WNDCLASSEXW>() as u32;
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.hCursor = LoadCursorW(core::ptr::null_mut(), IDC_ARROW);
        wc.lpszClassName = class.as_ptr();
        if RegisterClassExW(&wc) == 0 {
            iris_lib::ilog!("iris: tray: RegisterClassExW failed");
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            HWND_MESSAGE,
            core::ptr::null_mut(),
            hinst,
            core::ptr::null(),
        );
        if hwnd.is_null() {
            iris_lib::ilog!("iris: tray: CreateWindowExW failed");
            return;
        }

        let icon = build_icon();
        let mut nid: NOTIFYICONDATAW = core::mem::zeroed();
        nid.cbSize = core::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;
        nid.hIcon = icon;
        for (i, c) in "iris".encode_utf16().enumerate() {
            nid.szTip[i] = c;
        }
        if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
            iris_lib::ilog!("iris: tray: Shell_NotifyIconW failed");
        }

        let mut msg: MSG = core::mem::zeroed();
        while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        Shell_NotifyIconW(NIM_DELETE, &nid);
        if !icon.is_null() {
            DestroyIcon(icon);
        }
        DestroyWindow(hwnd);
    });
}
