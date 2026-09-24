//! The X11 present report: a window's first XDamage after a mark.
//!
//! GPUI paces X11 frames with a timer. A present reaches the X server
//! after the tick that issued it, a frame or more later for a new
//! window's first present, so a later frame tick is no evidence that
//! the window's pixels are on screen. The X server damages a window
//! when a present writes it, and a compositor repaints from the same
//! reports. A caller that uncovers the screen behind a new window, as
//! the capture overlay does over the landed toast, waits for the report.
//!
//! The mark is a round trip made before the frame draws. Damage the
//! server applied before it, such as the map's background fill, carries
//! an earlier sequence number than damage from any later present.

use std::os::fd::AsRawFd;
use std::sync::{mpsc, Arc};
use std::time::Instant;

use futures::channel::oneshot;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::damage::{self, ConnectionExt as _, ReportLevel};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

/// A damage object on one window, on a private connection, and the
/// thread that waits for its report.
pub struct PresentWatch {
    conn: Arc<RustConnection>,
    mark: mpsc::Sender<u64>,
    presented: oneshot::Receiver<()>,
}

impl PresentWatch {
    /// Create a damage object on window `xid` and start the thread that
    /// waits for its report until `until`. None when the server has no
    /// Damage extension or the thread does not start.
    pub fn open(xid: u32, until: Instant) -> Option<Self> {
        let (conn, _) = x11rb::connect(None).ok()?;
        conn.extension_information(damage::X11_EXTENSION_NAME)
            .ok()??;
        conn.damage_query_version(1, 1).ok()?.reply().ok()?;
        let damage = conn.generate_id().ok()?;
        conn.damage_create(damage, xid, ReportLevel::RAW_RECTANGLES)
            .ok()?;
        let conn = Arc::new(conn);
        let (mark, marked) = mpsc::channel();
        let (report, presented) = oneshot::channel();
        let waiter = Arc::clone(&conn);
        std::thread::Builder::new()
            .name("iris-present".into())
            .spawn(move || wait(&waiter, damage, &marked, until, report))
            .ok()?;
        Some(Self {
            conn,
            mark,
            presented,
        })
    }

    /// Set the mark and return the report. Call it before the frame to
    /// watch draws: the round trip returns once the server has processed
    /// the damage object and the mark, so damage from a present issued
    /// after this call carries the mark's sequence number or a later
    /// one. The report resolves when that frame, or a later one, writes
    /// the window, and is canceled at the deadline or on an X error.
    pub fn arm(self) -> Option<oneshot::Receiver<()>> {
        let sync = self.conn.get_input_focus().ok()?;
        let mark = sync.sequence_number();
        sync.reply().ok()?;
        self.mark.send(mark).ok()?;
        Some(self.presented)
    }
}

/// The waiting thread: take the mark, then send the report at the first
/// damage notification at or after it. Every return drops `report`,
/// which cancels an unsent report, and the last reference to the
/// connection, which closes it and frees the damage object.
fn wait(
    conn: &RustConnection,
    damage: damage::Damage,
    marked: &mpsc::Receiver<u64>,
    until: Instant,
    report: oneshot::Sender<()>,
) {
    let Ok(mark) = marked.recv_timeout(until.saturating_duration_since(Instant::now())) else {
        return;
    };
    let fd = conn.stream().as_raw_fd();
    loop {
        // The mark's round trip may have queued events; drain the queue
        // before polling the socket.
        loop {
            match conn.poll_for_event_with_sequence() {
                Ok(Some((Event::DamageNotify(ev), seq))) if ev.damage == damage && seq >= mark => {
                    let _ = report.send(());
                    return;
                }
                // The connection carries only this watch's requests: an
                // error on it means no report follows.
                Ok(Some((Event::Error(_), _))) | Err(_) => return,
                Ok(Some(_)) => {}
                Ok(None) => break,
            }
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // Rounded up: a sub-millisecond remainder waits one millisecond
        // instead of spinning.
        let ms = i32::try_from(left.as_micros().div_ceil(1000)).unwrap_or(i32::MAX);
        if unsafe { libc::poll(&mut pfd, 1, ms) } < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
        {
            return;
        }
    }
}
