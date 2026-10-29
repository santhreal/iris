//! Captured frames and screen rects, shared by every platform. The
//! grabs themselves are in [`crate::sys::capture`].

/// One grabbed frame: RGBA8, row-major, tightly packed, origin at the
/// top-left of the virtual screen.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Root-space rectangle of a top-level window, used by overlay hover-snap.
#[derive(Clone, Copy, serde::Serialize)]
pub struct WinRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WinRect {
    /// The monitor among `monitors` (primary first) that holds this
    /// rect's center, or the primary when none does.
    pub fn host<'a>(&self, monitors: &'a [WinRect]) -> Option<&'a WinRect> {
        let cx = i64::from(self.x) + i64::from(self.width) / 2;
        let cy = i64::from(self.y) + i64::from(self.height) / 2;
        monitors
            .iter()
            .find(|m| {
                let (x, y) = (i64::from(m.x), i64::from(m.y));
                (x..x + i64::from(m.width)).contains(&cx)
                    && (y..y + i64::from(m.height)).contains(&cy)
            })
            .or(monitors.first())
    }

    /// The smallest rect that holds every rect in `rects`: over the
    /// monitors, the virtual screen. None when `rects` is empty.
    pub fn union(rects: &[WinRect]) -> Option<WinRect> {
        rects.iter().copied().reduce(|u, m| {
            let x0 = u.x.min(m.x);
            let y0 = u.y.min(m.y);
            let x1 = (u.x + u.width as i32).max(m.x + m.width as i32);
            let y1 = (u.y + u.height as i32).max(m.y + m.height as i32);
            WinRect {
                x: x0,
                y: y0,
                width: (x1 - x0) as u32,
                height: (y1 - y0) as u32,
            }
        })
    }
}

pub trait CaptureBackend {
    /// Grab the whole virtual screen as one frame.
    fn grab_screen(&self) -> Result<Frame, String>;
}

// WHY: the class closed here is "a window placed for a rect lands on
// the wrong monitor": the recording chip picks the rect's monitor
// through `host`, and the overlay's shell and frozen frame both span
// `union`. Not covered: monitor enumeration itself, which the
// per-platform backends own.
#[cfg(test)]
mod tests {
    use super::WinRect;

    const fn rect(x: i32, y: i32, width: u32, height: u32) -> WinRect {
        WinRect {
            x,
            y,
            width,
            height,
        }
    }

    /// Primary 1000x1000 at the origin; a second monitor to its left at
    /// negative x, as Windows and macOS lay out a monitor placed there.
    const MONITORS: [WinRect; 2] = [rect(0, 0, 1000, 1000), rect(-800, 200, 800, 600)];

    fn host_x(r: WinRect, monitors: &[WinRect]) -> Option<i32> {
        r.host(monitors).map(|m| m.x)
    }

    #[test]
    fn host_is_the_monitor_holding_the_center() {
        assert_eq!(host_x(rect(-700, 300, 300, 200), &MONITORS), Some(-800));
        // Starts on the left monitor, centered on the primary: the
        // center decides, not the origin.
        assert_eq!(host_x(rect(-100, 300, 600, 200), &MONITORS), Some(0));
    }

    #[test]
    fn host_edges_are_half_open() {
        let side_by_side = [rect(0, 0, 1000, 1000), rect(1000, 0, 500, 500)];
        // Center at x=1000: past the primary's last column.
        assert_eq!(host_x(rect(999, 0, 2, 2), &side_by_side), Some(1000));
        assert_eq!(host_x(rect(998, 0, 2, 2), &side_by_side), Some(0));
    }

    #[test]
    fn host_falls_back_to_the_primary() {
        let nowhere = rect(-800, -500, 100, 100);
        assert_eq!(host_x(nowhere, &MONITORS), Some(0));
        assert_eq!(host_x(nowhere, &[]), None);
    }

    #[test]
    fn union_spans_every_monitor_from_the_top_left_one() {
        let span = |rects: &[WinRect]| WinRect::union(rects).map(|u| (u.x, u.y, u.width, u.height));
        assert_eq!(span(&MONITORS), Some((-800, 0, 1800, 1000)));
        assert_eq!(span(&MONITORS[..1]), Some((0, 0, 1000, 1000)));
        assert_eq!(
            span(&[rect(1920, 0, 1920, 1080)]),
            Some((1920, 0, 1920, 1080))
        );
        assert_eq!(span(&[]), None);
    }
}
