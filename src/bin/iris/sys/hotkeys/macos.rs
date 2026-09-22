//! Global hotkeys on macOS via the Carbon `RegisterEventHotKey` API.
//!
//! Carbon hotkeys are the one global-hotkey mechanism that needs no
//! Accessibility permission and no event tap: the system delivers a
//! `kEventHotKeyPressed` to the application event target, which the
//! running `NSApplication`/CFRunLoop dispatches. Registration and
//! dispatch are both plain C calls, so this file is self-contained FFI
//! with no crate beyond `libc` types.
//!
//! `spawn` registers the configured chords and installs one handler;
//! the handler maps the hotkey id back to a `Command` and pushes it on
//! the channel. `request_regrab` re-registers in place — Carbon calls
//! are thread-safe here, so no self-pipe is needed.

use futures::channel::mpsc::UnboundedSender;
use iris_lib::config::Config;
use parking_lot::Mutex;

use crate::daemon::Command;

// ---- Carbon FFI ------------------------------------------------------
// Event Manager + hotkey registration, all in Carbon.framework.

type OSStatus = i32;
type EventRef = *mut core::ffi::c_void;
type EventHandlerRef = *mut core::ffi::c_void;
type EventTargetRef = *mut core::ffi::c_void;
type EventHandlerCallRef = *mut core::ffi::c_void;
type EventHotKeyRef = *mut core::ffi::c_void;
type CFRunLoopRef = *mut core::ffi::c_void;

const K_EVENT_CLASS_APPLICATION: u32 = u32::from_be_bytes(*b"appl");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const K_EVENT_HOT_KEY_SIGNATURE: u32 = u32::from_be_bytes(*b"iris");
const NO_ERR: OSStatus = 0;

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
struct EventHotKeyId {
    signature: u32,
    id: u32,
}

type EventHandlerProcPtr =
    unsafe extern "C" fn(EventHandlerCallRef, EventRef, *mut core::ffi::c_void) -> OSStatus;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerProcPtr,
        num_types: usize,
        list: *const EventTypeSpec,
        user_data: *mut core::ffi::c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OSStatus;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        hot_key_id: EventHotKeyId,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;
    fn UnregisterEventHotKey(hot_key: EventHotKeyRef) -> OSStatus;
    fn GetEventParameter(
        event: EventRef,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: usize,
        actual_size: *mut usize,
        out_data: *mut core::ffi::c_void,
    ) -> OSStatus;
}

// ---- hotkey grammar --------------------------------------------------
// Carbon modifier flags (same numeric space as cmdKey etc.).
const CMD_KEY: u32 = 0x0100;
const SHIFT_KEY: u32 = 0x0200;
const OPTION_KEY: u32 = 0x0800;
const CONTROL_KEY: u32 = 0x1000;

/// macOS virtual key code for a key name in the hotkey grammar.
fn keycode_for(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    Some(match lower.as_str() {
        // macOS has no PrintScreen; F13 is the conventional capture key.
        "print" | "printscreen" | "prtscn" | "prtsc" | "prtscr" | "f13" => 105,
        "escape" | "esc" => 53,
        "enter" | "return" => 36,
        "space" | "spacebar" => 49,
        "tab" => 48,
        "backspace" => 51,
        "delete" | "del" => 117,
        "home" => 115,
        "end" => 119,
        "pageup" | "pgup" => 116,
        "pagedown" | "pgdn" => 121,
        "up" | "arrowup" => 126,
        "down" | "arrowdown" => 125,
        "left" | "arrowleft" => 123,
        "right" | "arrowright" => 124,
        s if s.len() == 1 => {
            let c = s.chars().next()?;
            match c {
                'a' => 0,
                's' => 1,
                'd' => 2,
                'f' => 3,
                'h' => 4,
                'g' => 5,
                'z' => 6,
                'x' => 7,
                'c' => 8,
                'v' => 9,
                'b' => 11,
                'q' => 12,
                'w' => 13,
                'e' => 14,
                'r' => 15,
                'y' => 16,
                't' => 17,
                '1' => 18,
                '2' => 19,
                '3' => 20,
                '4' => 21,
                '6' => 22,
                '5' => 23,
                '9' => 25,
                '7' => 26,
                '8' => 28,
                '0' => 29,
                'o' => 31,
                'u' => 32,
                'i' => 34,
                'p' => 35,
                'l' => 37,
                'j' => 38,
                'k' => 40,
                'n' => 45,
                'm' => 46,
                _ => return None,
            }
        }
        s if s.starts_with('f') => {
            let n: u32 = s[1..].parse().ok()?;
            // F1..F20 key codes.
            match n {
                1 => 122,
                2 => 120,
                3 => 99,
                4 => 118,
                5 => 96,
                6 => 97,
                7 => 98,
                8 => 100,
                9 => 101,
                10 => 109,
                11 => 103,
                12 => 111,
                13 => 105,
                14 => 107,
                15 => 113,
                16 => 106,
                17 => 64,
                18 => 79,
                19 => 80,
                20 => 90,
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// "Ctrl+Shift+R" -> (Carbon modifier flags, key code).
fn parse_hotkey(s: &str) -> Option<(u32, u32)> {
    let mut mods = 0u32;
    let mut key = None;
    for tok in s.split('+').map(|t| t.trim()).filter(|t| !t.is_empty()) {
        match tok.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= CONTROL_KEY,
            "shift" => mods |= SHIFT_KEY,
            "alt" | "option" => mods |= OPTION_KEY,
            "super" | "cmd" | "command" | "meta" | "win" | "windows" => mods |= CMD_KEY,
            name => key = keycode_for(name),
        }
    }
    key.map(|k| (mods, k))
}

// ---- registration state ----------------------------------------------

/// An opaque Carbon hotkey handle. It is never dereferenced, only
/// passed back to `UnregisterEventHotKey`, and `STATE`'s mutex
/// serializes that, so moving it between threads is sound.
struct HotKeyRef(EventHotKeyRef);
// SAFETY: see the type's doc comment.
unsafe impl Send for HotKeyRef {}

/// Live hotkey registrations plus the command each id maps to. The
/// handler reads the map; `register_all` rewrites it on regrab.
struct State {
    /// (hotkey id, command): the id is what the event reports.
    map: Vec<(u32, Command)>,
    /// Carbon refs to unregister on the next regrab.
    refs: Vec<HotKeyRef>,
    /// Next hotkey id; never reused so a stale event cannot misfire.
    next_id: u32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static TX: Mutex<Option<UnboundedSender<Command>>> = Mutex::new(None);

fn register_all() {
    let mut guard = STATE.lock();
    let Some(state) = guard.as_mut() else {
        return;
    };
    unsafe {
        for HotKeyRef(r) in state.refs.drain(..) {
            UnregisterEventHotKey(r);
        }
    }
    state.map.clear();
    let target = unsafe { GetApplicationEventTarget() };
    let cfg = Config::load();
    for (hotkey, cmd) in [
        (&cfg.capture_hotkey, Command::Capture),
        (&cfg.record_hotkey, Command::RecordToggle),
    ] {
        let Some((mods, code)) = parse_hotkey(hotkey) else {
            iris_lib::ilog!("iris: cannot parse hotkey {hotkey:?}");
            continue;
        };
        let id = state.next_id;
        state.next_id += 1;
        let hkid = EventHotKeyId {
            signature: K_EVENT_HOT_KEY_SIGNATURE,
            id,
        };
        let mut out: EventHotKeyRef = core::ptr::null_mut();
        let status = unsafe { RegisterEventHotKey(code, mods, hkid, target, 0, &mut out) };
        if status != NO_ERR {
            iris_lib::ilog!("iris: cannot register hotkey {hotkey:?} (status {status})");
        } else {
            state.refs.push(HotKeyRef(out));
            state.map.push((id, cmd));
        }
    }
}

unsafe extern "C" fn on_hotkey(
    _call: EventHandlerCallRef,
    event: EventRef,
    _data: *mut core::ffi::c_void,
) -> OSStatus {
    let mut hkid = EventHotKeyId {
        signature: 0,
        id: 0,
    };
    let status = GetEventParameter(
        event,
        K_EVENT_PARAM_DIRECT_OBJECT,
        u32::from_be_bytes(*b"hotk"),
        core::ptr::null_mut(),
        core::mem::size_of::<EventHotKeyId>(),
        core::ptr::null_mut(),
        &mut hkid as *mut _ as *mut core::ffi::c_void,
    );
    if status != NO_ERR {
        return status;
    }
    let cmd = STATE.lock().as_ref().and_then(|s| {
        s.map
            .iter()
            .find(|(id, _)| *id == hkid.id)
            .map(|(_, c)| c.clone())
    });
    if let (Some(cmd), Some(tx)) = (cmd, TX.lock().clone()) {
        let _ = tx.unbounded_send(cmd);
    }
    NO_ERR
}

/// Signalled by settings save: unregister everything, reload config,
/// re-register. Carbon registration is thread-safe, so this runs in
/// place rather than waking a dedicated thread.
pub(super) fn request_regrab() {
    register_all();
}

/// Register the configured hotkeys and install the press handler. The
/// events dispatch on the application's existing run loop, so this
/// needs no dedicated thread; a registration failure degrades to a log
/// line, never a crash.
pub(super) fn spawn(tx: UnboundedSender<Command>) {
    *TX.lock() = Some(tx);
    *STATE.lock() = Some(State {
        map: Vec::new(),
        refs: Vec::new(),
        next_id: 1,
    });
    let spec = EventTypeSpec {
        event_class: K_EVENT_CLASS_APPLICATION,
        event_kind: K_EVENT_HOT_KEY_PRESSED,
    };
    unsafe {
        let target = GetApplicationEventTarget();
        let mut handler: EventHandlerRef = core::ptr::null_mut();
        let status = InstallEventHandler(
            target,
            on_hotkey,
            1,
            &spec,
            core::ptr::null_mut(),
            &mut handler,
        );
        if status != NO_ERR {
            iris_lib::ilog!("iris: cannot install hotkey handler (status {status})");
            return;
        }
    }
    register_all();
}
