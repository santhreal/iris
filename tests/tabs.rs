//! The Settings pane tabs under the pointer, read from the screen.
//!
//! WHY: the class closed here is "a tab lit by something other than the
//! pointer over it". Two defects lit tabs wrongly. A tab clicked away
//! from kept the hover color of its label until the pointer moved again,
//! because layout read a hover flag that only mouse moves updated and
//! that froze while the tab was selected. A hovered tab lit its label
//! over a glyph painted in a fixed dim color. The case clicks one tab,
//! then another, and reads the first tab's label against a tab never
//! hovered; then rests the pointer on an unselected tab and reads its
//! glyph and label against their unhovered peaks.
//!
//! Not covered: the editor's tool segments and the toolbar chevron,
//! which paint their glyphs through the same text-color constructor. The
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

use std::ops::Range;

use x11rb::connection::Connection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{click, client, move_to, rect, windows_of};

const SCREEN: &str = "1280x1024x24";
const SETTINGS: &str = "Settings - iris";
/// The Settings window's width, logical px.
const WIDTH: f32 = 600.0;
/// The tab cell is 44 tall, centered in the 52px toolbar, and stacks an
/// 18px glyph, a 3px gap, and a label line. Rows of each, logical px.
const GLYPH_ROWS: Range<f32> = 9.0..26.0;
const LABEL_ROWS: Range<f32> = 30.0..43.0;
/// Left of this, logical px, the toolbar holds the window controls.
const TABS_FROM: f32 = 100.0;
/// The tabs, left to right.
const CAPTURE: usize = 1;
const RECORDING: usize = 2;
const UPDATES: usize = 4;
/// Levels apart: a lit peak from a dim one (FG over FG_DIM is about 78
/// at full coverage, 30 at the 40% a thin stem reaches), and two
/// readings of one label drawn alike.
const LIT: u8 = 30;
const SAME: u8 = 8;

/// The brightest neutral level in `rows` x `cols` of `px`: the least
/// channel of each pixel, so an accent-colored selected label never
/// counts as lit white.
fn peak(px: &[[u8; 3]], w: usize, rows: &Range<usize>, cols: &Range<usize>) -> u8 {
    rows.clone()
        .flat_map(|y| cols.clone().map(move |x| px[y * w + x]))
        .map(|p| p[0].min(p[1]).min(p[2]))
        .max()
        .unwrap()
}

/// The column span of each tab label, left to right: runs of columns in
/// `rows` that differ from the toolbar, joined across letter gaps.
fn labels(
    px: &[[u8; 3]],
    w: usize,
    rows: &Range<usize>,
    from: usize,
    gap: usize,
) -> Vec<Range<usize>> {
    let ground = px[rows.start * w + from];
    let inked = |x: usize| {
        rows.clone().any(|y| {
            let p = px[y * w + x];
            (0..3).any(|c| p[c].abs_diff(ground[c]) > 50)
        })
    };
    let mut spans: Vec<Range<usize>> = Vec::new();
    for x in (from..w).filter(|&x| inked(x)) {
        match spans.last_mut() {
            Some(span) if x - span.end <= gap => span.end = x + 1,
            _ => spans.push(x..x + 1),
        }
    }
    spans
}

#[test]
fn a_tab_lights_only_under_the_pointer() {
    let case = "a_tab_lights_only_under_the_pointer";
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
    daemon.send(&["--settings"]);
    let settings = daemon.until("Settings to open", || {
        windows_of(&conn, root, SETTINGS).first().copied()
    });
    daemon.settle();
    let (x0, y0, ..) = rect(&conn, root, settings);
    // Rest the pointer on the pane, clear of the tabs.
    move_to(&conn, root, (x0 + 10, y0 + 300));
    daemon.settle();

    let (w, _, px) = client(&conn, root, settings);
    let scale = w as f32 / WIDTH;
    let rows = |r: &Range<f32>| (r.start * scale) as usize..(r.end * scale) as usize;
    let (glyph_rows, label_rows) = (rows(&GLYPH_ROWS), rows(&LABEL_ROWS));
    let tabs = labels(
        &px,
        w,
        &label_rows,
        (TABS_FROM * scale) as usize,
        (6.0 * scale) as usize,
    );
    assert_eq!(tabs.len(), 5, "the toolbar shows five tab labels: {tabs:?}");
    // The window resizes to each pane; read its origin at each use.
    let center = |tab: usize| {
        let (x0, y0, ..) = rect(&conn, root, settings);
        let x = (tabs[tab].start + tabs[tab].end) / 2;
        let y = (label_rows.start + label_rows.end) / 2;
        (x0 + x as i16, y0 + y as i16)
    };
    let dim_glyph = peak(&px, w, &glyph_rows, &tabs[UPDATES]);
    let dim_label = peak(&px, w, &label_rows, &tabs[UPDATES]);
    let dim_capture = peak(&px, w, &label_rows, &tabs[CAPTURE]);

    // Select Capture, then Recording: the pointer leaves Capture by a
    // click on another tab and moves no more.
    click(&conn, root, center(CAPTURE));
    daemon.settle();
    click(&conn, root, center(RECORDING));
    daemon.settle();
    let (_, _, px) = client(&conn, root, settings);
    let left = peak(&px, w, &label_rows, &tabs[CAPTURE]);
    assert!(
        left.abs_diff(dim_capture) <= SAME,
        "the tab clicked away from stays lit: its label peaks at {left}, \
         {dim_capture} before it was hovered"
    );

    // Rest the pointer on Updates, unselected.
    move_to(&conn, root, center(UPDATES));
    daemon.settle();
    let (_, _, px) = client(&conn, root, settings);
    let label = peak(&px, w, &label_rows, &tabs[UPDATES]);
    let glyph = peak(&px, w, &glyph_rows, &tabs[UPDATES]);
    assert!(
        label >= dim_label.saturating_add(LIT),
        "the hovered tab's label stays dim: {label}, unhovered {dim_label}"
    );
    assert!(
        glyph >= dim_glyph.saturating_add(LIT),
        "the hovered tab's glyph stays dim beside its lit label: {glyph}, unhovered {dim_glyph}"
    );
}
