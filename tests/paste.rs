//! A paste into the editor's text entry, read from the saved file.
//!
//! WHY: the class closed here is "a text entry takes keys but not the
//! clipboard". The entry appended the character of each key press and
//! dropped Ctrl+V, so a label could only be typed. The case puts two
//! lines on the clipboard, opens a text entry, pastes, saves, and reads
//! one line of ink as wide as both lines' words in the file. The editor
//! and Settings edit their text through one function, whose unit tests
//! check the rules that make pasted text one line; `settings_paste.rs`
//! reads the Settings side.
//!
//! Not covered: Cmd+V on macOS and Ctrl+V on Windows, which reach the
//! same key handler; a clipboard that holds no text. The case runs only
//! with `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without them it
//! prints that it did not run.

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

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{chord, click, client, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The editor's title on `plain.png`.
const EDITOR: &str = "plain.png - iris";
/// The image's size and its one color, [r, g, b].
const SIZE: (u32, u32) = (600, 400);
const IMAGE: [u8; 3] = [60, 140, 200];
/// Keysyms.
const T: u32 = 0x74;
const V: u32 = 0x76;
const RETURN: u32 = 0xff0d;
const CONTROL: u32 = 0xffe3;
/// Where the label goes, in image pixels.
const LABEL_AT: (f32, f32) = (80.0, 200.0);
/// The clipboard: two lines, the second ending in a line break.
const CLIPBOARD: &str = "Launch day\nsecond line\n";
/// The label's text size on a 600 px image, `text_size`'s floor, and the
/// least width of "Launch day second line" in it: 22 characters of Inter
/// Bold at about 0.55 em each, less a third for slack.
const TEXT_SIZE: u32 = 14;
const LEAST_WIDTH: u32 = 22 * TEXT_SIZE * 55 / 100 * 2 / 3;

/// The marks' color, `#ff453a`, over the image.
fn red(p: [u8; 3]) -> bool {
    p[0] > IMAGE[0] + 80 && p[2] < IMAGE[2] - 60
}

#[test]
fn a_paste_into_a_text_entry_writes_the_clipboard_as_one_line() {
    let case = "a_paste_into_a_text_entry_writes_the_clipboard_as_one_line";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    // The clipboard is this process's, on the case's X server. This is
    // the file's one case, so no other case reads the environment while
    // it changes.
    std::env::set_var("DISPLAY", &display);
    std::env::remove_var("WAYLAND_DISPLAY");
    iris_lib::clipboard::set_text(CLIPBOARD).unwrap();
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.png");
    image::RgbImage::from_pixel(SIZE.0, SIZE.1, image::Rgb(IMAGE))
        .save(&file)
        .unwrap();
    let written = std::fs::metadata(&file).unwrap().modified().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&[OsStr::new("--annotate"), file.as_os_str()]);
    let editor = daemon.until("the editor to open", || {
        windows_of(&conn, root, EDITOR).first().copied()
    });
    let bgr = [IMAGE[2], IMAGE[1], IMAGE[0]];
    let (origin, scale) = daemon.until("the editor to draw the image", || {
        let (w, _, px) = client(&conn, root, editor);
        let (mut left, mut top, mut right) = (usize::MAX, usize::MAX, 0);
        for (i, _) in px.iter().enumerate().filter(|&(_, &p)| p == bgr) {
            left = left.min(i % w);
            top = top.min(i / w);
            right = right.max(i % w);
        }
        let scale = (right + 1).saturating_sub(left) as f32 / SIZE.0 as f32;
        (scale > 0.25).then_some(((left, top), scale))
    });
    daemon.settle();
    let at = conn
        .translate_coordinates(editor, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let canvas_at = |p: (f32, f32)| {
        (
            origin.0 + (p.0 * scale) as usize,
            origin.1 + (p.1 * scale) as usize,
        )
    };
    tap(&conn, root, T);
    daemon.settle();
    let (cx, cy) = canvas_at(LABEL_AT);
    click(&conn, root, (at.dst_x + cx as i16, at.dst_y + cy as i16));
    // A text entry repaints its caret while it is open, so the daemon
    // does not go idle until Return commits it: wait for the entry to
    // draw instead.
    daemon.until("the text entry to open", || {
        let (w, _, px) = client(&conn, root, editor);
        (cy - 40..cy + 20)
            .flat_map(|y| (cx - 10..cx + 40).map(move |x| (x, y)))
            .any(|(x, y)| px[y * w + x] != bgr)
            .then_some(())
    });
    chord(&conn, root, &[CONTROL, V]);
    tap(&conn, root, RETURN);
    daemon.settle();
    tap(&conn, root, RETURN);
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
    let ink: Vec<(u32, u32)> = saved
        .enumerate_pixels()
        .filter(|(_, _, p)| red(p.0))
        .map(|(x, y, _)| (x, y))
        .collect();
    let (left, right) = ink
        .iter()
        .fold((u32::MAX, 0), |(l, r), &(x, _)| (l.min(x), r.max(x)));
    let (top, bottom) = ink
        .iter()
        .fold((u32::MAX, 0), |(t, b), &(_, y)| (t.min(y), b.max(y)));
    let (width, height) = (
        (right + 1).saturating_sub(left),
        (bottom + 1).saturating_sub(top),
    );
    assert!(
        width >= LEAST_WIDTH && height <= TEXT_SIZE * 5 / 4,
        "the pasted label inks {width}x{height} px: one line is at least \
         {LEAST_WIDTH} px wide and at most {} px tall",
        TEXT_SIZE * 5 / 4
    );
}
