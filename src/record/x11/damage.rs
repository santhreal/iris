//! The XDamage subscription of a recording source. RAW_RECTANGLES
//! delivers one Notify per damaged region with its area, so the dirty
//! flag is exact: set by an event intersecting the record rect, cleared
//! when read. Without the extension every read reports a change, and
//! the loop grabs every frame.
//!
//! A paused recording releases the subscription: a busy source then
//! sends no events and wakes nothing. The first read after the pause
//! subscribes again and reports a change.

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::damage::{self, ConnectionExt as DamageExt, ReportLevel};
use x11rb::protocol::xproto::Drawable;
use x11rb::rust_connection::RustConnection;

use super::Rect;

pub(super) struct DamageWatch<'c> {
    conn: &'c RustConnection,
    drawable: Drawable,
    /// Damage outside this rect does not dirty the frame; None means
    /// the whole drawable counts (window recording).
    rect: Option<Rect>,
    /// The live subscription: None while paused, and for good once a
    /// subscribe failed.
    id: Option<damage::Damage>,
    failed: bool,
    dirty: bool,
}

impl<'c> DamageWatch<'c> {
    /// Subscribe to `drawable`. The first read reports a change.
    pub(super) fn new(conn: &'c RustConnection, drawable: Drawable, rect: Option<Rect>) -> Self {
        let mut watch = Self {
            conn,
            drawable,
            rect,
            id: None,
            failed: false,
            dirty: true,
        };
        watch.subscribe();
        watch
    }

    fn subscribe(&mut self) {
        self.id = create(self.conn, self.drawable);
        self.failed = self.id.is_none();
        self.dirty = true;
    }

    /// Fold one DamageNotify into the dirty flag. Called from the drain
    /// that tracks the source's other events, so each event is read
    /// once; one from a released subscription is ignored.
    pub(super) fn note(&mut self, ev: &damage::NotifyEvent) {
        if Some(ev.damage) != self.id {
            return;
        }
        if let Some(r) = self.rect {
            let a = &ev.area;
            let (ax, ay) = (i32::from(a.x), i32::from(a.y));
            let (aw, ah) = (i32::from(a.width), i32::from(a.height));
            let (rx, ry) = (i32::from(r.x), i32::from(r.y));
            let (rw, rh) = (i32::from(r.w), i32::from(r.h));
            let hit = ax < rx + rw && ax + aw > rx && ay < ry + rh && ay + ah > ry;
            if !hit {
                return;
            }
        }
        self.dirty = true;
    }

    /// True when the source changed since the last read. Paused, the
    /// subscription is released and the read is false. The subtract
    /// keeps the server-side region from growing without bound; it is
    /// hygiene, not correctness, since RAW_RECTANGLES events do not
    /// depend on the accumulated region.
    pub(super) fn take_dirty(&mut self, paused: bool) -> bool {
        if paused {
            self.release();
            return false;
        }
        if self.id.is_none() && !self.failed {
            self.subscribe();
        }
        if self.failed {
            return true;
        }
        let dirty = std::mem::take(&mut self.dirty);
        if let (true, Some(id)) = (dirty, self.id) {
            let _ = DamageExt::damage_subtract(self.conn, id, x11rb::NONE, x11rb::NONE);
            let _ = self.conn.flush();
        }
        dirty
    }

    fn release(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = DamageExt::damage_destroy(self.conn, id);
            let _ = self.conn.flush();
        }
    }
}

impl Drop for DamageWatch<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

/// A RAW_RECTANGLES damage object on `drawable`; None without the
/// extension or when the create fails.
fn create(conn: &RustConnection, drawable: Drawable) -> Option<damage::Damage> {
    conn.extension_information(damage::X11_EXTENSION_NAME)
        .ok()??;
    DamageExt::damage_query_version(conn, 1, 1)
        .ok()?
        .reply()
        .ok()?;
    let id = conn.generate_id().ok()?;
    DamageExt::damage_create(conn, id, drawable, ReportLevel::RAW_RECTANGLES)
        .ok()?
        .check()
        .ok()?;
    Some(id)
}
