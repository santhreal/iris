//! Text and counter labels on the editor's canvas against the saved file.
//!
//! WHY: the class closed here is "the canvas sets type that the file does
//! not". The canvas lays text out with the UI's text system and the save
//! bakes it with its own rasterizer, each with its own idea of the face's
//! size, weight, and baseline, so a label could sit, size, or weigh
//! differently once saved than it did while editing. The case writes a
//! word and places a counter on an image of one color, reads both from
//! the screen, saves, and reads that the file has the same ink in the
//! same place.
//!
//! Not covered: Windows and macOS, whose text systems lay out the same
//! face; zoom other than the image's fit; weight, and the line box's
//! centering of ascent and descent, which each move an edge of the ink
//! by under the case's slack at this size. The case runs only with
//! `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without them it prints
//! that it did not run.

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
use x11::{click, client, extent, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The editor's title on `plain.png`.
const EDITOR: &str = "plain.png - iris";
/// The image's size and its one color, [r, g, b].
const SIZE: (u32, u32) = (600, 400);
const IMAGE: [u8; 3] = [60, 140, 200];
/// Keysyms: `t` selects text, `n` the counter, `Return` commits the
/// word and then saves.
const T: u32 = 0x74;
const N: u32 = 0x6e;
const RETURN: u32 = 0xff0d;
/// The word: an ascender, an x-height letter, and a descender.
const WORD: [u32; 3] = [0x68, 0x61, 0x67];
/// Where the word and the counter go, in image pixels.
const WORD_AT: (f32, f32) = (80.0, 200.0);
const COUNTER_AT: (f32, f32) = (450.0, 200.0);
/// How far an edge of the ink may move between the canvas and the file,
/// in image pixels: a pixel of rounding on each side.
const SLACK: f32 = 1.5;

/// Edges of ink, left, top, right, bottom, in image pixels.
type Bounds = [f32; 4];

/// A view of the image: the color at an image pixel.
trait View {
    fn at(&self, x: u32, y: u32) -> [u8; 3];
}

/// The saved file.
impl View for image::RgbImage {
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        self.get_pixel(x, y).0
    }
}

/// The canvas: the window's pixels, the image's origin in them, and how
/// many window pixels an image pixel spans.
struct Canvas {
    w: usize,
    px: Vec<[u8; 3]>,
    origin: (usize, usize),
    scale: f32,
}

impl View for Canvas {
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let wx = self.origin.0 + ((x as f32 + 0.5) * self.scale) as usize;
        let wy = self.origin.1 + ((y as f32 + 0.5) * self.scale) as usize;
        let [b, g, r] = self.px[wy * self.w + wx];
        [r, g, b]
    }
}

/// The bounds of the pixels in `columns` that `ink` picks, over the
/// image's height.
fn bounds(
    view: &impl View,
    columns: std::ops::Range<u32>,
    ink: impl Fn([u8; 3]) -> bool,
) -> Option<Bounds> {
    let mut found: Option<Bounds> = None;
    for y in 0..SIZE.1 {
        for x in columns.clone() {
            if ink(view.at(x, y)) {
                let (x, y) = (x as f32, y as f32);
                let b = found.get_or_insert([x, y, x + 1.0, y + 1.0]);
                *b = [
                    b[0].min(x),
                    b[1].min(y),
                    b[2].max(x + 1.0),
                    b[3].max(y + 1.0),
                ];
            }
        }
    }
    found
}

/// The marks' color, `#ff453a`, over the image.
fn red(p: [u8; 3]) -> bool {
    p[0] > IMAGE[0] + 80 && p[2] < IMAGE[2] - 60
}

/// A counter's label: light on the red disc.
fn label(p: [u8; 3]) -> bool {
    p[1] > 170 && p[0] > 200
}

/// The word's ink, and the counter's label relative to its disc's
/// centre, in image pixels.
fn marks(view: &impl View) -> Option<(Bounds, Bounds)> {
    let split = ((WORD_AT.0 + COUNTER_AT.0) / 2.0) as u32;
    let word = bounds(view, 0..split, red)?;
    let disc = bounds(view, split..SIZE.0, red)?;
    let (cx, cy) = ((disc[0] + disc[2]) / 2.0, (disc[1] + disc[3]) / 2.0);
    let label = bounds(view, split..SIZE.0, label)?;
    Some((
        word,
        [label[0] - cx, label[1] - cy, label[2] - cx, label[3] - cy],
    ))
}

fn near(a: Bounds, b: Bounds) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= SLACK)
}

#[test]
fn the_saved_file_sets_text_and_counters_as_the_canvas_does() {
    let case = "the_saved_file_sets_text_and_counters_as_the_canvas_does";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.png");
    image::RgbImage::from_pixel(SIZE.0, SIZE.1, image::Rgb(IMAGE))
        .save(&file)
        .unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
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
    tap(&conn, root, T);
    daemon.settle();
    click(&conn, root, screen_at(WORD_AT));
    // A text entry repaints its caret while it is open, so the daemon
    // does not go idle until Return commits it: wait for the entry to
    // draw instead.
    daemon.until("the text entry to open", || {
        let (w, _, px) = client(&conn, root, editor);
        let canvas = Canvas {
            w,
            px,
            origin,
            scale,
        };
        let (x, y) = (WORD_AT.0 as u32, WORD_AT.1 as u32);
        (y - 40..y + 20)
            .flat_map(|y| (x - 10..x + 40).map(move |x| (x, y)))
            .any(|(x, y)| canvas.at(x, y) != IMAGE)
            .then_some(())
    });
    for key in WORD {
        tap(&conn, root, key);
    }
    tap(&conn, root, RETURN);
    daemon.settle();
    tap(&conn, root, N);
    daemon.settle();
    click(&conn, root, screen_at(COUNTER_AT));
    daemon.settle();
    let canvas = daemon.until("the word and the counter to draw", || {
        let (w, _, px) = client(&conn, root, editor);
        marks(&Canvas {
            w,
            px,
            origin,
            scale,
        })
    });
    tap(&conn, root, RETURN);
    let saved = daemon.until("the editor to save and close", || {
        if !windows_of(&conn, root, EDITOR).is_empty() {
            return None;
        }
        marks(&image::open(&file).ok()?.to_rgb8())
    });
    assert!(
        near(canvas.0, saved.0),
        "the word's ink is at {:?} on the canvas and at {:?} in the file",
        canvas.0,
        saved.0
    );
    assert!(
        near(canvas.1, saved.1),
        "the counter's label is at {:?} from its centre on the canvas and at {:?} in the file",
        canvas.1,
        saved.1
    );
}
