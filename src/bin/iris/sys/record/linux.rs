//! Linux recording: the Wayland portal picks a window; on X11 the
//! window or region is read from the root and the chip window follows
//! it by XID.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use futures::channel::mpsc::UnboundedSender;
use gpui::App;
use iris_lib::capture::WinRect;
use iris_lib::record::{self, ActiveRecording};

use super::Params;
use crate::chip;
use crate::daemon::{command_tx, Command};

fn wayland_only() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("DISPLAY").is_none()
}

/// Region recording reads the root window through X11 SHM; the portal
/// cannot name a rect.
pub(super) fn region_available() -> Result<(), String> {
    if wayland_only() {
        return Err("region recording needs X11; on Wayland record a window".to_string());
    }
    Ok(())
}

pub(super) fn start_window(cx: &mut App, p: Params) -> Result<ActiveRecording, String> {
    if wayland_only() {
        return Ok(p.spawn(record::wayland::record_window));
    }
    let follower = XcbChip::open(cx, p.mic)?;
    Ok(p.spawn(move |spec| record::x11::record_window_follow(spec, follower)))
}

pub(super) fn start_region(
    cx: &mut App,
    p: Params,
    rect: WinRect,
) -> Result<ActiveRecording, String> {
    region_available()?;
    let rect = record::x11::Rect {
        x: i16::try_from(rect.x).map_err(|_| format!("region x {} out of range", rect.x))?,
        y: i16::try_from(rect.y).map_err(|_| format!("region y {} out of range", rect.y))?,
        w: u16::try_from(rect.width)
            .map_err(|_| format!("region width {} out of range", rect.width))?,
        h: u16::try_from(rect.height)
            .map_err(|_| format!("region height {} out of range", rect.height))?,
    };
    let follower = XcbChip::open(cx, p.mic)?;
    Ok(p.spawn(move |spec| record::x11::record_region(spec, follower, rect)))
}

/// ChipFollow that moves the GPUI chip window by XID (no app round-trip
/// per move) and asks the daemon to close it at the end. The XID is
/// resolved lazily: at window-open time the X window may not be in the
/// tree yet.
struct XcbChip {
    xid: AtomicU32,
    conn: Option<&'static x11rb::rust_connection::RustConnection>,
    done: Option<UnboundedSender<Command>>,
}

impl XcbChip {
    fn open(cx: &mut App, mic: bool) -> Result<Arc<Self>, String> {
        let xid = chip::open(cx, mic, None)?;
        Ok(Arc::new(Self {
            xid: AtomicU32::new(xid),
            conn: iris_lib::capture::x11::shared_conn().ok().map(|(c, _)| c),
            done: command_tx(),
        }))
    }

    fn resolve_xid(&self) -> u32 {
        let cached = self.xid.load(Ordering::SeqCst);
        if cached != 0 {
            return cached;
        }
        let Some(conn) = &self.conn else { return 0 };
        if let Some(xid) = crate::sys::window::chip_xid_on(*conn) {
            self.xid.store(xid, Ordering::SeqCst);
            return xid;
        }
        0
    }
}

impl record::x11::ChipFollow for XcbChip {
    fn place(&self, rect: record::x11::Rect) {
        let Some(conn) = &self.conn else { return };
        let xid = self.resolve_xid();
        if xid == 0 {
            return;
        }
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt};
        // Bleed-compensated: the pill sits 16px inside the window.
        let x = i32::from(rect.x) + i32::from(rect.w) - 148 - 16;
        let y = (i32::from(rect.y) - 44 - 16).max(0);
        let _ = conn.configure_window(xid, &ConfigureWindowAux::new().x(x).y(y));
        let _ = conn.flush();
    }
    fn hide(&self) {
        if let Some(tx) = &self.done {
            let _ = tx.unbounded_send(Command::ChipHide);
        }
    }
}
