//! Window moves and resizes that follow the root pointer, for a window
//! manager that does not handle _NET_WM_MOVERESIZE: the window follows
//! the root pointer from the press until button 1 is up. X11 only;
//! `sys::window` sends _NET_WM_MOVERESIZE where the window manager
//! handles it, and routes Wayland to the compositor.
//!
//! The press gave GPUI's connection the pointer grab, so the X server
//! reports core pointer events to no other client. XInput 2.1 raw
//! events reach every client that selects them, grab or not: each one
//! wakes the drag, which reads the pointer and configures the window.

use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

use gpui::ResizeEdge;
use x11rb::connection::Connection;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _, KeyButMask};
use x11rb::rust_connection::RustConnection;

#[cfg(test)]
mod tests;

/// How long a drag with raw events waits for one before it reads the
/// pointer anyway: the bound on a drag whose release no raw event
/// reported.
const QUIET: Duration = Duration::from_millis(100);

/// The pointer read period on an X server without XInput 2.1: 125 Hz.
const POLL: Duration = Duration::from_millis(8);

/// The least time between two size changes. GPUI rebuilds its swapchain
/// for each, so a resize follows the pointer at up to 60 Hz; a move
/// follows every pointer read.
const RESIZE_PERIOD: Duration = Duration::from_millis(16);

/// A window rect in root pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Move window `xid` with the pointer from the press at `grab`
/// (window-local, root pixels) until the button is released.
pub fn begin_wm_move(xid: u32, grab: (i32, i32)) {
    follow(xid, grab, |r, (dx, dy)| Rect {
        x: r.x + dx,
        y: r.y + dy,
        ..r
    });
}

/// Resize window `xid` from `edge` with the pointer from the press at
/// `grab` until the button is released, never below `min` (root
/// pixels).
pub fn begin_wm_resize(xid: u32, grab: (i32, i32), edge: ResizeEdge, min: (i32, i32)) {
    let min = (min.0.max(1), min.1.max(1));
    follow(xid, grab, move |r, d| resized(r, edge, d, min));
}

/// `r` with `edge` dragged by `(dx, dy)`. The opposite edges stay where
/// they are, and the size stops at `min`.
pub(super) fn resized(
    r: Rect,
    edge: ResizeEdge,
    (dx, dy): (i32, i32),
    (min_w, min_h): (i32, i32),
) -> Rect {
    use ResizeEdge::*;
    let mut out = r;
    match edge {
        Left | TopLeft | BottomLeft => {
            out.w = (r.w - dx).max(min_w);
            out.x = r.x + r.w - out.w;
        }
        Right | TopRight | BottomRight => out.w = (r.w + dx).max(min_w),
        Top | Bottom => {}
    }
    match edge {
        Top | TopLeft | TopRight => {
            out.h = (r.h - dy).max(min_h);
            out.y = r.y + r.h - out.h;
        }
        Bottom | BottomLeft | BottomRight => out.h = (r.h + dy).max(min_h),
        Left | Right => {}
    }
    out
}

/// Follow the root pointer from the press at `grab` until button 1 is
/// up, configuring window `xid` to `place(start, travel)`: `start` is
/// its rect at the press, `travel` the pointer's offset from the press.
/// The release position is applied before the drag ends.
fn follow(xid: u32, grab: (i32, i32), place: impl Fn(Rect, (i32, i32)) -> Rect + Send + 'static) {
    std::thread::spawn(move || {
        // A connection per drag: its raw-event selection and its event
        // queue end with the drag.
        let Ok((conn, screen_num)) = x11rb::connect(None) else {
            return;
        };
        let root = conn.setup().roots[screen_num].root;
        let idle = if select_raw(&conn, root) { QUIET } else { POLL };
        let Some(start) = rect(&conn, xid, root) else {
            return;
        };
        let anchor = (start.x + grab.0, start.y + grab.1);
        let fd = conn.stream().as_raw_fd();
        let mut placed = start;
        let mut resized_at = Instant::now() - RESIZE_PERIOD;
        loop {
            // A raw event is only a wake; the pointer read is the state.
            while let Ok(Some(_)) = conn.poll_for_event() {}
            let Some(pointer) = conn
                .query_pointer(root)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
            else {
                return;
            };
            let held = pointer.mask.contains(KeyButMask::BUTTON1);
            let travel = (
                i32::from(pointer.root_x) - anchor.0,
                i32::from(pointer.root_y) - anchor.1,
            );
            let next = place(start, travel);
            let resizing = (next.w, next.h) != (placed.w, placed.h);
            let now = Instant::now();
            let due = resized_at + RESIZE_PERIOD;
            let mut timeout = idle;
            if held && resizing && now < due {
                timeout = due - now;
            } else {
                configure(&conn, xid, placed, next);
                let _ = conn.flush();
                if resizing {
                    resized_at = now;
                }
                placed = next;
            }
            if !held {
                return;
            }
            wait(fd, timeout);
        }
    });
}

/// Select XInput 2.1 raw motion and button releases of the master
/// pointers on `root`. False where the server has no XInput 2.1, whose
/// raw events stop at another client's grab.
fn select_raw(conn: &RustConnection, root: u32) -> bool {
    let Some(version) = conn
        .xinput_xi_query_version(2, 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return false;
    };
    if (version.major_version, version.minor_version) < (2, 1) {
        return false;
    }
    let mask = xinput::EventMask {
        deviceid: xinput::Device::ALL_MASTER.into(),
        mask: vec![xinput::XIEventMask::RAW_MOTION | xinput::XIEventMask::RAW_BUTTON_RELEASE],
    };
    conn.xinput_xi_select_events(root, &[mask])
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some()
}

/// Sleep until the X connection is readable or `timeout` passes.
fn wait(fd: i32, timeout: Duration) {
    let ms = timeout.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32;
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `pfd` is one valid pollfd for the duration of the call.
    unsafe { libc::poll(&mut pfd, 1, ms) };
}

/// The window's root position and size.
fn rect(conn: &RustConnection, xid: u32, root: u32) -> Option<Rect> {
    let pos = conn.translate_coordinates(xid, root, 0, 0).ok()?;
    let size = conn.get_geometry(xid).ok()?;
    let (pos, size) = (pos.reply().ok()?, size.reply().ok()?);
    Some(Rect {
        x: pos.dst_x.into(),
        y: pos.dst_y.into(),
        w: size.width.into(),
        h: size.height.into(),
    })
}

/// Configure the fields of `next` that differ from `prev`: a move sends
/// no size, so the window does not lay out again for it.
fn configure(conn: &RustConnection, xid: u32, prev: Rect, next: Rect) {
    if next == prev {
        return;
    }
    let mut aux = ConfigureWindowAux::new();
    if next.x != prev.x {
        aux = aux.x(next.x);
    }
    if next.y != prev.y {
        aux = aux.y(next.y);
    }
    if next.w != prev.w {
        aux = aux.width(next.w as u32);
    }
    if next.h != prev.h {
        aux = aux.height(next.h as u32);
    }
    let _ = conn.configure_window(xid, &aux);
}
