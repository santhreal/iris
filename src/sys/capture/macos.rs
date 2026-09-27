//! macOS capture through CoreGraphics.
//!
//! Root space is physical pixels, like the other backends: global
//! display points scaled by the highest backing scale among the active
//! displays. `CGWindowListCreateImage` renders a rect spanning several
//! displays at that same scale, so frames, monitor rects, and window
//! rects share one coordinate space. Recording still uses ffmpeg's
//! avfoundation input (`capture_args`).
#![cfg(target_os = "macos")]

use core::ffi::c_void;

use crate::capture::{Frame, WinRect};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

type CFTypeRef = *const c_void;
type Ref = *mut c_void;

const K_CG_WINDOW_LIST_ON_SCREEN_ONLY: u32 = 1 << 0;
const K_CG_WINDOW_LIST_EXCLUDE_DESKTOP: u32 = 1 << 4;
const K_CG_NULL_WINDOW_ID: u32 = 0;
const K_CG_WINDOW_IMAGE_DEFAULT: u32 = 0;
const K_CG_IMAGE_ALPHA_PREMULTIPLIED_LAST: u32 = 1;
const K_CG_BITMAP_BYTE_ORDER_32_BIG: u32 = 4 << 12;
const K_CF_NUMBER_SINT32: isize = 3;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCopyDisplayMode(display: u32) -> Ref;
    fn CGDisplayModeGetPixelWidth(mode: Ref) -> usize;
    fn CGDisplayModeRelease(mode: Ref);
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGWindowListCreateImage(bounds: CGRect, list: u32, window: u32, image: u32) -> Ref;
    fn CGWindowListCopyWindowInfo(list: u32, relative_to: u32) -> Ref;
    fn CGRectMakeWithDictionaryRepresentation(dict: CFTypeRef, rect: *mut CGRect) -> bool;
    fn CGImageGetWidth(image: Ref) -> usize;
    fn CGImageGetHeight(image: Ref) -> usize;
    fn CGImageRelease(image: Ref);
    fn CGColorSpaceCreateDeviceRGB() -> Ref;
    fn CGColorSpaceRelease(space: Ref);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: Ref,
        info: u32,
    ) -> Ref;
    fn CGContextDrawImage(ctx: Ref, rect: CGRect, image: Ref);
    fn CGContextRelease(ctx: Ref);
    static kCGWindowLayer: CFTypeRef;
    static kCGWindowBounds: CFTypeRef;
    static kCGWindowOwnerPID: CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, kind: isize, out: *mut c_void) -> bool;
    fn CFRelease(cf: CFTypeRef);
}

/// Fail with a corrective message, and raise the system prompt, when
/// Screen Recording permission is missing. Without it CoreGraphics
/// returns the wallpaper with every window removed instead of failing.
fn require_permission() -> Result<(), String> {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return Ok(());
    }
    unsafe { CGRequestScreenCaptureAccess() };
    Err(
        "iris needs Screen Recording permission: enable it in System Settings > \
         Privacy & Security > Screen Recording, then restart iris"
            .to_string(),
    )
}

/// Active displays in CoreGraphics order (the order ffmpeg's
/// avfoundation numbers its `Capture screen N` devices).
fn display_list() -> Result<Vec<u32>, String> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    let err = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    if err != 0 {
        return Err(format!("CGGetActiveDisplayList failed ({err})"));
    }
    Ok(ids[..count as usize].to_vec())
}

/// Active displays, main display first.
fn displays() -> Result<Vec<u32>, String> {
    let main = unsafe { CGMainDisplayID() };
    let mut out = display_list()?;
    out.sort_by_key(|&d| d != main);
    Ok(out)
}

/// `(x, y, w, h)` in one display's own pixels.
pub type Crop = (u32, u32, u32, u32);

/// The display to record and the crop within it: `(ordinal, crop)`,
/// where `ordinal` counts avfoundation screen devices and `crop` is
/// `(x, y, w, h)` in that display's own pixels. `rect` (root pixels)
/// selects the display holding its center; `None` records the main
/// display uncropped.
pub fn recording_screen(rect: Option<WinRect>) -> Result<(usize, Option<Crop>), String> {
    require_permission()?;
    let ids = display_list()?;
    let root = root_scale_of(&ids);
    let Some(r) = rect else {
        let main = unsafe { CGMainDisplayID() };
        let ord = ids.iter().position(|&d| d == main).unwrap_or(0);
        return Ok((ord, None));
    };
    let (cx, cy) = (r.x + r.width as i32 / 2, r.y + r.height as i32 / 2);
    for (ord, &d) in ids.iter().enumerate() {
        let b = to_root(unsafe { CGDisplayBounds(d) }, root);
        if cx < b.x || cy < b.y || cx >= b.x + b.width as i32 || cy >= b.y + b.height as i32 {
            continue;
        }
        // Root pixels to this display's pixels.
        let f = display_scale(d) / root;
        let x = ((r.x - b.x).max(0) as f64 * f) as u32;
        let y = ((r.y - b.y).max(0) as f64 * f) as u32;
        let w = (r.width.min(b.width) as f64 * f) as u32;
        let h = (r.height.min(b.height) as f64 * f) as u32;
        return Ok((ord, Some((x, y, w, h))));
    }
    Err("the selected region is not on any display".to_string())
}

/// Backing scale of `display`: physical pixel width over point width.
fn display_scale(display: u32) -> f64 {
    let points = unsafe { CGDisplayBounds(display) }.size.width;
    let mode = unsafe { CGDisplayCopyDisplayMode(display) };
    if mode.is_null() || points <= 0.0 {
        return 1.0;
    }
    let pixels = unsafe { CGDisplayModeGetPixelWidth(mode) } as f64;
    unsafe { CGDisplayModeRelease(mode) };
    (pixels / points).max(1.0)
}

/// Points-to-root-pixels factor: the highest display backing scale.
fn root_scale_of(displays: &[u32]) -> f64 {
    displays
        .iter()
        .map(|&d| display_scale(d))
        .fold(1.0, f64::max)
}

/// Root pixels per GPUI logical pixel (a point).
pub fn root_scale() -> f32 {
    displays().map(|d| root_scale_of(&d) as f32).unwrap_or(1.0)
}

fn to_root(r: CGRect, scale: f64) -> WinRect {
    WinRect {
        x: (r.origin.x * scale).round() as i32,
        y: (r.origin.y * scale).round() as i32,
        width: (r.size.width * scale).round().max(0.0) as u32,
        height: (r.size.height * scale).round().max(0.0) as u32,
    }
}

fn to_points(r: WinRect, scale: f64) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: r.x as f64 / scale,
            y: r.y as f64 / scale,
        },
        size: CGSize {
            width: r.width as f64 / scale,
            height: r.height as f64 / scale,
        },
    }
}

/// Per-monitor rectangles in root pixels, main display first.
pub fn monitors() -> Result<Vec<WinRect>, String> {
    let ids = displays()?;
    let scale = root_scale_of(&ids);
    Ok(ids
        .iter()
        .map(|&d| to_root(unsafe { CGDisplayBounds(d) }, scale))
        .collect())
}

/// On-screen normal-layer windows, front to back, excluding iris's own.
fn windows(scale: f64) -> Result<Vec<WinRect>, String> {
    let list = unsafe {
        CGWindowListCopyWindowInfo(
            K_CG_WINDOW_LIST_ON_SCREEN_ONLY | K_CG_WINDOW_LIST_EXCLUDE_DESKTOP,
            K_CG_NULL_WINDOW_ID,
        )
    };
    if list.is_null() {
        return Err("CGWindowListCopyWindowInfo returned no list".to_string());
    }
    let own_pid = std::process::id() as i32;
    let mut out = Vec::new();
    unsafe {
        for i in 0..CFArrayGetCount(list) {
            let info = CFArrayGetValueAtIndex(list, i);
            let int = |key: CFTypeRef| {
                let n = CFDictionaryGetValue(info, key);
                let mut v = 0i32;
                (!n.is_null()
                    && CFNumberGetValue(n, K_CF_NUMBER_SINT32, &mut v as *mut i32 as *mut c_void))
                .then_some(v)
            };
            if int(kCGWindowLayer) != Some(0) || int(kCGWindowOwnerPID) == Some(own_pid) {
                continue;
            }
            let bounds = CFDictionaryGetValue(info, kCGWindowBounds);
            let mut r = CGRect::default();
            if bounds.is_null() || !CGRectMakeWithDictionaryRepresentation(bounds, &mut r) {
                continue;
            }
            let rect = to_root(r, scale);
            if rect.width > 0 && rect.height > 0 {
                out.push(rect);
            }
        }
        CFRelease(list);
    }
    Ok(out)
}

/// Monitors plus on-screen windows for the overlay's hover-snap.
pub fn layout() -> Result<(Vec<WinRect>, Vec<WinRect>), String> {
    let ids = displays()?;
    let scale = root_scale_of(&ids);
    let mons = ids
        .iter()
        .map(|&d| to_root(unsafe { CGDisplayBounds(d) }, scale))
        .collect();
    Ok((mons, windows(scale)?))
}

/// The frontmost normal window: the window list is ordered front to
/// back, so its first entry not owned by iris.
pub fn active_window_rect() -> Result<WinRect, String> {
    let scale = root_scale_of(&displays()?);
    windows(scale)?
        .into_iter()
        .next()
        .ok_or_else(|| "no on-screen window to capture".to_string())
}

/// Render the on-screen contents of `rect` (points) into an RGBA frame.
fn grab_points(rect: CGRect) -> Result<Frame, String> {
    require_permission()?;
    let image = unsafe {
        CGWindowListCreateImage(
            rect,
            K_CG_WINDOW_LIST_ON_SCREEN_ONLY,
            K_CG_NULL_WINDOW_ID,
            K_CG_WINDOW_IMAGE_DEFAULT,
        )
    };
    if image.is_null() {
        return Err("CGWindowListCreateImage returned no image".to_string());
    }
    let (w, h) = unsafe { (CGImageGetWidth(image), CGImageGetHeight(image)) };
    let mut rgba = vec![0u8; w * h * 4];
    let drawn = unsafe {
        let space = CGColorSpaceCreateDeviceRGB();
        let ctx = CGBitmapContextCreate(
            rgba.as_mut_ptr() as *mut c_void,
            w,
            h,
            8,
            w * 4,
            space,
            K_CG_IMAGE_ALPHA_PREMULTIPLIED_LAST | K_CG_BITMAP_BYTE_ORDER_32_BIG,
        );
        CGColorSpaceRelease(space);
        let ok = !ctx.is_null();
        if ok {
            let full = CGRect {
                origin: CGPoint::default(),
                size: CGSize {
                    width: w as f64,
                    height: h as f64,
                },
            };
            CGContextDrawImage(ctx, full, image);
            CGContextRelease(ctx);
        }
        CGImageRelease(image);
        ok
    };
    if !drawn || w == 0 || h == 0 {
        return Err(format!("cannot read a {w}x{h} screen image"));
    }
    Ok(Frame {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

/// Grab one rect of root space.
pub fn grab_rect(rect: WinRect) -> Result<Frame, String> {
    let scale = root_scale_of(&displays()?);
    grab_points(to_points(rect, scale))
}

/// Grab the union of every display.
pub fn capture_full_frame() -> Result<Frame, String> {
    let ids = displays()?;
    let union = ids
        .iter()
        .map(|&d| unsafe { CGDisplayBounds(d) })
        .reduce(|a, b| {
            let x0 = a.origin.x.min(b.origin.x);
            let y0 = a.origin.y.min(b.origin.y);
            let x1 = (a.origin.x + a.size.width).max(b.origin.x + b.size.width);
            let y1 = (a.origin.y + a.size.height).max(b.origin.y + b.size.height);
            CGRect {
                origin: CGPoint { x: x0, y: y0 },
                size: CGSize {
                    width: x1 - x0,
                    height: y1 - y0,
                },
            }
        })
        .ok_or("no active display")?;
    grab_points(union)
}

pub struct Backend;

impl crate::capture::CaptureBackend for Backend {
    fn grab_screen(&self) -> Result<Frame, String> {
        capture_full_frame()
    }
}
