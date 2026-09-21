//! The system-tray icon on macOS via `NSStatusItem`.
//!
//! The status item lives in the menu bar and owns an `NSMenu` of the
//! same commands as the other platforms. Menu items need an ObjC
//! target that responds to a selector, so `spawn` defines a tiny
//! `IrisTrayTarget` class at runtime whose `onItem:` reads the item's
//! tag and pushes the matching `Command` onto the daemon channel.
//! Everything is `objc_msgSend` FFI — no crate beyond the runtime.

use futures::channel::mpsc::UnboundedSender;
use std::sync::OnceLock;

use crate::daemon::Command;

// ---- Objective-C runtime FFI -----------------------------------------

type Id = *mut core::ffi::c_void;
type Sel = *mut core::ffi::c_void;
type Class = *mut core::ffi::c_void;
type Imp = *const core::ffi::c_void;

#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const core::ffi::c_char) -> Class;
    fn objc_allocateClassPair(
        superclass: Class,
        name: *const core::ffi::c_char,
        extra_bytes: usize,
    ) -> Class;
    fn objc_registerClassPair(cls: Class);
    fn class_addMethod(cls: Class, name: Sel, imp: Imp, types: *const core::ffi::c_char) -> bool;
    fn sel_registerName(name: *const core::ffi::c_char) -> Sel;
    // objc_msgSend is variadic; declare the arities used.
    fn objc_msgSend(receiver: Id, sel: Sel, ...) -> Id;
}

fn cls(name: &str) -> Class {
    unsafe { objc_getClass(name.as_ptr() as *const _) }
}
fn sel(name: &str) -> Sel {
    unsafe { sel_registerName(name.as_ptr() as *const _) }
}
unsafe fn msg0(obj: Id, s: Sel) -> Id {
    objc_msgSend(obj, s)
}
unsafe fn msg1(obj: Id, s: Sel, a: usize) -> Id {
    objc_msgSend(obj, s, a)
}
unsafe fn msg_id_id(obj: Id, s: Sel, a: Id) -> Id {
    objc_msgSend(obj, s, a)
}
unsafe fn msg_id_id_id(obj: Id, s: Sel, a: Id, b: Id) -> Id {
    objc_msgSend(obj, s, a, b)
}
unsafe fn msg_id_id_sel(obj: Id, s: Sel, a: Id, b: Sel) -> Id {
    objc_msgSend(obj, s, a, b)
}
unsafe fn msg_id_id_sel_id(obj: Id, s: Sel, a: Id, b: Sel, c: Id) -> Id {
    objc_msgSend(obj, s, a, b, c)
}
unsafe fn msg_f64(obj: Id, s: Sel, a: f64) -> Id {
    objc_msgSend(obj, s, a)
}
unsafe fn nsstr(s: &str) -> Id {
    let cls = cls("NSString");
    let alloc = msg0(cls, sel("alloc"));
    msg_id_id_id(
        alloc,
        sel("initWithBytes:length:encoding:"),
        s.as_ptr() as Id,
        s.len() as Id,
        4 as Id, // NSUTF8StringEncoding
    )
}

// ---- menu -> command --------------------------------------------------

const TAG_CAPTURE: isize = 1;
const TAG_RECORD: isize = 2;
const TAG_LIBRARY: isize = 3;
const TAG_SETTINGS: isize = 4;
const TAG_QUIT: isize = 5;

static TX: OnceLock<UnboundedSender<Command>> = OnceLock::new();

/// `onItem:` — the menu action. `sender` is the NSMenuItem; its tag
/// selects the command. Signature `v@:@` (void, self, _cmd, sender).
unsafe extern "C" fn on_item(_this: Id, _cmd: Sel, sender: Id) {
    let tag = msg0(sender, sel("tag")) as isize;
    let cmd = match tag {
        TAG_CAPTURE => Some(Command::Capture),
        TAG_RECORD => Some(Command::RecordToggle),
        TAG_LIBRARY => Some(Command::Library),
        TAG_SETTINGS => Some(Command::Settings),
        TAG_QUIT => Some(Command::Quit),
        _ => None,
    };
    if let (Some(cmd), Some(tx)) = (cmd, TX.get()) {
        let _ = tx.unbounded_send(cmd);
    }
}

/// Define `IrisTrayTarget` (an NSObject subclass with `onItem:`) and
/// return an instance. Done once; the class persists for the process.
unsafe fn tray_target() -> Id {
    static ONCE: std::sync::Once = std::sync::Once::new();
    static mut TARGET: Id = core::ptr::null_mut();
    ONCE.call_once(|| {
        let nsobject = cls("NSObject");
        let c = objc_allocateClassPair(nsobject, b"IrisTrayTarget\0".as_ptr() as *const _, 0);
        class_addMethod(
            c,
            sel("onItem:"),
            on_item as Imp,
            b"v@:@\0".as_ptr() as *const _,
        );
        objc_registerClassPair(c);
        let alloc = msg0(c as Id, sel("alloc"));
        TARGET = msg0(alloc, sel("init"));
    });
    TARGET
}

/// Spawn the status item. The menu bar item and its menu are created on
/// the calling thread; AppKit delivers `onItem:` on the main run loop,
/// which GPUI already runs, so no dedicated thread is needed. A
/// failure degrades to a log line, never a crash.
pub(super) fn spawn(tx: UnboundedSender<Command>) {
    let _ = TX.set(tx);
    unsafe {
        let bar = msg0(cls("NSStatusBar"), sel("systemStatusBar"));
        if bar.is_null() {
            iris_lib::ilog!("iris: tray: NSStatusBar unavailable");
            return;
        }
        // NSVariableStatusItemLength = -1.
        let item = msg_f64(bar, sel("statusItemWithLength:"), -1.0);
        if item.is_null() {
            iris_lib::ilog!("iris: tray: statusItemWithLength failed");
            return;
        }
        let button = msg0(item, sel("button"));
        if !button.is_null() {
            let title = nsstr("iris");
            msg_id_id(button, sel("setTitle:"), title);
        }

        let target = tray_target();
        let menu = msg0(msg0(cls("NSMenu"), sel("alloc")), sel("init"));
        for (tag, label) in [
            (TAG_CAPTURE, "Capture"),
            (TAG_RECORD, "Record window"),
            (TAG_LIBRARY, "Library"),
            (TAG_SETTINGS, "Settings"),
        ] {
            let title = nsstr(label);
            let mi = msg_id_id_sel_id(
                msg0(cls("NSMenuItem"), sel("alloc")),
                sel("initWithTitle:action:keyEquivalent:"),
                title,
                sel("onItem:"),
                nsstr(""),
            );
            msg1(mi, sel("setTag:"), tag as usize);
            msg_id_id(mi, sel("setTarget:"), target);
            msg_id_id(menu, sel("addItem:"), mi);
        }
        // Separator, then Quit.
        let sep = msg0(cls("NSMenuItem"), sel("separatorItem"));
        msg_id_id(menu, sel("addItem:"), sep);
        let quit = msg_id_id_sel_id(
            msg0(cls("NSMenuItem"), sel("alloc")),
            sel("initWithTitle:action:keyEquivalent:"),
            nsstr("Quit"),
            sel("onItem:"),
            nsstr(""),
        );
        msg1(quit, sel("setTag:"), TAG_QUIT as usize);
        msg_id_id(quit, sel("setTarget:"), target);
        msg_id_id(menu, sel("addItem:"), quit);

        msg_id_id(item, sel("setMenu:"), menu);
    }
}
