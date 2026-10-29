//! The highlighter's stroke on the editor's canvas, read back from the
//! screen.
//!
//! WHY: the class closed here is "a translucent stroke on the canvas adds
//! up where its parts overlap". The canvas painted a stroke as one path of
//! a quad and two round caps per segment, and the path rasterizer blends
//! each triangle over the last. A highlight, at alpha 0.35, takes a point
//! every 2 px of pointer travel, so each pixel sat under several caps and
//! quads: the canvas showed a near-opaque bar where the saved file holds
//! one 0.35 wash. The case drags a highlight across an image of one color
//! and reads that every pixel of the stroke's core is one blend of the
//! stroke's color over the image.
//!
//! Not covered: Windows and macOS, whose renderers composite the same
//! path coverage; opaque strokes, where overlaps do not show. The case
//! runs only with `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without them
//! it prints that it did not run.

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
use x11::{client, drag, extent, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The editor's title on `plain.png`.
const EDITOR: &str = "plain.png - iris";
/// The image's one color, [r, g, b]: each channel far from the stroke's.
const IMAGE: [u8; 3] = [60, 140, 200];
/// The editor's first color, `#ff453a`, and the highlight's alpha.
const STROKE: [u8; 3] = [0xff, 0x45, 0x3a];
const ALPHA: f32 = 0.35;
/// The keysym of `h`, which selects the highlighter.
const H: u32 = 0x68;
/// How far a channel may read from the one blend: rounding in the blend
/// and the image's scaling.
const SLACK: i32 = 4;

#[test]
fn a_highlight_on_the_canvas_blends_its_color_once() {
    let case = "a_highlight_on_the_canvas_blends_its_color_once";
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
    image::RgbImage::from_pixel(600, 400, image::Rgb(IMAGE))
        .save(&file)
        .unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&[OsStr::new("--annotate"), file.as_os_str()]);
    let editor = daemon.until("the editor to open", || {
        windows_of(&conn, root, EDITOR).first().copied()
    });
    // The image, once it spans 200 by 100 px.
    let (left, top, right, bottom) = daemon.until("the editor to draw the image", || {
        extent(&client(&conn, root, editor), IMAGE)
            .filter(|&(left, top, right, bottom)| right - left >= 200 && bottom - top >= 100)
    });
    daemon.settle();
    let at = conn
        .translate_coordinates(editor, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let screen_at = |x: usize, y: usize| (at.dst_x + x as i16, at.dst_y + y as i16);
    let y = (top + bottom) / 2;
    tap(&conn, root, H);
    daemon.settle();
    drag(
        &conn,
        root,
        screen_at(left + 40, y),
        screen_at(right - 40, y),
        3,
    );
    // The stroke's core, clear of its round ends and of anything drawn
    // at the pointer where the drag ended.
    let core = left + 80..right - 80;
    let row = |(w, _, px): &(usize, usize, Vec<[u8; 3]>)| -> Vec<[u8; 3]> {
        core.clone()
            .map(|x| {
                let [b, g, r] = px[y * w + x];
                [r, g, b]
            })
            .collect()
    };
    daemon.until("the stroke to draw", || {
        let row = row(&client(&conn, root, editor));
        row.iter().all(|&p| p != IMAGE).then_some(())
    });
    daemon.settle();
    let row = row(&client(&conn, root, editor));
    let want: [i32; 3] = std::array::from_fn(|c| {
        let (under, over) = (f32::from(IMAGE[c]), f32::from(STROKE[c]));
        (under + (over - under) * ALPHA).round() as i32
    });
    let off: Vec<(usize, [u8; 3])> = row
        .iter()
        .enumerate()
        .filter(|(_, p)| (0..3).any(|c| (i32::from(p[c]) - want[c]).abs() > SLACK))
        .map(|(i, &p)| (core.start + i, p))
        .collect();
    assert!(
        off.is_empty(),
        "{} of {} pixels of the stroke's core are not one blend {want:?} of {STROKE:?} \
         over {IMAGE:?}; first {:?}",
        off.len(),
        row.len(),
        &off[..off.len().min(8)]
    );
}
