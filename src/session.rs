//! The display session this process runs in.
//!
//! GPUI opens its windows as Wayland surfaces whenever `WAYLAND_DISPLAY`
//! is set and non-empty, and as X11 windows otherwise. Every capture,
//! recording, clipboard, and window path selects its backend by the same
//! test, so iris never mixes the two: on a Wayland session an XWayland
//! `DISPLAY` reaches only X11 clients, not the session's own windows or
//! pixels.

use std::ffi::OsStr;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Wayland,
    X11,
}

/// True on a Wayland session. Always false off Linux.
pub fn wayland() -> bool {
    current() == Some(Kind::Wayland)
}

/// True on an X11 session: `DISPLAY` is set and no Wayland compositor
/// is. Always false off Linux.
pub fn x11() -> bool {
    current() == Some(Kind::X11)
}

fn current() -> Option<Kind> {
    kind(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("DISPLAY").as_deref(),
    )
}

/// GPUI's selection rule: a non-empty `WAYLAND_DISPLAY` wins over
/// `DISPLAY`.
fn kind(wayland_display: Option<&OsStr>, display: Option<&OsStr>) -> Option<Kind> {
    let set = |v: Option<&OsStr>| v.is_some_and(|v| !v.is_empty());
    if !cfg!(target_os = "linux") {
        None
    } else if set(wayland_display) {
        Some(Kind::Wayland)
    } else if set(display) {
        Some(Kind::X11)
    } else {
        None
    }
}

// WHY: the class closed here is "one path decides the session differently
// from GPUI": recording once routed a Wayland session with XWayland to
// the X11 recorder because it required DISPLAY to be unset. The cases pin
// GPUI's rule on the pure classifier, so no test mutates the process
// environment. Not covered: a compositor that sets neither variable.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn session_follows_gpui_selection() {
        let os = |s: Option<&'static str>| s.map(OsStr::new);
        let cases = [
            (Some("wayland-0"), None, Some(Kind::Wayland)),
            (Some("wayland-0"), Some(":0"), Some(Kind::Wayland)),
            (Some(""), Some(":0"), Some(Kind::X11)),
            (None, Some(":0"), Some(Kind::X11)),
            (None, Some(""), None),
            (Some(""), None, None),
            (None, None, None),
        ];
        for (wl, x, want) in cases {
            assert_eq!(
                kind(os(wl), os(x)),
                want,
                "WAYLAND_DISPLAY={wl:?} DISPLAY={x:?}"
            );
        }
    }
}
