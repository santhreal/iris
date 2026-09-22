//! Minimal Objective-C runtime access for the macOS code paths (tray,
//! pasteboard).
//!
//! `objc_msgSend` has no fixed signature: each call site must invoke it
//! through a function pointer of the exact callee type. Calling it as a
//! C variadic is wrong on arm64, where variadic arguments travel on the
//! stack and `objc_msgSend` reads registers. The `send*` helpers do the
//! typed cast; the caller supplies argument and return types.
//!
//! Class and selector names are `&CStr`, so every name reaching the
//! runtime is NUL-terminated.

use core::ffi::{c_char, c_void, CStr};

pub type Id = *mut c_void;
pub type Sel = *mut c_void;
pub type Class = *mut c_void;
pub type Imp = *const c_void;

/// `BOOL` as returned in a register: `bool` on arm64, `signed char` on
/// x86_64. Both occupy the low byte, so reading an `i8` is correct on
/// both.
pub type Bool = i8;

#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn sel_registerName(name: *const c_char) -> Sel;
    pub fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
    pub fn objc_registerClassPair(cls: Class);
    pub fn class_addMethod(cls: Class, name: Sel, imp: Imp, types: *const c_char) -> Bool;
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
    // Declared without parameters, per Apple's guidance: it is only ever
    // called through a pointer cast to the concrete callee type.
    fn objc_msgSend();
}

// Foundation must be loaded for its classes to resolve through
// `objc_getClass`; AppKit pulls it in for the pasteboard and tray.
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

pub fn class(name: &CStr) -> Class {
    unsafe { objc_getClass(name.as_ptr()) }
}

pub fn sel(name: &CStr) -> Sel {
    unsafe { sel_registerName(name.as_ptr()) }
}

/// # Safety
/// `obj` must respond to `sel` with signature `R (void)`.
pub unsafe fn send<R>(obj: Id, sel: Sel) -> R {
    let f: unsafe extern "C" fn(Id, Sel) -> R =
        core::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel)
}

/// # Safety
/// `obj` must respond to `sel` with signature `R (A)`.
pub unsafe fn send1<A, R>(obj: Id, sel: Sel, a: A) -> R {
    let f: unsafe extern "C" fn(Id, Sel, A) -> R =
        core::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel, a)
}

/// # Safety
/// `obj` must respond to `sel` with signature `R (A, B, C)`.
pub unsafe fn send3<A, B, C, R>(obj: Id, sel: Sel, a: A, b: B, c: C) -> R {
    let f: unsafe extern "C" fn(Id, Sel, A, B, C) -> R =
        core::mem::transmute(objc_msgSend as unsafe extern "C" fn());
    f(obj, sel, a, b, c)
}

/// An owned (+1) `NSString` holding `s`.
pub fn nsstring(s: &str) -> Id {
    const NS_UTF8_STRING_ENCODING: usize = 4;
    unsafe {
        let alloc: Id = send(class(c"NSString"), sel(c"alloc"));
        send3(
            alloc,
            sel(c"initWithBytes:length:encoding:"),
            s.as_ptr() as *const c_void,
            s.len(),
            NS_UTF8_STRING_ENCODING,
        )
    }
}

/// Run `f` inside an autorelease pool, so autoreleased objects created
/// off the main run loop are released when `f` returns.
pub fn with_autorelease_pool<T>(f: impl FnOnce() -> T) -> T {
    struct Pool(*mut c_void);
    impl Drop for Pool {
        fn drop(&mut self) {
            unsafe { objc_autoreleasePoolPop(self.0) }
        }
    }
    let _pool = Pool(unsafe { objc_autoreleasePoolPush() });
    f()
}
