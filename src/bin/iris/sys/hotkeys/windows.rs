//! Global hotkeys on Windows via `RegisterHotKey`.
//!
//! One dedicated thread owns the registrations and a message queue.
//! `RegisterHotKey` with a null HWND posts `WM_HOTKEY` to the calling
//! thread's queue, so the loop is a plain `GetMessageW` pump: a chord
//! press arrives as a message whose `wParam` is the hotkey id, and a
//! settings save arrives as a `WM_APP` posted by `request_regrab`.
//! That mirrors the Linux self-pipe: the thread sleeps until a key or
//! a regrab lands, never polls.

use futures::channel::mpsc::UnboundedSender;
use iris_lib::config::Config;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN, VK_0, VK_A, VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME,
    VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SNAPSHOT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW, TranslateMessage, MSG, WM_APP,
    WM_HOTKEY,
};

use crate::daemon::Command;

/// `WM_APP` offset that means "reload config and re-register".
const WM_REGRAB: u32 = WM_APP + 1;

/// The hotkey thread's id, published once its message queue exists so
/// `request_regrab` can post to it. 0 = not running.
static HOTKEY_TID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Virtual-key code for a key name in the hotkey grammar. The VK_*
/// constants are u16; `RegisterHotKey` wants a u32.
fn vk_for(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    Some(match lower.as_str() {
        "print" | "printscreen" | "prtscn" | "prtsc" | "prtscr" => VK_SNAPSHOT as u32,
        "escape" | "esc" => VK_ESCAPE as u32,
        "enter" | "return" => VK_RETURN as u32,
        "space" | "spacebar" => VK_SPACE as u32,
        "tab" => VK_TAB as u32,
        "backspace" => VK_BACK as u32,
        "delete" | "del" => VK_DELETE as u32,
        "home" => VK_HOME as u32,
        "end" => VK_END as u32,
        "pageup" | "pgup" => VK_PRIOR as u32,
        "pagedown" | "pgdn" => VK_NEXT as u32,
        "up" | "arrowup" => VK_UP as u32,
        "down" | "arrowdown" => VK_DOWN as u32,
        "left" | "arrowleft" => VK_LEFT as u32,
        "right" | "arrowright" => VK_RIGHT as u32,
        s if s.len() == 1 => {
            let c = s.chars().next()?;
            match c {
                'a'..='z' => VK_A as u32 + (c as u32 - 'a' as u32),
                '0'..='9' => VK_0 as u32 + (c as u32 - '0' as u32),
                _ => return None,
            }
        }
        s if s.starts_with('f') => {
            let n: u32 = s[1..].parse().ok()?;
            if !(1..=24).contains(&n) {
                return None;
            }
            VK_F1 as u32 + (n - 1)
        }
        _ => return None,
    })
}

/// "Ctrl+Shift+R" -> (modifier flags, virtual-key code).
fn parse_hotkey(s: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods: HOT_KEY_MODIFIERS = 0;
    let mut key = None;
    for tok in s.split('+').map(|t| t.trim()).filter(|t| !t.is_empty()) {
        match tok.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "shift" => mods |= MOD_SHIFT,
            "alt" | "option" => mods |= MOD_ALT,
            "super" | "cmd" | "command" | "meta" | "win" | "windows" => mods |= MOD_WIN,
            name => key = vk_for(name),
        }
    }
    key.map(|k| (mods, k))
}

/// Signalled by settings save: unregister everything, reload config,
/// re-register. The signal is a `WM_APP` on the hotkey thread's queue,
/// so its `GetMessageW` wakes on it instead of polling a channel.
pub(super) fn request_regrab() {
    let tid = HOTKEY_TID.load(std::sync::atomic::Ordering::Acquire);
    if tid != 0 {
        unsafe {
            PostThreadMessageW(tid, WM_REGRAB, 0, 0);
        }
    }
}

/// Register the configured hotkeys and forward presses. Runs on its own
/// thread with a message queue; a registration failure degrades to a
/// log line, never a crash.
pub(super) fn spawn(tx: UnboundedSender<Command>) {
    std::thread::spawn(move || {
        // Force the thread's message queue into existence before
        // publishing the tid: PostThreadMessageW fails on a thread
        // that has never touched the queue.
        let mut msg: MSG = unsafe { core::mem::zeroed() };
        unsafe {
            PeekMessageW(
                &mut msg,
                core::ptr::null_mut(),
                0,
                0,
                0, /* PM_NOREPEAT */
            );
        }
        HOTKEY_TID.store(
            unsafe { GetCurrentThreadId() },
            std::sync::atomic::Ordering::Release,
        );

        // (hotkey id, command): the id is what WM_HOTKEY reports in
        // wParam, so it doubles as the dispatch key.
        let mut registered: Vec<(i32, Command)> = Vec::new();
        let mut next_id: i32 = 1;

        let register_all = |registered: &mut Vec<(i32, Command)>, next_id: &mut i32| {
            for (id, _) in registered.drain(..) {
                unsafe {
                    UnregisterHotKey(core::ptr::null_mut(), id);
                }
            }
            let cfg = Config::load();
            for (hotkey, cmd) in [
                (&cfg.capture_hotkey, Command::Capture),
                (&cfg.record_hotkey, Command::RecordToggle),
            ] {
                let Some((mods, vk)) = parse_hotkey(hotkey) else {
                    iris_lib::ilog!("iris: cannot parse hotkey {hotkey:?}");
                    continue;
                };
                let id = *next_id;
                *next_id += 1;
                // MOD_NOREPEAT: holding the chord fires once, not per
                // key-repeat, so a held hotkey cannot queue captures.
                let ok =
                    unsafe { RegisterHotKey(core::ptr::null_mut(), id, mods | MOD_NOREPEAT, vk) };
                if ok == 0 {
                    iris_lib::ilog!("iris: cannot register hotkey {hotkey:?}");
                } else {
                    registered.push((id, cmd));
                }
            }
        };
        register_all(&mut registered, &mut next_id);

        loop {
            let r = unsafe { GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) };
            if r <= 0 {
                // 0 = WM_QUIT, -1 = error: either way the pump ends.
                break;
            }
            match msg.message {
                WM_HOTKEY => {
                    let id = msg.wParam as i32;
                    if let Some((_, cmd)) = registered.iter().find(|(rid, _)| *rid == id) {
                        if tx.unbounded_send(cmd.clone()).is_err() {
                            break;
                        }
                    }
                }
                WM_REGRAB => register_all(&mut registered, &mut next_id),
                _ => unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                },
            }
        }
        HOTKEY_TID.store(0, std::sync::atomic::Ordering::Release);
    });
}
