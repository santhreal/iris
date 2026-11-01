//! The editor's tool segments under the pointer, read from the screen.
//!
//! WHY: the class closed here is "a segment lit by something other than
//! the pointer over it". Two defects lit toolbar segments wrongly: a
//! segment clicked away from kept its hover color until the pointer
//! moved again, because layout read a hover flag that only mouse moves
//! updated and that froze while the segment was selected; and a hovered
//! segment drew its glyph in a fixed dim color. The case selects
//! Rectangle, then Ellipse, and reads the Rectangle glyph against its
//! reading before either click; then rests the pointer on Counter,
//! unselected, in its cell's corner clear of the glyph and of the
//! tooltip that opens beside the pointer, and reads the glyph against
//! its unhovered peak.
//!
//! Not covered: the toolbar's icon buttons and the Copy chevron, which
//! paint their glyphs through the same text-color constructor. The case
//! runs only with `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without
//! them it prints that it did not run.

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
use std::ops::Range;

use x11rb::connection::Connection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{click, client, extent, move_to, rect, windows_of};

const SCREEN: &str = "1280x1024x24";
const EDITOR: &str = "plain.png - iris";
/// The image's size and its one color, [r, g, b].
const SIZE: (u32, u32) = (600, 400);
const IMAGE: [u8; 3] = [60, 140, 200];
/// The tool segments, px (`widgets::segmented`): square cells 24 wide
/// and 2 apart from y 14 in the 52 px toolbar, each with a 16 px glyph
/// centered in it; and a row of the selected cell's fill above its
/// glyph.
const CELL: usize = 24;
const PITCH: usize = 26;
const CELL_TOP: usize = 14;
const GLYPH: usize = 16;
const FILL_ROW: usize = 16;
/// The tools' places in the control (`editor::action::TOOLS`). Pen is
/// selected when the editor opens.
const PEN: usize = 1;
const ELLIPSE: usize = 4;
const RECTANGLE: usize = 5;
const COUNTER: usize = 10;
/// Levels apart: a lit peak from a dim one (FG over FG_DIM is about 78
/// at full coverage, 30 at the 40% a thin stem reaches), and two
/// readings of one glyph drawn alike.
const LIT: u8 = 30;
const SAME: u8 = 8;

/// The selected segment's fill, `theme::ACCENT` (#0a84ff), as [b, g, r].
fn accent([b, g, r]: [u8; 3]) -> bool {
    b > 240 && (120..145).contains(&g) && r < 30
}

/// The brightest neutral level in `rows` x `cols` of `px`: the least
/// channel of each pixel, so the accent fill never counts as lit white.
fn peak(px: &[[u8; 3]], w: usize, rows: &Range<usize>, cols: &Range<usize>) -> u8 {
    rows.clone()
        .flat_map(|y| cols.clone().map(move |x| px[y * w + x]))
        .map(|p| p[0].min(p[1]).min(p[2]))
        .max()
        .unwrap()
}

/// The left edge of the first tool segment: the selected Pen cell is the
/// run of fill on FILL_ROW no wider than a cell, centered in it, where
/// Done's fill is wider.
fn first_cell(px: &[[u8; 3]], w: usize) -> Option<usize> {
    let row = &px[FILL_ROW * w..(FILL_ROW + 1) * w];
    let mut x = 0;
    for run in row.chunk_by(|a, b| accent(*a) == accent(*b)) {
        if accent(run[0]) && run.len() <= CELL {
            return (x + run.len() / 2).checked_sub(PEN * PITCH + CELL / 2);
        }
        x += run.len();
    }
    None
}

#[test]
fn a_segment_lights_only_under_the_pointer() {
    let case = "a_segment_lights_only_under_the_pointer";
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
    daemon.until("the editor to draw the image", || {
        extent(&client(&conn, root, editor), IMAGE).map(|_| ())
    });
    let (x0, y0, ..) = rect(&conn, root, editor);
    // Rest the pointer under the toolbar, clear of the segments.
    move_to(&conn, root, (x0 + 10, y0 + 300));
    daemon.settle();

    let (w, _, px) = client(&conn, root, editor);
    let first = first_cell(&px, w).expect("the selected Pen segment's fill is on the toolbar");
    let left = |tool: usize| first + tool * PITCH;
    let glyph = |tool: usize| left(tool) + (CELL - GLYPH) / 2..left(tool) + (CELL + GLYPH) / 2;
    let rows = CELL_TOP + (CELL - GLYPH) / 2..CELL_TOP + (CELL + GLYPH) / 2;
    // A point `d` px right of and below a cell's top-left corner.
    let at = |tool: usize, d: usize| (x0 + (left(tool) + d) as i16, y0 + (CELL_TOP + d) as i16);
    let dim_rectangle = peak(&px, w, &rows, &glyph(RECTANGLE));
    let dim_counter = peak(&px, w, &rows, &glyph(COUNTER));

    // Select Rectangle, then Ellipse: the pointer leaves Rectangle by a
    // click on another segment and moves no more.
    click(&conn, root, at(RECTANGLE, CELL / 2));
    daemon.settle();
    click(&conn, root, at(ELLIPSE, CELL / 2));
    daemon.settle();
    let (_, _, px) = client(&conn, root, editor);
    assert!(
        accent(px[FILL_ROW * w + left(ELLIPSE) + CELL / 2]),
        "a click on the Ellipse segment left it unselected"
    );
    let after = peak(&px, w, &rows, &glyph(RECTANGLE));
    assert!(
        after.abs_diff(dim_rectangle) <= SAME,
        "the segment clicked away from stays lit: its glyph peaks at {after}, \
         {dim_rectangle} before it was hovered"
    );

    // Rest the pointer on Counter, unselected, past its glyph: the
    // tooltip opens below and right of the pointer.
    move_to(&conn, root, at(COUNTER, CELL - 2));
    daemon.settle();
    let (_, _, px) = client(&conn, root, editor);
    let hovered = peak(&px, w, &rows, &glyph(COUNTER));
    assert!(
        hovered >= dim_counter.saturating_add(LIT),
        "the hovered segment's glyph stays dim: {hovered}, unhovered {dim_counter}"
    );
}
