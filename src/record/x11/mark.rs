use std::os::unix::io::AsRawFd;
use std::sync::mpsc::{channel, Receiver, Sender};
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

const XC_CROSSHAIR: u16 = 34;
const KEYSYM_ESCAPE: u32 = 0xff1b;
const BORDER_COLOR: u32 = 0x00f7768e;
const BORDER_THICKNESS: i16 = 3;

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
    join: Option<JoinHandle<()>>,
}

impl RecordingMark {
    /// Draw the border around `target` and keep it following the window
    /// (moves, resizes) until dropped or the window closes.
    pub fn show(chip: Arc<dyn ChipFollow>, target: Window) -> Result<Self, String> {
        let (tx, rx) = channel::<()>();
        let join = thread::spawn(move || border_thread(chip, target, rx));
        Ok(Self {
            stop: tx,
            join: Some(join),
        })
    }

    /// Draw the border around a fixed rect once; no follow thread, so
    /// the mark lives until dropped. The chip is placed once at the
    /// rect's top-right corner.
    pub fn show_static(
        chip: Arc<dyn ChipFollow>,
        root: Window,
        rect: Rect,
    ) -> Result<Self, String> {
        let (tx, rx) = channel::<()>();
        let join = thread::spawn(move || static_border_thread(chip, root, rect, rx));
        Ok(Self {
            stop: tx,
            join: Some(join),
        })
    }
}

impl Drop for RecordingMark {
    fn drop(&mut self) {
        let _ = self.stop.send(());
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
/// the thread only waits for stop.
fn static_border_thread(chip: Arc<dyn ChipFollow>, root: Window, rect: Rect, stop: Receiver<()>) {
    let Ok((conn, _)) = connect() else {
        return;
    };
    let Some(strips) = make_strips(&conn, root) else {
        return;
    };
    place_strips(&conn, &strips, rect);
    chip.place(rect);
    loop {
        // recv_timeout wakes the instant stop fires; a bare sleep
        // would leave the strips up for up to 200ms past it.
        if stop.recv_timeout(Duration::from_millis(200)).is_ok() {
            break;
        }
    }
    destroy_strips(&conn, &strips);
}

fn border_thread(chip: Arc<dyn ChipFollow>, target: Window, stop: Receiver<()>) {
    let Ok((conn, screen_num)) = connect() else {
        return;
    };
    let root = conn.setup().roots[screen_num].root;

    let Some(strips) = make_strips(&conn, root) else {
        return;
    };

    // Follow the target by event, not by polling: StructureNotify on
    // the target delivers ConfigureNotify on every move/resize, so the
    // 200ms get_geometry+translate round trips become a poll on the
    // connection's fd that wakes only when the window actually moves.
    let _ = conn.change_window_attributes(
        target,
        &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
            .event_mask(EventMask::STRUCTURE_NOTIFY),
    );
    let _ = conn.flush();
    let x_fd = conn.stream().as_raw_fd();

    let mut last: Option<Rect> = None;
    let mut settle = 0u32;
    loop {
        if stop.try_recv().is_ok() {
            break;
        }
        let mut gone = false;
        let mut woke = false;
        while let Ok(Some(event)) = conn.poll_for_event() {
            woke = true;
            if let Event::DestroyNotify(ev) = event {
                if ev.window == target {
                    gone = true;
                }
            }
        }
        if gone {
            break;
        }
        // The rect query is two round trips. It is needed on the first
        // pass (no rect yet), on every event wake (a ConfigureNotify
        // means the window moved), and through the ~2s settle window
        // (the WM's map-time re-framing lands without a notify we can
        // trust). After that a timeout wake means nothing moved, so
        // the query is pure waste for the recording's life.
        let need_rect = last.is_none() || woke || settle < 10;
        if need_rect {
            match root_rect(&conn, root, target) {
                Ok(rect) => {
                    let moved = last != Some(rect);
                    if moved {
                        place_strips(&conn, &strips, rect);
                        last = Some(rect);
                    }
                    // Re-assert the chip for the first ~2s: the WM can
                    // re-place it on map-time re-framing after the
                    // initial rect report. After that, only a real
                    // move re-places.
                    if moved || settle < 10 {
                        chip.place(rect);
                    }
                }
                Err(_) => break, // target closed; recording ends on its own
            }
            settle += 1;
        }
        // Sleep until the next X event or the 200ms re-check: a
        // ConfigureNotify wakes the poll instantly, so a moved window
        // re-borders in the same frame instead of up to 200ms late.
        let mut pfd = libc::pollfd {
            fd: x_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe {
            libc::poll(&mut pfd, 1, 200);
        }
    }

    destroy_strips(&conn, &strips);
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
