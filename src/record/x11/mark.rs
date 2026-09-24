use std::os::fd::AsFd;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::shape::SK as ShapeKind;
use x11rb::protocol::xfixes::ConnectionExt as XfixesExt;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt as XprotoExt, CreateWindowAux, Cursor, EventMask, Font,
    StackMode, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use super::{ChipFollow, Rect};
use crate::record::wake::Wake;
use crate::record::Doorbell;

const XC_CROSSHAIR: u16 = 34;
const KEYSYM_ESCAPE: u32 = 0xff1b;
const BORDER_COLOR: u32 = 0x00f7768e;
const BORDER_THICKNESS: i16 = 3;

/// Rect re-checks after the border first shows, `SETTLE_STEP` apart:
/// the WM's map-time re-framing lands without a notify to trust.
const SETTLE_PASSES: u32 = 10;
const SETTLE_STEP: Duration = Duration::from_millis(200);

pub(super) fn connect() -> Result<(RustConnection, usize), String> {
    x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))
}

fn root_rect(conn: &RustConnection, root: Window, win: Window) -> Result<Rect, String> {
    // The two requests are independent: send both before awaiting
    // either reply, halving the round trips on every border wake.
    let geom_cookie = conn.get_geometry(win).map_err(|e| e.to_string())?;
    let trans_cookie = conn
        .translate_coordinates(win, root, 0, 0)
        .map_err(|e| e.to_string())?;
    let geom = geom_cookie
        .reply()
        .map_err(|e| format!("window gone: {e}"))?;
    let translated = trans_cookie
        .reply()
        .map_err(|e| format!("translate_coordinates: {e}"))?;
    Ok(Rect {
        x: translated.dst_x,
        y: translated.dst_y,
        w: geom.width,
        h: geom.height,
    })
}

pub(super) fn make_crosshair(conn: &RustConnection) -> Result<Cursor, String> {
    let font: Font = conn.generate_id().map_err(|e| e.to_string())?;
    let cursor = conn.generate_id().map_err(|e| e.to_string())?;
    // X11 requests run in order, so the font open, cursor create and
    // font close can all be in flight together; checking each reply
    // serially was three round trips for one cursor.
    let open = conn.open_font(font, b"cursor").map_err(|e| e.to_string())?;
    let create = conn
        .create_glyph_cursor(
            cursor,
            font,
            font,
            XC_CROSSHAIR,
            XC_CROSSHAIR + 1,
            0,
            0,
            0,
            u16::MAX,
            u16::MAX,
            u16::MAX,
        )
        .map_err(|e| e.to_string())?;
    let close = conn.close_font(font).map_err(|e| e.to_string())?;
    open.check().map_err(|e| format!("open cursor font: {e}"))?;
    create
        .check()
        .map_err(|e| format!("create crosshair cursor: {e}"))?;
    close
        .check()
        .map_err(|e| format!("close cursor font: {e}"))?;
    Ok(cursor)
}

/// Keycodes that map to Escape on this server.
pub(super) fn escape_keycodes(conn: &RustConnection) -> Result<Vec<u8>, String> {
    let setup = conn.setup();
    let (lo, hi) = (setup.min_keycode, setup.max_keycode);
    let reply = conn
        .get_keyboard_mapping(lo, hi - lo + 1)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("get_keyboard_mapping: {e}"))?;
    let per = reply.keysyms_per_keycode as usize;
    let mut codes = Vec::new();
    for (i, syms) in reply.keysyms.chunks(per).enumerate() {
        if syms.contains(&KEYSYM_ESCAPE) {
            codes.push(lo + i as u8);
        }
    }
    Ok(codes)
}

// ---------- recording mark: border strips + timer chip ----------

/// Visual "this window is being recorded" state. Dropping it removes the
/// border; the chip window is closed separately via hide_chip().
pub struct RecordingMark {
    stop: Sender<()>,
    /// Rung after the stop: the follow thread sleeps until X input or
    /// a ring. The static thread installs no ring.
    bell: Doorbell,
    join: Option<JoinHandle<()>>,
}

impl RecordingMark {
    /// Draw the border around `target` and keep it following the window
    /// (moves, resizes) until dropped or the window closes.
    pub fn show(chip: Arc<dyn ChipFollow>, target: Window) -> Result<Self, String> {
        let (tx, rx) = channel::<()>();
        let bell = Doorbell::default();
        let wake = Wake::install(&bell)?;
        let join = thread::spawn(move || border_thread(chip, target, rx, wake));
        Ok(Self {
            stop: tx,
            bell,
            join: Some(join),
        })
    }

    /// Draw the border around a fixed rect once; no follow thread, so
    /// the mark lives until dropped. The chip is placed once at the
    /// rect's top-right corner.
    pub fn show_static(chip: Arc<dyn ChipFollow>, root: Window, rect: Rect) -> Self {
        let (tx, rx) = channel::<()>();
        let join = thread::spawn(move || static_border_thread(chip, root, rect, rx));
        Self {
            stop: tx,
            bell: Doorbell::default(),
            join: Some(join),
        }
    }
}

impl Drop for RecordingMark {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        self.bell.ring();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Create the four override-redirect border strips on `root`. Returns
/// the strip windows; the caller destroys them.
fn make_strips(conn: &RustConnection, root: Window) -> Option<[Window; 4]> {
    let mut strips = [0u32; 4];
    for (made, strip) in strips.iter_mut().enumerate() {
        let Some(win) = conn.generate_id().ok() else {
            destroy_strips(conn, &strips[..made]);
            return None;
        };
        let aux = CreateWindowAux::new()
            .background_pixel(BORDER_COLOR)
            .override_redirect(1);
        if conn
            .create_window(
                x11rb::COPY_FROM_PARENT as u8,
                win,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &aux,
            )
            .is_err()
        {
            // A failed create leaves the earlier strips mapped:
            // destroy them rather than leaking four border windows.
            destroy_strips(conn, &strips[..made]);
            return None;
        }
        // Empty input region: clicks pass through to whatever is beneath.
        let _ = conn
            .xfixes_set_window_shape_region(win, ShapeKind::INPUT, 0, 0, x11rb::NONE)
            .map(|c| c.check());
        let _ = conn.map_window(win);
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        *strip = win;
    }
    let _ = conn.flush();
    Some(strips)
}

fn destroy_strips(conn: &RustConnection, strips: &[Window]) {
    for strip in strips {
        let _ = conn.destroy_window(*strip);
    }
    let _ = conn.flush();
}

/// Static variant: strips around a fixed rect, chip placed once, then
/// the thread sleeps until the mark drops.
fn static_border_thread(chip: Arc<dyn ChipFollow>, root: Window, rect: Rect, stop: Receiver<()>) {
    let Ok((conn, _)) = connect() else {
        return;
    };
    let Some(strips) = make_strips(&conn, root) else {
        return;
    };
    place_strips(&conn, &strips, rect);
    chip.place(rect);
    // A send or the sender's drop ends the wait.
    let _ = stop.recv();
    destroy_strips(&conn, &strips);
}

fn border_thread(chip: Arc<dyn ChipFollow>, target: Window, stop: Receiver<()>, wake: Wake) {
    let Ok((conn, screen_num)) = connect() else {
        return;
    };
    let root = conn.setup().roots[screen_num].root;

    let Some(strips) = make_strips(&conn, root) else {
        return;
    };

    // Follow the target by event, not by polling: StructureNotify on
    // the target delivers ConfigureNotify on every move/resize, and the
    // thread sleeps on the connection until one arrives.
    let _ = conn.change_window_attributes(
        target,
        &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
            .event_mask(EventMask::STRUCTURE_NOTIFY),
    );
    let _ = conn.flush();

    let mut last: Option<Rect> = None;
    let mut settle = 0u32;
    // The first pass queries the rect with no event.
    let mut woke = true;
    loop {
        // Drained before the stop is read: a stop after the read rings
        // again, and the next wait ends at once.
        wake.drain();
        if !matches!(stop.try_recv(), Err(TryRecvError::Empty)) {
            break;
        }
        let Some(events) = drain_events(&conn, target) else {
            break;
        };
        woke |= events;
        // The rect query is two round trips. It is needed on the first
        // pass, after every event (a ConfigureNotify means the window
        // moved), and on each settle pass. After that a window that
        // stays put costs no query and no wakeup.
        if woke || settle < SETTLE_PASSES {
            match root_rect(&conn, root, target) {
                Ok(rect) => {
                    let moved = last != Some(rect);
                    if moved {
                        place_strips(&conn, &strips, rect);
                        last = Some(rect);
                    }
                    // Re-assert the chip through the settle passes: the
                    // WM can re-place it on map-time re-framing after
                    // the initial rect report. After that, only a real
                    // move re-places.
                    if moved || settle < SETTLE_PASSES {
                        chip.place(rect);
                    }
                }
                Err(_) => break, // target closed; recording ends on its own
            }
            settle += 1;
        }
        // The query's round trips can read events into the connection's
        // buffer, where they would not wake the poll.
        let Some(events) = drain_events(&conn, target) else {
            break;
        };
        woke = events;
        if !woke {
            let settling = (settle < SETTLE_PASSES).then_some(SETTLE_STEP);
            wake.wait(Some(conn.stream().as_fd()), settling);
        }
    }

    destroy_strips(&conn, &strips);
}

/// Read every queued event: whether any arrived, or None once the
/// target is destroyed.
fn drain_events(conn: &RustConnection, target: Window) -> Option<bool> {
    let mut any = false;
    while let Ok(Some(event)) = conn.poll_for_event() {
        any = true;
        if matches!(event, Event::DestroyNotify(ev) if ev.window == target) {
            return None;
        }
    }
    Some(any)
}

fn place_strips(conn: &RustConnection, strips: &[Window; 4], r: Rect) {
    let t = BORDER_THICKNESS;
    // top, bottom: full width incl. corners; left, right: between them.
    let geoms = [
        (r.x - t, r.y - t, r.w as i16 + 2 * t, t),
        (r.x - t, r.y + r.h as i16, r.w as i16 + 2 * t, t),
        (r.x - t, r.y, t, r.h as i16),
        (r.x + r.w as i16, r.y, t, r.h as i16),
    ];
    for (win, (x, y, w, h)) in strips.iter().zip(geoms) {
        let _ = conn.configure_window(
            *win,
            &ConfigureWindowAux::new()
                .x(i32::from(x))
                .y(i32::from(y))
                .width(w.max(1) as u32)
                .height(h.max(1) as u32)
                .stack_mode(StackMode::ABOVE),
        );
    }
    let _ = conn.flush();
}
