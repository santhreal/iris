//! The system-tray icon on Windows via `Shell_NotifyIconW`.
//!
//! A tray icon needs a window to receive its callback and menu
//! commands, so one thread owns a hidden top-level window and pumps
//! it. The icon's callback message arrives on a click and shows the
//! popup menu drawn from the shared rows; the chosen row id comes back
//! as `WM_COMMAND`. The window is top-level, not message-only, so the
//! `TaskbarCreated` broadcast reaches it and the icon comes back after
//! Explorer restarts.

use std::sync::atomic::{AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, LoadCursorW,
    LoadImageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetForegroundWindow,
    TrackPopupMenu, TranslateMessage, CW_USEDEFAULT, HICON, HMENU, IDC_ARROW, IMAGE_ICON,
    LR_DEFAULTCOLOR, MF_SEPARATOR, MF_STRING, MSG, SM_CXSMICON, SM_CYSMICON, TPM_BOTTOMALIGN,
    TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONUP,
    WM_RBUTTONUP, WNDCLASSEXW, WS_EX_TOOLWINDOW,
};

use super::Drawn;

/// The tray icon's callback message id (distinct from the hotkey
/// thread's `WM_APP` offsets; they live on different queues anyway).
const WM_TRAYICON: u32 = WM_APP + 2;

/// The `TaskbarCreated` message id, registered by `spawn`: Explorer
/// broadcasts it when it starts again, with every tray icon gone.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// The app icon at the notification area's size, from the executable's
/// resources: `packaging/windows/iris.rc` embeds `iris.ico` as icon 1,
/// the icon GPUI's windows load too. Null without the resource.
fn load_icon() -> HICON {
    unsafe {
        LoadImageW(
            GetModuleHandleW(core::ptr::null()),
            // MAKEINTRESOURCEW(1).
            core::ptr::without_provenance::<u16>(1),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )
    }
}

/// The notification area entry for `hwnd` with `icon`.
fn notify_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { core::mem::zeroed() };
    nid.cbSize = core::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAYICON;
    nid.hIcon = icon;
    for (i, c) in "iris".encode_utf16().enumerate() {
        nid.szTip[i] = c;
    }
    nid
}

/// The icon `spawn` loaded, for the re-add after Explorer restarts.
static ICON: std::sync::atomic::AtomicPtr<core::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

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
    for (id, row) in super::drawn() {
        match row {
            Drawn::Separator => {
                AppendMenuW(menu, MF_SEPARATOR, 0, core::ptr::null());
            }
            Drawn::Item(label) => {
                let wide: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                AppendMenuW(menu, MF_STRING, id, wide.as_ptr());
            }
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
            super::pick(wparam & 0xFFFF);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ if msg != 0 && msg == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            let nid = notify_data(hwnd, ICON.load(Ordering::Relaxed));
            if Shell_NotifyIconW(NIM_ADD, &nid) == 0 {
                iris_lib::ilog!("iris: tray: Shell_NotifyIconW failed after Explorer restarted");
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Spawn the tray on its own thread: register the window class, create
/// the hidden window, add the icon, and pump messages for the
/// process lifetime. A failure degrades to a log line, never a crash.
pub(super) fn spawn() {
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
        let taskbar: Vec<u16> = "TaskbarCreated\0".encode_utf16().collect();
        TASKBAR_CREATED.store(RegisterWindowMessageW(taskbar.as_ptr()), Ordering::Relaxed);
        // Never shown: no WS_VISIBLE, and a tool window has no taskbar
        // button or Alt+Tab entry.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            class.as_ptr(),
            0,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            hinst,
            core::ptr::null(),
        );
        if hwnd.is_null() {
            iris_lib::ilog!("iris: tray: CreateWindowExW failed");
            return;
        }

        let icon = load_icon();
        if icon.is_null() {
            iris_lib::ilog!("iris: tray: the executable has no icon resource 1");
        }
        ICON.store(icon, Ordering::Relaxed);
        let nid = notify_data(hwnd, icon);
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
