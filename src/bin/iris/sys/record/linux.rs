//! Linux recording: the Wayland portal picks a window; on X11 the
//! window or region is read from the root and the chip window follows
//! it by XID.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use futures::channel::oneshot;
use gpui::App;
use iris_lib::capture::WinRect;
use iris_lib::record::{self, ActiveRecording};

use super::Params;
use crate::chip;

/// Region recording reads the root window through X11 SHM; the portal
/// cannot name a rect.
pub(super) fn region_available() -> Result<(), String> {
    if iris_lib::session::wayland() {
        return Err("region recording needs X11; on Wayland record a window".to_string());
    }
    Ok(())
}

/// On X11 the chip opens once the pick lands, over the picked window:
/// none shows while the pick waits for a click, and its clock starts
/// with the recording.
pub(super) fn start_window(cx: &mut App, p: Params) -> Result<ActiveRecording, String> {
    if iris_lib::session::wayland() {
        return p.spawn(record::wayland::record_window);
    }
    let monitors = iris_lib::capture::monitors().unwrap_or_default();
    let (picked, landed) = oneshot::channel();
    let follower = XcbChip::new(
        0,
        monitors.clone(),
        crate::sys::window::root_scale(cx),
        Some(picked),
    );
    let chip = follower.clone();
    let output = p.output.clone();
    let rec = p.spawn(move |spec| record::x11::record_window_follow(spec, follower))?;
    // A source that ends before a pick lands drops the sender unsent.
    cx.spawn(async move |cx| {
        if let Ok(rect) = landed.await {
            let _ = cx.update(|cx| open_chip_over(cx, &output, rect, &monitors, &chip));
        }
    })
    .detach();
    Ok(rec)
}

/// Open the chip over the window a pick landed on (`rect`, root
/// pixels), in the recording's state now, and hand its window to
/// `follower`: the pause and mic hotkeys work during the pick. Only the
/// recording writing `output` gets the chip, and only while it runs:
/// one stopped or ended since, or replaced by a newer one, gets none.
fn open_chip_over(
    cx: &mut App,
    output: &Path,
    rect: WinRect,
    monitors: &[WinRect],
    follower: &XcbChip,
) {
    let (mic, paused) = match crate::daemon::RECORDING.lock().as_ref() {
        Some(rec) if rec.output == output && !rec.ended() => (rec.mic(), rec.paused()),
        _ => return,
    };
    match chip::open(cx, mic, Some(rect), monitors) {
        Ok(xid) => {
            if paused {
                chip::set_paused(cx, true);
            }
            follower.xid.store(xid, Ordering::Release);
        }
        Err(e) => crate::notice::failed(cx, "Recording chip did not open", &e),
    }
}

pub(super) fn start_region(
    cx: &mut App,
    p: Params,
    rect: WinRect,
) -> Result<ActiveRecording, String> {
    region_available()?;
    let root = record::x11::Rect {
        x: i16::try_from(rect.x).map_err(|_| format!("region x {} out of range", rect.x))?,
        y: i16::try_from(rect.y).map_err(|_| format!("region y {} out of range", rect.y))?,
        w: u16::try_from(rect.width)
            .map_err(|_| format!("region width {} out of range", rect.width))?,
        h: u16::try_from(rect.height)
            .map_err(|_| format!("region height {} out of range", rect.height))?,
    };
    let monitors = iris_lib::capture::monitors().unwrap_or_default();
    let xid = chip::open(cx, p.chip_mic(), Some(rect), &monitors)?;
    let follower = XcbChip::new(xid, monitors, crate::sys::window::root_scale(cx), None);
    p.spawn(move |spec| record::x11::record_region(spec, follower, root))
}

/// ChipFollow that moves the GPUI chip window by XID, with no app round
/// trip per move. A window recording opens the chip once the pick
/// lands; until then the XID is 0 and a move places nothing. The
/// monitors and the root scale are read once, at the start, on the UI
/// thread: every move places the chip on the monitor of the target's
/// current rect.
struct XcbChip {
    xid: AtomicU32,
    conn: Option<&'static x11rb::rust_connection::RustConnection>,
    monitors: Vec<WinRect>,
    /// Root pixels per logical pixel (`sys::window::root_scale`).
    scale: f32,
    /// Until the first place when the chip is not open yet: that place
    /// sends the target's rect here, and the chip opens over it.
    pick: parking_lot::Mutex<Option<oneshot::Sender<WinRect>>>,
}

impl XcbChip {
    fn new(
        xid: u32,
        monitors: Vec<WinRect>,
        scale: f32,
        pick: Option<oneshot::Sender<WinRect>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            xid: AtomicU32::new(xid),
            conn: iris_lib::capture::x11::shared_conn().ok().map(|(c, _)| c),
            monitors,
            scale,
            pick: parking_lot::Mutex::new(pick),
        })
    }
}

impl record::x11::ChipFollow for XcbChip {
    fn place(&self, rect: record::x11::Rect) {
        let rect = WinRect {
            x: rect.x.into(),
            y: rect.y.into(),
            width: rect.w.into(),
            height: rect.h.into(),
        };
        if let Some(pick) = self.pick.lock().take() {
            let _ = pick.send(rect);
            return;
        }
        let Some(conn) = &self.conn else { return };
        let xid = self.xid.load(Ordering::Acquire);
        if xid == 0 {
            return;
        }
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt};
        let s = self.scale;
        let (x, y) = chip::origin(Some(rect), &self.monitors, s);
        let (x, y) = ((x * s).round() as i32, (y * s).round() as i32);
        let _ = conn.configure_window(xid, &ConfigureWindowAux::new().x(x).y(y));
        let _ = conn.flush();
    }
}
