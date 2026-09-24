//! macOS: Finder folder windows through AppleScript, window rects from
//! `CGWindowListCopyWindowInfo`, and pointer input through `CGEventPost`.

use super::Rect;
use std::ffi::{c_char, c_void, CStr};
use std::path::Path;
use std::process::Command;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGPoint,
}

type Ref = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    static kCGWindowOwnerName: Ref;
    static kCGWindowName: Ref;
    static kCGWindowBounds: Ref;
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> Ref;
    fn CGRectMakeWithDictionaryRepresentation(dict: Ref, rect: *mut CGRect) -> bool;
    fn CGEventCreateMouseEvent(source: Ref, kind: u32, at: CGPoint, button: u32) -> Ref;
    fn CGEventSetIntegerValueField(event: Ref, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: Ref);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    fn CFDictionaryGetValue(dict: Ref, key: Ref) -> Ref;
    fn CFStringGetCString(s: Ref, buf: *mut c_char, len: isize, encoding: u32) -> bool;
    fn CFRelease(cf: Ref);
}

const ON_SCREEN_ONLY: u32 = 1;
const UTF8: u32 = 0x0800_0100;
const HID_EVENT_TAP: u32 = 0;
const MOUSE_CLICK_STATE: u32 = 1;
const LEFT_MOUSE_DOWN: u32 = 1;
const LEFT_MOUSE_UP: u32 = 2;
const MOUSE_MOVED: u32 = 5;
const LEFT_MOUSE_DRAGGED: u32 = 6;

pub fn init() {}

unsafe fn string(s: Ref) -> String {
    let mut buf: [c_char; 512] = [0; 512];
    if s.is_null() || !CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) {
        return String::new();
    }
    CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
}

/// Every on-screen window, front to back: owner, title, bounds.
fn windows() -> Vec<(String, String, Rect)> {
    let mut out = Vec::new();
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY, 0);
        if list.is_null() {
            return out;
        }
        for i in 0..CFArrayGetCount(list) {
            let info = CFArrayGetValueAtIndex(list, i);
            let mut r = CGRect::default();
            let bounds = CFDictionaryGetValue(info, kCGWindowBounds);
            if bounds.is_null() || !CGRectMakeWithDictionaryRepresentation(bounds, &mut r) {
                continue;
            }
            out.push((
                string(CFDictionaryGetValue(info, kCGWindowOwnerName)),
                string(CFDictionaryGetValue(info, kCGWindowName)),
                Rect {
                    x: r.origin.x,
                    y: r.origin.y,
                    w: r.size.x,
                    h: r.size.y,
                },
            ));
        }
        CFRelease(list);
    }
    out
}

pub fn describe() -> String {
    windows()
        .into_iter()
        .map(|(owner, title, r)| format!("  {owner} {title:?} {r:?}\n"))
        .collect()
}

pub fn toast() -> Option<Rect> {
    windows()
        .into_iter()
        .find(|(owner, title, _)| owner == "iris" && title == super::TOAST)
        .map(|(_, _, r)| r)
}

/// A Finder window on a folder; closed on drop.
pub struct Folder {
    name: String,
    pub rect: Rect,
}

fn osascript(script: &str) -> String {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "osascript: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Open `dir` in Finder at the top left of the main display.
pub fn open_folder(dir: &Path) -> Folder {
    let bounds = osascript(&format!(
        "tell application \"Finder\"\n\
           activate\n\
           set w to make new Finder window\n\
           set target of w to (POSIX file \"{}\" as alias)\n\
           set current view of w to icon view\n\
           set bounds of w to {{40, 80, 560, 480}}\n\
           get bounds of w\n\
         end tell",
        dir.display()
    ));
    let b: Vec<f64> = bounds
        .split(", ")
        .map(|v| v.parse().expect("Finder window bounds are numbers"))
        .collect();
    assert_eq!(b.len(), 4, "Finder window bounds {bounds:?}");
    // Finder draws the folder's contents a moment after the window.
    std::thread::sleep(std::time::Duration::from_millis(800));
    Folder {
        name: dir.file_name().unwrap().to_string_lossy().into_owned(),
        rect: Rect {
            x: b[0],
            y: b[1],
            w: b[2] - b[0],
            h: b[3] - b[1],
        },
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "tell application \"Finder\" to close (every window whose name is \"{}\")",
                self.name
            ))
            .output();
    }
}

fn post(kind: u32, (x, y): (f64, f64)) {
    unsafe {
        let event = CGEventCreateMouseEvent(std::ptr::null(), kind, CGPoint { x, y }, 0);
        if event.is_null() {
            eprintln!("CGEventCreateMouseEvent({kind}) failed");
            return;
        }
        if kind != MOUSE_MOVED {
            CGEventSetIntegerValueField(event, MOUSE_CLICK_STATE, 1);
        }
        CGEventPost(HID_EVENT_TAP, event);
        CFRelease(event);
    }
}

pub fn hover(at: (f64, f64)) {
    post(MOUSE_MOVED, at);
}

pub fn press(at: (f64, f64)) {
    post(LEFT_MOUSE_DOWN, at);
}

pub fn drag_to(at: (f64, f64)) {
    post(LEFT_MOUSE_DRAGGED, at);
}

pub fn release(at: (f64, f64)) {
    post(LEFT_MOUSE_UP, at);
}
