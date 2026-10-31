//! A window's close control and the window manager's close, read from
//! what the window leaves on disk.
//!
//! WHY: the class closed here is "a close that skips the window's close
//! path". The close control and the window manager's close removed the
//! window outright, so an editor closed that way lost every mark, and a
//! Settings field closed that way lost its typed value, where Ctrl+W
//! saved the image and applied the value. Every close now runs one
//! handler per window. The cases close an editor holding a rectangle,
//! and Settings with a typed file name, each way, and read the saved
//! file and the config. A stand-in compositing manager runs, so iris
//! draws its own frame and close control.
//!
//! Not covered: the Library, which has nothing to finish and removes
//! the window; an editor whose image never decoded, which closes
//! without saving; the close button of a window manager that sends
//! something other than `WM_DELETE_WINDOW`; Windows and macOS. The cases
//! run only with `IRIS_X11_TEST_DISPLAY` set and need Xvfb; without them
//! they print that they did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::ffi::OsStr;
use std::path::Path;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{click, client, drag, extent, field_left_of, tap, windows_of, wm_close, Compositor};

const SCREEN: &str = "1280x1024x24";
const EDITOR: &str = "plain.png - iris";
const SETTINGS: &str = "Settings - iris";
/// The image's size and its one color, [r, g, b].
const SIZE: (u32, u32) = (600, 400);
const IMAGE: [u8; 3] = [60, 140, 200];
/// The close control's centre from the window's top-right corner,
/// pixels: with no desktop button layout to read, the last window
/// control, 24 px round, 12 px in, and centered in the 52 px toolbar.
const CLOSE: (usize, usize) = (12 + 12, 26);
/// Keysyms.
const R: u32 = 0x72;
const X: u32 = 0x78;
/// The rectangle's corners, in image pixels.
const RECT: [(f32, f32); 2] = [(100.0, 100.0), (200.0, 180.0)];
/// The Settings window's width, the card's inset from its right edge,
/// the File name row's center, and the file name field's width,
/// logical px (see `settings_paste.rs`).
const WIDTH: f32 = 600.0;
const INSET: f32 = 20.0;
const TEMPLATE_Y: f32 = 52.0 + 20.0 + 36.0 + 20.0 + 20.0 + 74.0 + 18.0;
const FIELD_W: f32 = 200.0;
/// The template before and after the typed `x`.
const BEFORE: &str = "{date}_{time}";
const AFTER: &str = "{date}_{time}x";

/// How a case closes its window.
#[derive(Clone, Copy)]
enum Close {
    /// A click on the toolbar's close control.
    Control,
    /// The window manager's close: `WM_DELETE_WINDOW`.
    Manager,
}

impl Close {
    fn run(self, conn: &RustConnection, root: u32, window: u32) {
        match self {
            Close::Control => {
                let (w, _, _) = client(conn, root, window);
                let at = conn
                    .translate_coordinates(window, root, 0, 0)
                    .unwrap()
                    .reply()
                    .unwrap();
                let center = (w - CLOSE.0, CLOSE.1);
                click(
                    conn,
                    root,
                    (at.dst_x + center.0 as i16, at.dst_y + center.1 as i16),
                );
            }
            Close::Manager => wm_close(conn, window),
        }
    }
}

/// A stand-in compositing manager on `screen`: with one running, iris
/// draws its own frame, close control included, as on a desktop with a
/// compositor (see `frame.rs`).
fn compositor(conn: &RustConnection, root: u32, screen: usize) -> Compositor {
    let selection = conn
        .intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    Compositor::start(conn, root, selection)
}

/// Open an editor on a one-color image, draw a rectangle, close it
/// `how`, and assert the saved file holds the rectangle.
fn editor_saves(case: &str, how: Close) {
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let _compositor = compositor(&conn, root, screen);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.png");
    image::RgbImage::from_pixel(SIZE.0, SIZE.1, image::Rgb(IMAGE))
        .save(&file)
        .unwrap();
    let written = std::fs::metadata(&file).unwrap().modified().unwrap();
    let daemon = start(dir.path(), &display);
    daemon.send(&[OsStr::new("--annotate"), file.as_os_str()]);
    let editor = daemon.until("the editor to open", || {
        windows_of(&conn, root, EDITOR).first().copied()
    });
    let (origin, scale) = daemon.until("the editor to draw the image", || {
        let (left, top, right, _) = extent(&client(&conn, root, editor), IMAGE)?;
        let scale = (right + 1 - left) as f32 / SIZE.0 as f32;
        (scale > 0.25).then_some(((left, top), scale))
    });
    daemon.settle();
    let at = conn
        .translate_coordinates(editor, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let screen_at = |p: (f32, f32)| {
        (
            at.dst_x + (origin.0 as f32 + p.0 * scale) as i16,
            at.dst_y + (origin.1 as f32 + p.1 * scale) as i16,
        )
    };
    tap(&conn, root, R);
    daemon.settle();
    drag(&conn, root, screen_at(RECT[0]), screen_at(RECT[1]), 6);
    daemon.settle();
    how.run(&conn, root, editor);
    // The save lands after the window closes, over the original file.
    let saved = daemon.until("the editor to save and close", || {
        if !windows_of(&conn, root, EDITOR).is_empty() {
            return None;
        }
        let modified = std::fs::metadata(&file).ok()?.modified().ok()?;
        if modified == written {
            return None;
        }
        // A file still being written does not decode.
        Some(image::open(&file).ok()?.to_rgb8())
    });
    // The rectangle's left side, on the row halfway down it.
    let y = ((RECT[0].1 + RECT[1].1) / 2.0) as u32;
    let x = RECT[0].0 as u32;
    let red = (x - 2..=x + 2)
        .map(|x| saved.get_pixel(x, y).0)
        .any(|[r, g, b]| r > IMAGE[0] + 80 && b < IMAGE[2] - 60 && g < IMAGE[1]);
    assert!(red, "the saved file has the rectangle's left side at x {x}");
}

fn start(dir: &Path, display: &str) -> Daemon {
    Daemon::start(dir, |cmd| {
        cmd.env("DISPLAY", display).env_remove("WAYLAND_DISPLAY");
    })
}

/// The file name template in `dir`'s config, or `None` while the config
/// cannot be read whole, as mid-write.
fn template(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("config").join("config.toml")).ok()?;
    let table: toml::Table = text.parse().ok()?;
    Some(match table.get("screenshot_template") {
        Some(v) => v.as_str()?.to_string(),
        None => iris_lib::config::Config::default().screenshot_template,
    })
}

/// The accent of a focus ring, as [b, g, r].
fn accent(p: [u8; 3]) -> bool {
    p[0] > 200 && p[2] < 60
}

/// Open Settings, type `x` at the end of the file name field, close the
/// window `how`, and assert the config holds the typed name.
fn settings_applies(case: &str, how: Close) {
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let _compositor = compositor(&conn, root, screen);
    let dir = tempfile::tempdir().unwrap();
    let daemon = start(dir.path(), &display);
    assert_eq!(template(dir.path()).as_deref(), Some(BEFORE));
    daemon.send(&["--settings"]);
    let settings = daemon.until("Settings to open", || {
        windows_of(&conn, root, SETTINGS).first().copied()
    });
    daemon.settle();
    // The field: the text field left of the card's right padding on the
    // row's center line, once Settings has drawn it. The file name
    // preview sits left of it; its ink moves with the clock, so the
    // card's fill is read in the padding.
    let (scale, y, left, right) = daemon.until("the File name field to draw", || {
        let (w, _, px) = client(&conn, root, settings);
        let scale = w as f32 / WIDTH;
        let y = (TEMPLATE_Y * scale) as usize;
        let row = &px[y * w..(y + 1) * w];
        let card = row[w - ((INSET + 6.0) * scale) as usize];
        let before = w - (INSET * scale) as usize - 1;
        let (left, right) = field_left_of(row, card, before, (FIELD_W * scale) as usize)?;
        Some((scale, y, left, right))
    });
    let at = conn
        .translate_coordinates(settings, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    click(
        &conn,
        root,
        (at.dst_x + ((left + right) / 2) as i16, at.dst_y + y as i16),
    );
    // The open field draws an accent border and focus ring.
    let ring = (y - (20.0 * scale) as usize, y + (20.0 * scale) as usize);
    daemon.until("the field to open for typing", || {
        let (w, _, px) = client(&conn, root, settings);
        (ring.0..ring.1)
            .flat_map(|y| (left..=right).map(move |x| (x, y)))
            .any(|(x, y)| accent(px[y * w + x]))
            .then_some(())
    });
    tap(&conn, root, X);
    daemon.settle();
    assert_eq!(
        template(dir.path()).as_deref(),
        Some(BEFORE),
        "a typed value waits for the field to close"
    );
    how.run(&conn, root, settings);
    daemon.until("Settings to close", || {
        windows_of(&conn, root, SETTINGS).is_empty().then_some(())
    });
    assert_eq!(
        template(dir.path()).as_deref(),
        Some(AFTER),
        "closing Settings dropped the typed file name"
    );
}

#[test]
fn the_close_control_saves_the_editor() {
    editor_saves("the_close_control_saves_the_editor", Close::Control);
}

#[test]
fn the_window_managers_close_saves_the_editor() {
    editor_saves("the_window_managers_close_saves_the_editor", Close::Manager);
}

#[test]
fn the_close_control_applies_a_settings_field() {
    settings_applies("the_close_control_applies_a_settings_field", Close::Control);
}

#[test]
fn the_window_managers_close_applies_a_settings_field() {
    settings_applies(
        "the_window_managers_close_applies_a_settings_field",
        Close::Manager,
    );
}
