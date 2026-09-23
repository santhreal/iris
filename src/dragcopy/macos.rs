//! Files on macOS as `NSURL` file URLs: written to the general
//! `NSPasteboard` for Copy (Finder pastes the files, text fields receive
//! the paths), and carried by an `NSDraggingSession` for drag-out.

use std::path::PathBuf;

use crate::objc::{
    class, class_addMethod, nsstring, objc_allocateClassPair, objc_registerClassPair, sel, send,
    send1, send2, send3, with_autorelease_pool, Bool, Id, Imp, Sel,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSRect {
    origin: NSPoint,
    w: f64,
    h: f64,
}

/// A +0 file `NSURL` for `p`, or `None` when AppKit cannot form one.
unsafe fn file_url(p: &std::path::Path) -> Option<Id> {
    let s = nsstring(&p.to_string_lossy());
    let url: Id = send1(class(c"NSURL"), sel(c"fileURLWithPath:"), s);
    // nsstring returns +1; the URL holds its own reference.
    send::<()>(s, sel(c"release"));
    (!url.is_null()).then_some(url)
}

/// Replace the general pasteboard's contents with `paths` (absolute).
pub(super) fn copy_abs_paths(paths: &[PathBuf]) -> Result<(), String> {
    with_autorelease_pool(|| unsafe {
        let pb: Id = send(class(c"NSPasteboard"), sel(c"generalPasteboard"));
        if pb.is_null() {
            return Err("NSPasteboard unavailable".to_string());
        }
        let urls: Id = send(class(c"NSMutableArray"), sel(c"array"));
        for p in paths {
            let url = file_url(p)
                .ok_or_else(|| format!("cannot form a file URL for {}", p.display()))?;
            send1::<Id, ()>(urls, sel(c"addObject:"), url);
        }
        let _: isize = send(pb, sel(c"clearContents"));
        let ok: Bool = send1(pb, sel(c"writeObjects:"), urls);
        if ok == 0 {
            return Err("NSPasteboard writeObjects: failed".to_string());
        }
        Ok(())
    })
}

/// `NSDragOperationCopy`: the drop target receives copies of the files.
const NS_DRAG_OPERATION_COPY: usize = 1;

unsafe extern "C" fn source_operation_mask(_this: Id, _cmd: Sel, _session: Id, _ctx: isize) -> usize {
    NS_DRAG_OPERATION_COPY
}

/// The `NSDraggingSource` for every file drag: an NSObject subclass
/// answering `draggingSession:sourceOperationMaskForDraggingContext:`
/// with Copy. One instance for the process; sessions do not retain it.
unsafe fn drag_source() -> Id {
    static SOURCE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *SOURCE.get_or_init(|| {
        let c = objc_allocateClassPair(class(c"NSObject"), c"IrisDragSource".as_ptr(), 0);
        class_addMethod(
            c,
            sel(c"draggingSession:sourceOperationMaskForDraggingContext:"),
            source_operation_mask as Imp,
            c"Q@:@q".as_ptr(),
        );
        objc_registerClassPair(c);
        let alloc: Id = send(c, sel(c"alloc"));
        send::<Id>(alloc, sel(c"init")) as usize
    }) as Id
}

/// Drag `paths` (absolute) out of `ns_view` as files, from the mouse
/// event being handled. Each file is one dragging item imaged with its
/// Finder icon. Main thread only, inside a mouse-down or mouse-dragged
/// handler: AppKit tracks the drag from that event.
pub(super) fn drag_abs_paths(ns_view: Id, paths: &[PathBuf]) -> Result<(), String> {
    if ns_view.is_null() {
        return Err("drag: no window view".to_string());
    }
    with_autorelease_pool(|| unsafe {
        let app: Id = send(class(c"NSApplication"), sel(c"sharedApplication"));
        let event: Id = send(app, sel(c"currentEvent"));
        if event.is_null() {
            return Err("drag: no current mouse event".to_string());
        }
        let in_window: NSPoint = send(event, sel(c"locationInWindow"));
        let at: NSPoint = send2(
            ns_view,
            sel(c"convertPoint:fromView:"),
            in_window,
            core::ptr::null_mut::<core::ffi::c_void>(),
        );
        let workspace: Id = send(class(c"NSWorkspace"), sel(c"sharedWorkspace"));
        let items: Id = send(class(c"NSMutableArray"), sel(c"array"));
        for (i, p) in paths.iter().enumerate() {
            let url = file_url(p)
                .ok_or_else(|| format!("cannot form a file URL for {}", p.display()))?;
            let alloc: Id = send(class(c"NSDraggingItem"), sel(c"alloc"));
            let item: Id = send1(alloc, sel(c"initWithPasteboardWriter:"), url);
            let path = nsstring(&p.to_string_lossy());
            let icon: Id = send1(workspace, sel(c"iconForFile:"), path);
            send::<()>(path, sel(c"release"));
            // Stack the images slightly so a multi-file drag reads as one.
            let off = i as f64 * 6.0;
            let frame = NSRect {
                origin: NSPoint {
                    x: at.x - 32.0 + off,
                    y: at.y - 32.0 - off,
                },
                w: 64.0,
                h: 64.0,
            };
            send2::<NSRect, Id, ()>(item, sel(c"setDraggingFrame:contents:"), frame, icon);
            send1::<Id, ()>(items, sel(c"addObject:"), item);
            send::<()>(item, sel(c"release"));
        }
        let session: Id = send3(
            ns_view,
            sel(c"beginDraggingSessionWithItems:event:source:"),
            items,
            event,
            drag_source(),
        );
        if session.is_null() {
            return Err("drag: AppKit refused the dragging session".to_string());
        }
        Ok(())
    })
}
