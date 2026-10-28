//! Icons in a translucent color, read back from the screen.
//!
//! WHY: the class closed here is "parts of a glyph add up where they
//! overlap". Icons were triangle soups painted as one path, and the
//! path rasterizer blends each triangle over the last: in a color with
//! alpha below 1, every round cap, corner, and crossing drew brighter
//! than the stroke it belonged to, so a bracket showed bright knots at
//! its corners and a line its endpoints. The case opens the empty
//! library, whose 40 px viewfinder glyph is four corner brackets in a
//! faint white, and reads that the brightest ink is the stroke's own.
//!
//! Not covered: Windows and macOS, whose renderers paint the same
//! sprite; a glyph in an opaque color, where overlaps do not show. The
//! case runs only with `IRIS_X11_TEST_DISPLAY` set and needs Xvfb;
//! without them it prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use x11rb::connection::Connection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{client, windows_of};

const LIBRARY: &str = "Library - iris";
const SCREEN: &str = "1280x1024x24";
/// Half the width of the column read around the window's centre: the
/// 40 px glyph and none of the toolbar's side controls.
const BAND: usize = 24;

#[test]
fn a_glyph_in_a_translucent_color_draws_its_overlaps_once() {
    let case = "a_glyph_in_a_translucent_color_draws_its_overlaps_once";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&["--library"]);
    let library = daemon.until("the library to open", || {
        windows_of(&conn, root, LIBRARY).first().copied()
    });
    let (peak, stroke, bg) = daemon.until("the empty library to draw its glyph", || {
        glyph_ink(&client(&conn, root, library))
    });
    assert!(
        peak - stroke <= 6,
        "ink at {peak} over a stroke at {stroke} above the background {bg:?}: \
         overlapping parts of the glyph drew twice"
    );
}

/// The empty library's glyph in the window `(w, h, px)`: its brightest
/// ink and its stroke's level above the background, and the
/// background. `None` until the window draws the glyph.
fn glyph_ink(&(w, h, ref px): &(usize, usize, Vec<[u8; 3]>)) -> Option<(i32, i32, [u8; 3])> {
    // The empty state stacks the glyph over two lines of text in the
    // window's centre, on the window's background.
    let bg = px[(h - 4) * w + w / 2];
    let lift = |p: [u8; 3]| i32::from(p[1]) - i32::from(bg[1]);
    let band = |y: usize| &px[y * w + w / 2 - BAND..y * w + w / 2 + BAND];
    // Runs of inked rows. The gap between the glyph's top and bottom
    // brackets joins; the gap above a line of text does not. The glyph
    // is the lowest run taller than a line of text.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for y in 0..h {
        if !band(y).iter().any(|&p| lift(p) > 12) {
            continue;
        }
        match runs.last_mut() {
            Some(run) if y - run.1 <= 10 => run.1 = y,
            _ => runs.push((y, y)),
        }
    }
    let glyph = runs.iter().rev().find(|run| run.1 - run.0 >= 24)?;
    let mut ink: Vec<i32> = (glyph.0..=glyph.1)
        .flat_map(|y| band(y).iter().map(|&p| lift(p)))
        .filter(|&l| l > 12)
        .collect();
    ink.sort_unstable();
    // Most stroked pixels are fully covered, so the median is the
    // stroke's own level; a pixel brighter than that is an overlap
    // drawn twice.
    Some((*ink.last()?, ink[ink.len() / 2], bg))
}
