//! X11 per-window recording: click-to-pick a window, then record ONLY that
//! window via its XComposite backing pixmap. Occluding windows, the
//! desktop, and other applications never enter the frame — the pixmap holds
//! the window's own pixels.
//!
//! While recording, the target window wears a thin red override-redirect
//! border with an empty input shape (clicks pass straight through) plus a
//! display-only timer chip — recording state is visible on the window
//! itself rather than in a floating panel.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as CompositeExt, Redirect};
use x11rb::protocol::shape::SK as ShapeKind;
use x11rb::protocol::xfixes::ConnectionExt as XfixesExt;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt as XprotoExt, CreateWindowAux, Cursor, EventMask, Font,
    GrabMode, GrabStatus, ImageFormat, StackMode, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

use super::encoder::{Encoder, EncoderConfig};
use super::{unique_recording_path, RecordingSpec, CANCELLED_PREFIX};

const XC_CROSSHAIR: u16 = 34;
const KEYSYM_ESCAPE: u32 = 0xff1b;
const BORDER_COLOR: u32 = 0x00f7768e;
const BORDER_THICKNESS: i16 = 3;

/// Picked target: window id plus its geometry.
pub struct PickedWindow {
    pub id: Window,
    pub width: u32,
    pub height: u32,
}

/// Outer geometry of a window in root coordinates.
#[derive(Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
}

/// Follow target for the recording chip: the border thread reports the
/// target window's rect, and the chip's owner repositions (and at the
/// end removes) the chip window. The GPUI daemon implements this over
/// its native chip window.
pub trait ChipFollow: Send + Sync {
    fn place(&self, rect: Rect);
    fn hide(&self);
}

fn connect() -> Result<(RustConnection, usize), String> {
    x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))
}

fn root_rect(conn: &RustConnection, root: Window, win: Window) -> Result<Rect, String> {
    let geom = conn
        .get_geometry(win)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("window gone: {e}"))?;
    let translated = conn
        .translate_coordinates(win, root, 0, 0)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("translate_coordinates: {e}"))?;
    Ok(Rect {
        x: translated.dst_x,
        y: translated.dst_y,
        w: geom.width,
        h: geom.height,
    })
}

fn make_crosshair(conn: &RustConnection) -> Result<Cursor, String> {
    let font: Font = conn.generate_id().map_err(|e| e.to_string())?;
    conn.open_font(font, b"cursor")
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("open cursor font: {e}"))?;
    let cursor = conn.generate_id().map_err(|e| e.to_string())?;
    conn.create_glyph_cursor(
        cursor, font, font, XC_CROSSHAIR, XC_CROSSHAIR + 1,
        0, 0, 0, u16::MAX, u16::MAX, u16::MAX,
    )
    .map_err(|e| e.to_string())?
    .check()
    .map_err(|e| format!("create crosshair cursor: {e}"))?;
    conn.close_font(font).map_err(|e| e.to_string())?;
    Ok(cursor)
}

/// Keycodes that map to Escape on this server.
fn escape_keycodes(conn: &RustConnection) -> Result<Vec<u8>, String> {
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

/// Click-to-pick: grab pointer and keyboard, wait for a click (target) or
/// Escape (cancel). Returns the top-level window under the click.
pub fn pick_window() -> Result<PickedWindow, String> {
    let (conn, screen_num) = connect()?;
    let root = conn.setup().roots[screen_num].root;
    let cursor = make_crosshair(&conn)?;
    let escapes = escape_keycodes(&conn)?;

    let status = conn
        .grab_pointer(
            false,
            root,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            root,
            cursor,
            CURRENT_TIME,
        )
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("grab_pointer: {e}"))?;
    if status.status != GrabStatus::SUCCESS {
        return Err(format!("pointer grab failed: {:?}", status.status));
    }
    let _kb = conn
        .grab_keyboard(false, root, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)
        .map_err(|e| e.to_string())?
        .reply();

    let picked = loop {
        let event = conn.wait_for_event().map_err(|e| format!("wait_for_event: {e}"))?;
        match event {
            Event::ButtonPress(_) => {
                let ptr = conn
                    .query_pointer(root)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| format!("query_pointer: {e}"))?;
                let child = ptr.child;
                if child == x11rb::NONE || child == root {
                    break Err(format!(
                        "{CANCELLED_PREFIX} clicked the desktop, not a window"
                    ));
                }
                // Walk up to the top-level ancestor (direct child of root).
                let mut win = child;
                loop {
                    let tree = conn
                        .query_tree(win)
                        .map_err(|e| e.to_string())?
                        .reply()
                        .map_err(|e| format!("query_tree: {e}"))?;
                    if tree.parent == root || tree.parent == x11rb::NONE {
                        break;
                    }
                    win = tree.parent;
                }
                let geom = conn
                    .get_geometry(win)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| format!("get_geometry: {e}"))?;
                break Ok(PickedWindow {
                    id: win,
                    width: u32::from(geom.width),
                    height: u32::from(geom.height),
                });
            }
            Event::KeyPress(ev) if escapes.contains(&ev.detail) => {
                break Err(format!("{CANCELLED_PREFIX} pick aborted"));
            }
            _ => {}
        }
    };

    let _ = conn.ungrab_pointer(CURRENT_TIME);
    let _ = conn.ungrab_keyboard(CURRENT_TIME);
    let _ = conn.free_cursor(cursor);
    picked
}

fn bgrx_to_rgba(data: &[u8], pixels: usize) -> Result<Vec<u8>, String> {
    let bpp = data.len() / pixels.max(1);
    let mut rgba = Vec::with_capacity(pixels * 4);
    match bpp {
        4 => {
            for px in data.chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        3 => {
            for px in data.chunks_exact(3) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        other => return Err(format!("unsupported bytes-per-pixel {other}")),
    }
    Ok(rgba)
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
}

impl Drop for RecordingMark {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn border_thread(chip: Arc<dyn ChipFollow>, target: Window, stop: std::sync::mpsc::Receiver<()>) {
    let Ok((conn, screen_num)) = connect() else {
        return;
    };
    let root = conn.setup().roots[screen_num].root;

    // Four override-redirect strips with an empty input shape.
    let mut strips = [0u32; 4];
    for strip in &mut strips {
        let Ok(win) = conn.generate_id() else { return };
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
            return;
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

    let mut last: Option<Rect> = None;
    loop {
        if stop.try_recv().is_ok() {
            break;
        }
        match root_rect(&conn, root, target) {
            Ok(rect) => {
                if last != Some(rect) {
                    place_strips(&conn, &strips, rect);
                    last = Some(rect);
                }
                // Every poll: the WM may place the chip on first map,
                // after the initial rect report.
                chip.place(rect);
            }
            Err(_) => break, // target closed; recording ends on its own
        }
        thread::sleep(Duration::from_millis(200));
    }

    for strip in strips {
        let _ = conn.destroy_window(strip);
    }
    let _ = conn.flush();
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

// ---------- recording ----------

/// Record the picked window until `spec.stop` fires, with the chip owned
/// by the caller through `chip`. This is the toolkit-independent entry
/// point used by the GPUI daemon.
pub fn record_window_follow(spec: RecordingSpec, chip: Arc<dyn ChipFollow>) -> Result<(), String> {
    let chip_inner = chip.clone();
    let result = (move || {
        let picked = pick_window()?;
        let _mark = RecordingMark::show(chip_inner, picked.id);
        record_target(&picked, &spec)
    })();
    // On every exit — cancel, error, or clean stop — the chip goes away.
    chip.hide();
    result
}

fn record_target(picked: &PickedWindow, spec: &RecordingSpec) -> Result<(), String> {
    let (conn, _) = connect()?;

    // Composite 0.2+ for window redirection + named pixmaps.
    let version = conn
        .composite_query_version(0, 4)
        .map_err(|e| format!("composite extension missing: {e}"))?
        .reply()
        .map_err(|e| format!("composite_query_version: {e}"))?;
    if version.major_version == 0 && version.minor_version < 2 {
        return Err(format!(
            "composite extension too old: {}.{}",
            version.major_version, version.minor_version
        ));
    }

    conn.composite_redirect_window(picked.id, Redirect::AUTOMATIC)
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("redirect window: {e}"))?;

    let result = record_loop(&conn, picked, spec);

    let _ = conn.composite_unredirect_window(picked.id, Redirect::AUTOMATIC);
    result
}

fn grab_pixmap(
    conn: &RustConnection,
    win: Window,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let pixmap = conn.generate_id().map_err(|e| e.to_string())?;
    conn.composite_name_window_pixmap(win, pixmap)
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("name_window_pixmap (window closed?): {e}"))?;
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            pixmap,
            0,
            0,
            width as u16,
            height as u16,
            !0u32,
        )
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| format!("get_image on window pixmap: {e}"))?;
    conn.free_pixmap(pixmap).map_err(|e| e.to_string())?;
    bgrx_to_rgba(&image.data, width as usize * height as usize)
}

fn record_loop(
    conn: &RustConnection,
    picked: &PickedWindow,
    spec: &RecordingSpec,
) -> Result<(), String> {
    let mut width = picked.width;
    let mut height = picked.height;
    let mut output: PathBuf = spec.output.clone();
    let mut encoder = Encoder::start(&EncoderConfig {
        output: output.clone(),
        width,
        height,
        fps: spec.fps,
        mic: spec.mic,
    })?;

    let frame_interval = Duration::from_secs_f64(1.0 / f64::from(spec.fps));
    let start = Instant::now();
    let mut frame_no: u64 = 0;

    loop {
        if spec.stop.try_recv().is_ok() {
            encoder.finish()?;
            return Ok(());
        }

        // Detect resize / close each frame. A vanished window is a clean
        // end of the recording, not an error.
        let geom = match conn.get_geometry(picked.id) {
            Ok(cookie) => match cookie.reply() {
                Ok(g) => g,
                Err(_) => {
                    encoder.finish()?;
                    return Ok(());
                }
            },
            Err(_) => {
                encoder.finish()?;
                return Ok(());
            }
        };
        let (w, h) = (u32::from(geom.width), u32::from(geom.height));
        if w != width || h != height {
            encoder.finish()?;
            width = w;
            height = h;
            let dir = output
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            output = unique_recording_path(&dir);
            encoder = Encoder::start(&EncoderConfig {
                output: output.clone(),
                width,
                height,
                fps: spec.fps,
                mic: spec.mic,
            })?;
        }

        // Absolute schedule: frame n is due at start + n/fps. On overrun,
        // skip the counter forward (drop) instead of bursting.
        let due = start + frame_interval * frame_no as u32;
        let now = Instant::now();
        if now < due {
            thread::sleep(due - now);
        } else if now - due > frame_interval * 2 {
            frame_no = ((now - start).as_secs_f64() * f64::from(spec.fps)) as u64;
        }

        let rgba = match grab_pixmap(conn, picked.id, width, height) {
            Ok(rgba) => rgba,
            Err(_) => {
                // Window closed mid-grab: keep what we have.
                encoder.finish()?;
                return Ok(());
            }
        };
        encoder.write_frame(&rgba)?;
        frame_no += 1;
    }
}
