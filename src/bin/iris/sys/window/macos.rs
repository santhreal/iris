//! AppKit window operations: the content view for file drags, capture
//! exclusion through the window's
//! sharing type, and moves through `performWindowDragWithEvent:` with
//! the mouse event being handled, which starts the system's drag and
//! follows the held button until release.

use iris_lib::objc::{class, sel, send, send1, Id};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The window's content `NSView`.
pub fn ns_view(window: &gpui::Window) -> Option<Id> {
    let RawWindowHandle::AppKit(h) = HasWindowHandle::window_handle(window).ok()?.as_raw() else {
        return None;
    };
    Some(h.ns_view.as_ptr() as Id)
}

fn ns_window(window: &gpui::Window) -> Option<Id> {
    let w: Id = unsafe { send(ns_view(window)?, sel(c"window")) };
    (!w.is_null()).then_some(w)
}

/// `NSWindowSharingNone`: screen capture and recording skip the window.
pub fn exclude_from_capture(window: &gpui::Window) {
    if let Some(w) = ns_window(window) {
        unsafe { send1::<usize, ()>(w, sel(c"setSharingType:"), 0) };
    }
}

pub fn begin_move(window: &gpui::Window) {
    let Some(ns_window) = ns_window(window) else {
        return;
    };
    unsafe {
        let app: Id = send(class(c"NSApplication") as Id, sel(c"sharedApplication"));
        let event: Id = send(app, sel(c"currentEvent"));
        if event.is_null() {
            return;
        }
        send1::<Id, ()>(ns_window, sel(c"performWindowDragWithEvent:"), event);
    }
}
