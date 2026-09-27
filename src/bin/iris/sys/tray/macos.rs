//! The system-tray icon on macOS via `NSStatusItem`.
//!
//! The status item lives in the menu bar and owns an `NSMenu` drawn
//! from the shared rows. Menu items need an ObjC target that responds
//! to a selector, so `spawn` defines a tiny `IrisTrayTarget` class at
//! runtime whose `onItem:` reads the item's tag, the row id, and
//! reports the pick. All runtime calls go through
//! `iris_lib::sys::objc`'s typed sends.

use iris_lib::sys::objc::{
    class, class_addMethod, nsstring, objc_allocateClassPair, objc_registerClassPair, sel, send,
    send1, send3, Id, Imp, Sel,
};

use super::Row;

/// `onItem:` — the menu action. `sender` is the NSMenuItem; its tag
/// is the row id. Signature `v@:@` (void, self, _cmd, sender).
unsafe extern "C" fn on_item(_this: Id, _cmd: Sel, sender: Id) {
    let tag: isize = send(sender, sel(c"tag"));
    if let Ok(id) = usize::try_from(tag) {
        super::pick(id);
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
pub(super) fn spawn() {
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
        for (id, row) in super::rows() {
            let mi: Id = match row {
                Row::Separator => send(class(c"NSMenuItem"), sel(c"separatorItem")),
                Row::Item { label, .. } => {
                    let mi: Id = send3(
                        send::<Id>(class(c"NSMenuItem"), sel(c"alloc")),
                        sel(c"initWithTitle:action:keyEquivalent:"),
                        nsstring(label),
                        sel(c"onItem:"),
                        nsstring(""),
                    );
                    // Row ids are small: the cast cannot wrap.
                    send1::<isize, ()>(mi, sel(c"setTag:"), id as isize);
                    send1::<Id, ()>(mi, sel(c"setTarget:"), target);
                    mi
                }
            };
            send1::<Id, ()>(menu, sel(c"addItem:"), mi);
        }
        send1::<Id, ()>(item, sel(c"setMenu:"), menu);
    }
}
