//! Window shapes for an X11 screen with no compositing manager, where a
//! transparent pixel shows black: the X Shape extension cuts a pop-up
//! window's bounding region to the areas it paints, so the screen
//! beneath shows through everywhere else.

use std::sync::LazyLock;

use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::{AtomEnum, ClipOrdering, ConnectionExt as _, Rectangle};

use crate::sys::window::Rounded;

#[cfg(test)]
mod tests;

/// A rectangle in device pixels: x, y, width, height.
type Span = [i32; 4];

/// Whether a compositing manager runs on the X screen: a client owns
/// the screen's `_NET_WM_CM_S<n>` selection (EWMH), or the root has a
/// `_NET_WM_CM_OWNER` property, as some older compositing managers set
/// instead. GPUI's X11 backend reads the same two when a window opens.
/// With no X connection the answer is true: iris then has no X11
/// window to draw.
pub fn compositor() -> bool {
    // The selection's name depends on the screen, which the shared
    // connection keeps for its life.
    static ATOMS: LazyLock<Option<(u32, u32)>> = LazyLock::new(|| {
        let (conn, screen) = iris_lib::sys::capture::x11::shared_conn().ok()?;
        let selection = conn
            .intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())
            .ok()?;
        let owner = conn.intern_atom(false, b"_NET_WM_CM_OWNER").ok()?;
        Some((selection.reply().ok()?.atom, owner.reply().ok()?.atom))
    });
    let (Ok((conn, screen)), Some((selection, owner))) =
        (iris_lib::sys::capture::x11::shared_conn(), *ATOMS)
    else {
        return true;
    };
    let root = conn.setup().roots[screen].root;
    let owned = conn.get_selection_owner(selection);
    let announced = conn.get_property(false, root, owner, AtomEnum::WINDOW, 0, 1);
    owned
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| reply.owner != x11rb::NONE)
        || announced
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| reply.value_len > 0)
}

/// The bounding shape last set on one window, so a frame that paints
/// the same areas sends no request.
#[derive(Default)]
pub struct Cutout {
    sent: Option<Vec<Span>>,
    next: Vec<Span>,
}

impl Cutout {
    /// Cut window `xid` to `areas`, window-local logical pixels at
    /// `scale` device pixels each.
    pub fn set(&mut self, xid: u32, scale: f32, areas: &[Rounded]) {
        self.next.clear();
        spans(&mut self.next, scale, areas);
        if self.sent.as_ref() == Some(&self.next) {
            return;
        }
        let Ok((conn, _)) = iris_lib::sys::capture::x11::shared_conn() else {
            return;
        };
        let rects: Vec<Rectangle> = self.next.iter().map(|&s| rectangle(s)).collect();
        let sent = conn.shape_rectangles(
            SO::SET,
            SK::BOUNDING,
            ClipOrdering::UNSORTED,
            xid,
            0,
            0,
            &rects,
        );
        if sent.is_ok() && conn.flush().is_ok() {
            std::mem::swap(self.sent.get_or_insert_with(Vec::new), &mut self.next);
        }
    }
}

/// Append to `out` the device-pixel rectangles covering `areas`,
/// window-local logical pixels at `scale` device pixels each. Each edge
/// rounds to the nearest device pixel on its own, so two areas that
/// meet in logical pixels meet in device pixels.
fn spans(out: &mut Vec<Span>, scale: f32, areas: &[Rounded]) {
    let device = |v: f32| (v * scale).round() as i32;
    for a in areas {
        let (x, y) = (device(a.x), device(a.y));
        let (w, h) = (device(a.x + a.w) - x, device(a.y + a.h) - y);
        rounded(out, x, y, w, h, device(a.r));
    }
}

/// `span` as an X rectangle, clamped to the protocol's field ranges.
fn rectangle([x, y, w, h]: Span) -> Rectangle {
    let coord = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    let extent = |v: i32| v.clamp(0, i32::from(u16::MAX)) as u16;
    Rectangle {
        x: coord(x),
        y: coord(y),
        width: extent(w),
        height: extent(h),
    }
}

/// Append to `out` the rectangles covering a `w`x`h` rect at (`x`, `y`)
/// whose corners have radius `r`, device pixels. A pixel is covered when
/// its centre lies inside the rounded rect. Corner rows with the same
/// inset share one rectangle, and no two rectangles overlap. The radius
/// is at most half the shorter side, as a painted corner is.
fn rounded(out: &mut Vec<Span>, x: i32, y: i32, w: i32, h: i32, r: i32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let r = r.clamp(0, w.min(h) / 2);
    // The first covered column of corner row `j`, counted from the
    // rect's edge: the pixel centres inside the corner's circle.
    let inset = |j: i32| {
        let rf = r as f32;
        let dy = rf - (j as f32 + 0.5);
        let dx = (rf * rf - dy * dy).max(0.0).sqrt();
        ((rf - 0.5 - dx).ceil() as i32).max(0)
    };
    let mut j = 0;
    while j < r {
        let i = inset(j);
        let mut k = j + 1;
        while k < r && inset(k) == i {
            k += 1;
        }
        if w > 2 * i {
            out.push([x + i, y + j, w - 2 * i, k - j]);
            out.push([x + i, y + h - k, w - 2 * i, k - j]);
        }
        j = k;
    }
    if h > 2 * r {
        out.push([x, y + r, w, h - 2 * r]);
    }
}
