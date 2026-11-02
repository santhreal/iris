//! Where and how large the editor draws the capture, and the frame of a
//! selected mark over it, read back from the screen.
//!
//! WHY: the classes closed here:
//!
//! - "the editor shows the capture anywhere but the rect it maps image
//!   points to". The stage's hairline was a border of the element that
//!   clips the image, so the image was laid out inside it: a pixel in
//!   from each side of that rect and two pixels short of its size, while
//!   marks, the crop, and the pointer used the whole rect. The image's
//!   corners were rounded at rest too, hiding corner pixels the saved
//!   file keeps. A capture larger than the stage, fit at a scale that is
//!   not a whole number, shows its colour over every pixel wholly inside
//!   the rect, the four corner pixels among them, and over no pixel
//!   wholly outside it.
//! - "a small capture opens magnified". The fit grew a capture smaller
//!   than the stage up to four times, filtered soft. A capture smaller
//!   than the stage opens at one screen pixel per image pixel.
//! - "a selected mark shows no frame on a light capture". The frame was
//!   a white line at 68% against the mark's stroke, so on a white
//!   capture nothing showed the selection and on a coloured one it read
//!   as a halo of the stroke. The cases select a rectangle on a white,
//!   a black, and a mid-blue capture and read, left of it and above it,
//!   the capture's colour for 3 px, then a white line and a dark rim.
//!
//! Not covered: a zoomed or panned image, whose rect moves by the same
//! view; the morph from the toast, whose rect `tests/morph.rs` does not
//! read; a scale factor other than 1, which Xvfb does not have; the
//! frame of a label, a counter, or a mark against the image's edge,
//! which takes the same bounds; Wayland, Windows, and macOS, which
//! share the stage's layout. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need Xvfb; without them they print
//! that they did not run.

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
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, xvfb_sized, Daemon, Server};
use x11::{click, client, drag, extent, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The editor's title on `plain.png`.
const EDITOR: &str = "plain.png - iris";
/// The image's color, [r, g, b].
const IMAGE: [u8; 3] = [60, 140, 200];
/// The editor's toolbar and tool options bar, which the stage sits
/// under (`theme::TOOLBAR_H` plus `OPTIONS_H`), and the room the fit
/// keeps clear on each side of the image (`STAGE_MARGIN`).
const STAGE_TOP: f32 = 52.0 + 40.0;
const MARGIN: f32 = 24.0;

/// The rect, left, top, right, and bottom, of a `iw` x `ih` image in a
/// `w` x `h` client area, as the editor fits it at rest on a screen of
/// scale 1: inside the margins and at most one pixel per image pixel.
fn fit((w, h): (usize, usize), (iw, ih): (u32, u32)) -> (f32, f32, f32, f32) {
    let (w, h, iw, ih) = (w as f32, h as f32, iw as f32, ih as f32);
    let scale = ((w - 2.0 * MARGIN) / iw)
        .min((h - STAGE_TOP - 2.0 * MARGIN) / ih)
        .min(1.0);
    let (left, top) = (
        (w - iw * scale) / 2.0,
        STAGE_TOP + (h - STAGE_TOP - ih * scale) / 2.0,
    );
    (left, top, left + iw * scale, top + ih * scale)
}

/// An editor open on a capture of one colour, with the connection a
/// case drives it by.
struct Stage {
    daemon: Daemon,
    conn: RustConnection,
    root: u32,
    editor: u32,
    _dir: tempfile::TempDir,
    _xvfb: Server,
}

impl Stage {
    /// The editor once it shows a `size` capture of `color` at the
    /// middle of its rect; `None` when the case does not run.
    fn open(case: &str, size: (u32, u32), color: [u8; 3]) -> Option<Stage> {
        if !enabled() {
            return None;
        }
        let (xvfb, display) = xvfb_sized(case, SCREEN)?;
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plain.png");
        image::RgbImage::from_pixel(size.0, size.1, image::Rgb(color))
            .save(&file)
            .unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        daemon.send(&[OsStr::new("--annotate"), file.as_os_str()]);
        let editor = daemon.until("the editor to open", || {
            windows_of(&conn, root, EDITOR).first().copied()
        });
        daemon.until("the editor to draw the image", || {
            let (w, h, px) = client(&conn, root, editor);
            let (left, top, right, bottom) = fit((w, h), size);
            let (x, y) = (
                ((left + right) / 2.0) as usize,
                ((top + bottom) / 2.0) as usize,
            );
            let [b, g, r] = px[y * w + x];
            ([r, g, b] == color).then_some(())
        });
        daemon.settle();
        Some(Stage {
            daemon,
            conn,
            root,
            editor,
            _dir: dir,
            _xvfb: xvfb,
        })
    }

    /// The editor's client area, as `client` reads it.
    fn client(&self) -> (usize, usize, Vec<[u8; 3]>) {
        client(&self.conn, self.root, self.editor)
    }

    /// The client area's point `p` on the screen.
    fn screen_at(&self, p: (usize, usize)) -> (i16, i16) {
        let at = self
            .conn
            .translate_coordinates(self.editor, self.root, 0, 0)
            .unwrap()
            .reply()
            .unwrap();
        (at.dst_x + p.0 as i16, at.dst_y + p.1 as i16)
    }
}

/// The editor's client area, as `client` reads it, once it shows a
/// `size` image of one colour; `None` when the case does not run.
fn open(case: &str, size: (u32, u32)) -> Option<(usize, usize, Vec<[u8; 3]>)> {
    Stage::open(case, size, IMAGE).map(|stage| stage.client())
}

#[test]
fn the_editor_shows_every_pixel_of_the_capture_over_its_rect() {
    // Larger than the stage on both axes, at a fit that is not a whole
    // number, so the rect's edges fall between pixels.
    let size = (1300, 900);
    let Some(shot) = open(
        "the_editor_shows_every_pixel_of_the_capture_over_its_rect",
        size,
    ) else {
        return;
    };
    let (w, h, px) = &shot;
    let (left, top, right, bottom) = fit((*w, *h), size);
    // The pixels wholly inside the rect, and those it touches at all.
    let inside = (
        left.ceil() as usize,
        top.ceil() as usize,
        right.floor() as usize - 1,
        bottom.floor() as usize - 1,
    );
    let touched = (
        left.floor() as usize,
        top.floor() as usize,
        right.ceil() as usize - 1,
        bottom.ceil() as usize - 1,
    );
    let shown = extent(&shot, IMAGE).expect("the image is on the stage");
    assert!(
        shown.0 <= inside.0 && shown.1 <= inside.1 && shown.2 >= inside.2 && shown.3 >= inside.3,
        "the image shows over {shown:?}, short of the pixels {inside:?} wholly inside its rect \
         {:?} in a {w}x{h} client area",
        (left, top, right, bottom)
    );
    assert!(
        shown.0 >= touched.0
            && shown.1 >= touched.1
            && shown.2 <= touched.2
            && shown.3 <= touched.3,
        "the image shows over {shown:?}, past the pixels {touched:?} its rect {:?} touches",
        (left, top, right, bottom)
    );
    let [r, g, b] = IMAGE;
    for (x, y) in [
        (inside.0, inside.1),
        (inside.2, inside.1),
        (inside.0, inside.3),
        (inside.2, inside.3),
    ] {
        let [pb, pg, pr] = px[y * w + x];
        assert_eq!(
            [pr, pg, pb],
            [r, g, b],
            "the image's corner pixel at ({x}, {y}) of its rect {:?} shows another color",
            (left, top, right, bottom)
        );
    }
}

#[test]
fn a_capture_smaller_than_the_stage_opens_at_its_own_pixels() {
    let size = (200, 120);
    let Some(shot) = open(
        "a_capture_smaller_than_the_stage_opens_at_its_own_pixels",
        size,
    ) else {
        return;
    };
    let (left, top, right, bottom) = extent(&shot, IMAGE).expect("the image is on the stage");
    let shown = (right + 1 - left, bottom + 1 - top);
    assert_eq!(
        shown,
        (size.0 as usize, size.1 as usize),
        "a {size:?} capture shows {shown:?} pixels, from ({left}, {top})"
    );
    let (want_left, want_top, ..) = fit((shot.0, shot.1), size);
    assert_eq!(
        (left, top),
        (want_left.ceil() as usize, want_top.ceil() as usize),
        "a {size:?} capture is not centered on the stage of a {}x{} client area",
        shot.0,
        shot.1
    );
}

/// The rectangle's corners, in image pixels, on a `MARKED` capture,
/// which opens at its own pixels; the keysyms of the Rectangle and
/// Select tools.
const MARKED: (u32, u32) = (600, 400);
const RECT: [(usize, usize); 2] = [(100, 100), (200, 180)];
const R: u32 = 0x72;
const V: u32 = 0x76;

/// The frame's white line and dark rim over `c`: `theme::SELECTION_EDGE`,
/// white at 92%, and `theme::SELECTION_RIM`, black at 35%.
fn frame_over(c: [u8; 3]) -> ([u8; 3], [u8; 3]) {
    let line = c.map(|v| (v as f32 + (255.0 - v as f32) * 0.92).round() as u8);
    let rim = c.map(|v| (v as f32 * 0.65).round() as u8);
    (line, rim)
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(&a, b)| a.abs_diff(b) <= 3)
}

/// The pixels from the rectangle's stroke outward, `[r, g, b]`: the
/// stroke, the capture's `color` between the stroke and the frame, the
/// frame's white line and dark rim, then `color` again. Returns the
/// pixels between the stroke and the line.
fn frame_gap(out: &[[u8; 3]], color: [u8; 3], side: &str) -> usize {
    let (line, rim) = frame_over(color);
    let ink = out
        .iter()
        .position(|&p| near(p, color))
        .unwrap_or_else(|| panic!("{side}: the stroke runs past the pixels read: {out:?}"));
    let at = (ink..out.len() - 2)
        .find(|&i| near(out[i], line) && near(out[i + 1], rim))
        .unwrap_or_else(|| {
            panic!(
                "{side}: no white line {line:?} with a dark rim {rim:?} outside the \
                 selected rectangle on {color:?}: {out:?}"
            )
        });
    assert!(
        out[ink..at].iter().all(|&p| near(p, color)),
        "{side}: between the stroke and the frame's line is not the capture's {color:?}: {out:?}"
    );
    assert!(
        near(out[at + 2], color),
        "{side}: the frame is wider than a line and a rim on {color:?}: {out:?}"
    );
    at - ink
}

/// Draw the rectangle on a capture of `color`, select it, and read its
/// frame on the left side and the top.
fn selected_frame(case: &str, color: [u8; 3]) {
    let Some(stage) = Stage::open(case, MARKED, color) else {
        return;
    };
    let (w, h, _) = stage.client();
    let (left, top, ..) = fit((w, h), MARKED);
    let origin = (left.ceil() as usize, top.ceil() as usize);
    let at = |(x, y): (usize, usize)| stage.screen_at((origin.0 + x, origin.1 + y));
    tap(&stage.conn, stage.root, R);
    stage.daemon.settle();
    drag(&stage.conn, stage.root, at(RECT[0]), at(RECT[1]), 6);
    stage.daemon.settle();
    tap(&stage.conn, stage.root, V);
    stage.daemon.settle();
    let mid = ((RECT[0].0 + RECT[1].0) / 2, (RECT[0].1 + RECT[1].1) / 2);
    click(&stage.conn, stage.root, at((RECT[0].0, mid.1)));
    stage.daemon.settle();
    let (w, _, px) = stage.client();
    let rgb = |x: usize, y: usize| {
        let [b, g, r] = px[(origin.1 + y) * w + origin.0 + x];
        [r, g, b]
    };
    let leftward: Vec<_> = (RECT[0].0 - 20..=RECT[0].0)
        .rev()
        .map(|x| rgb(x, mid.1))
        .collect();
    let upward: Vec<_> = (RECT[0].1 - 20..=RECT[0].1)
        .rev()
        .map(|y| rgb(mid.0, y))
        .collect();
    for (side, out) in [("left side", leftward), ("top", upward)] {
        let gap = frame_gap(&out, color, side);
        assert!(
            (2..=4).contains(&gap),
            "{side}: the frame's line is {gap} px from the stroke on {color:?}, not 3"
        );
    }
}

#[test]
fn a_selected_mark_shows_its_frame_on_a_white_capture() {
    selected_frame(
        "a_selected_mark_shows_its_frame_on_a_white_capture",
        [255, 255, 255],
    );
}

#[test]
fn a_selected_mark_shows_its_frame_on_a_black_capture() {
    selected_frame(
        "a_selected_mark_shows_its_frame_on_a_black_capture",
        [0, 0, 0],
    );
}

#[test]
fn a_selected_mark_shows_its_frame_on_a_mid_capture() {
    selected_frame("a_selected_mark_shows_its_frame_on_a_mid_capture", IMAGE);
}
