//! The system-tray icon on macOS via `NSStatusItem`.
//!
//! The status item lives in the menu bar and owns an `NSMenu` of the
//! same commands as the other platforms. Menu items need an ObjC
//! target that responds to a selector, so `spawn` defines a tiny
//! `IrisTrayTarget` class at runtime whose `onItem:` reads the item's
//! tag and pushes the matching `Command` onto the daemon channel.
//! All runtime calls go through `iris_lib::objc`'s typed sends.

use futures::channel::mpsc::UnboundedSender;
use std::sync::OnceLock;

use iris_lib::objc::{
    class, class_addMethod, nsstring, objc_allocateClassPair, objc_registerClassPair, sel, send,
    send1, send3, Id, Imp, Sel,
};

use crate::daemon::Command;

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
    let tag: isize = send(sender, sel(c"tag"));
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
        let nsobject = class(c"NSObject");
        let c = objc_allocateClassPair(nsobject, c"IrisTrayTarget".as_ptr(), 0);
        class_addMethod(c, sel(c"onItem:"), on_item as Imp, c"v@:@".as_ptr());
        objc_registerClassPair(c);
        let alloc: Id = send(c, sel(c"alloc"));
        TARGET = send(alloc, sel(c"init"));
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
        let bar: Id = send(class(c"NSStatusBar"), sel(c"systemStatusBar"));
        if bar.is_null() {
            iris_lib::ilog!("iris: tray: NSStatusBar unavailable");
            return;
        }
        // NSVariableStatusItemLength = -1.
        let item: Id = send1(bar, sel(c"statusItemWithLength:"), -1.0f64);
        if item.is_null() {
            iris_lib::ilog!("iris: tray: statusItemWithLength failed");
            return;
        }
        // The item is autoreleased and AppKit removes it from the menu
        // bar once released: hold it for the process lifetime.
        let _: Id = send(item, sel(c"retain"));
        let button: Id = send(item, sel(c"button"));
        if !button.is_null() {
            send1::<Id, ()>(button, sel(c"setTitle:"), nsstring("iris"));
        }

        let target = tray_target();
        let menu: Id = send(send::<Id>(class(c"NSMenu"), sel(c"alloc")), sel(c"init"));
        let entries = [
            (TAG_CAPTURE, "Capture"),
            (TAG_RECORD, "Record window"),
            (TAG_LIBRARY, "Library"),
            (TAG_SETTINGS, "Settings"),
            (0, ""),
            (TAG_QUIT, "Quit"),
        ];
        for (tag, label) in entries {
            // Tag 0 marks the separator before Quit.
            let mi: Id = if tag == 0 {
                send(class(c"NSMenuItem"), sel(c"separatorItem"))
            } else {
                let mi: Id = send3(
                    send::<Id>(class(c"NSMenuItem"), sel(c"alloc")),
                    sel(c"initWithTitle:action:keyEquivalent:"),
                    nsstring(label),
                    sel(c"onItem:"),
                    nsstring(""),
                );
                send1::<isize, ()>(mi, sel(c"setTag:"), tag);
                send1::<Id, ()>(mi, sel(c"setTarget:"), target);
                mi
            };
            send1::<Id, ()>(menu, sel(c"addItem:"), mi);
        }
        send1::<Id, ()>(item, sel(c"setMenu:"), menu);
    }
}
