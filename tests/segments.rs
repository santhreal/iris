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
use x11::{click, client, extent, glide, move_to, press_and_move, rect, release, tap, windows_of};

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

/// The tool options bar's center row: the 52 px toolbar, then the 40 px
/// bar (`editor::render_chrome::OPTIONS_H`).
const OPTIONS_ROW: usize = 52 + 40 / 2;
/// The toolbar's and the options bar's ground, `theme::BG_ELEV`.
const GROUND: [u8; 3] = [43, 43, 43];

/// The runs of `row` unlike GROUND from `from` to `to`: each its first
/// and last x, and whether it is a ring, the 2 px of accent, whole or
/// blended with the ground at a fractional edge, that circle the chosen
/// color.
fn controls(row: &[[u8; 3]], from: usize, to: usize) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    let mut x = from;
    for run in row[from..to].chunk_by(|a, b| (*a == GROUND) == (*b == GROUND)) {
        if run[0] != GROUND {
            let ring = run.len() <= 3 && run.iter().any(|&[b, _, r]| b > r.saturating_add(40));
            out.push((x, x + run.len() - 1, ring));
        }
        x += run.len();
    }
    out
}

/// WHY: the class closed here is "chrome that a layout centers on its
/// box instead of on what it shows". The toolbar centered the tools
/// between its unequal side paddings, 4 px right of the window's center
/// here and 38 px on macOS, so they stood off the options bar under
/// them; and each color's ring box, transparent 4 px past its color,
/// set the first color 16 px from its divider where every other control
/// stands 12 px from its neighbor, and moved the bar 2 px left of what
/// it shows. The case reads the tools' span against the window's center,
/// and the options bar's center row for even spaces and equal ends.
///
/// Not covered: the macOS and Windows paddings, which this build does
/// not lay out, and a window too narrow for the tools to center, where
/// they move toward the title.
#[test]
fn the_tools_and_their_options_center_and_space_evenly() {
    let case = "the_tools_and_their_options_center_and_space_evenly";
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
    move_to(&conn, root, (x0 + 10, y0 + 300));
    daemon.settle();

    let (w, _, px) = client(&conn, root, editor);
    let first = first_cell(&px, w).expect("the selected Pen segment's fill is on the toolbar");
    let tools = first + (COUNTER * PITCH + CELL) / 2;
    assert!(
        (2 * tools).abs_diff(w) <= 2,
        "the tools center {tools} px into a {w} px window"
    );

    let row = &px[OPTIONS_ROW * w..(OPTIONS_ROW + 1) * w];
    let runs = controls(row, 8, w - 8);
    let shown: Vec<_> = runs.iter().filter(|r| !r.2).collect();
    // The stroke widths, a divider, the fill toggle, a divider, and the
    // 11 colors.
    assert_eq!(
        shown.len(),
        15,
        "the options bar's center row reads {runs:?}"
    );
    let gaps: Vec<usize> = runs
        .windows(2)
        .filter(|p| !p[0].2 && !p[1].2)
        .map(|p| p[1].0 - p[0].1 - 1)
        .collect();
    let (lo, hi) = (gaps.iter().min().unwrap(), gaps.iter().max().unwrap());
    assert!(
        hi - lo <= 1,
        "the options bar's controls stand {gaps:?} px apart, not evenly"
    );
    let (left, right) = (shown[0].0, w - 1 - shown[14].1);
    assert!(
        left.abs_diff(right) <= 1,
        "the options bar shows its controls {left} px from the window's left edge \
         and {right} px from its right"
    );
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
    // The pointer stays on Ellipse, which the click selected: the
    // selected fill under the pointer.
    x11::until_color(
        &conn,
        root,
        (x0 + (left(ELLIPSE) + CELL / 2) as i16, y0 + FILL_ROW as i16),
        ACCENT_HOVER,
        "the Ellipse segment selected under the pointer",
    );
    let (_, _, px) = client(&conn, root, editor);
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

/// The selected segment's fill under the pointer and while pressed,
/// `theme::ACCENT_HOVER` and `theme::ACCENT_PRESS`, and at rest,
/// `theme::ACCENT`, as [r, g, b].
const ACCENT_HOVER: [u32; 3] = [12, 120, 228];
const ACCENT_PRESS: [u32; 3] = [15, 106, 197];
const ACCENT: [u32; 3] = [10, 132, 255];

/// WHY: the class closed here is "a press that draws no answer". The
/// selected segment kept its fill under the pointer and under a press,
/// where every other segment and the icon button that is on shade, so a
/// press on the selected tool, or on the fill toggle while it is on,
/// looked dead. The case reads the selected Pen cell above its glyph at
/// rest, hovered, pressed, with the press dragged off, and released
/// there.
///
/// Not covered: the fill toggle's own cell, which the same constructor
/// builds.
#[test]
fn the_selected_segment_shades_under_the_pointer_and_a_press() {
    let case = "the_selected_segment_shades_under_the_pointer_and_a_press";
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
    let park = (x0 + 10, y0 + 300);
    move_to(&conn, root, park);
    daemon.settle();

    let (w, _, px) = client(&conn, root, editor);
    let first = first_cell(&px, w).expect("the selected Pen segment's fill is on the toolbar");
    let cell = first + PEN * PITCH;
    let fill = (x0 + (cell + CELL / 2) as i16, y0 + FILL_ROW as i16);
    // Under the glyph, clear of the tooltip that opens below the pointer.
    let on = (
        x0 + (cell + CELL / 2) as i16,
        y0 + (CELL_TOP + CELL - 3) as i16,
    );
    x11::until_color(&conn, root, fill, ACCENT, "the selected Pen cell at rest");
    move_to(&conn, root, on);
    x11::until_color(&conn, root, fill, ACCENT_HOVER, "the hovered Pen cell");
    press_and_move(&conn, root, on, on, 1);
    x11::until_color(&conn, root, fill, ACCENT_PRESS, "the pressed Pen cell");
    glide(&conn, root, on, park, 20);
    x11::until_color(
        &conn,
        root,
        fill,
        ACCENT,
        "the Pen cell with its press dragged off",
    );
    release(&conn, root);
    daemon.settle();
    x11::until_color(
        &conn,
        root,
        fill,
        ACCENT,
        "the Pen cell after the release off it",
    );
}

/// The keysyms of `p`, `t`, and `v`, which pick Pen, Text, and Select.
const PEN_KEY: u32 = 0x70;
const TEXT_KEY: u32 = 0x74;
const SELECT_KEY: u32 = 0x76;

/// Pixels of `a` and `b`, two captures `w` px wide, in `rows` x `cols`
/// that differ by more than `over` on a channel.
fn changed(
    a: &[[u8; 3]],
    b: &[[u8; 3]],
    w: usize,
    (rows, cols): (Range<usize>, Range<usize>),
    over: u8,
) -> usize {
    rows.flat_map(|y| cols.clone().map(move |x| y * w + x))
        .filter(|&i| (0..3).any(|c| a[i][c].abs_diff(b[i][c]) > over))
        .count()
}

/// Levels apart: the tooltip's text from the bar's ground under it; and
/// a hover in a dimmed group, its glyph lit or its color's ring drawn,
/// about 26 at DISABLED_OPACITY, from the tooltip's shadow, under 6.
const TIP_TEXT: u8 = 40;
const DIM_HOVER: u8 = 12;

/// WHY: the class closed here is "a dimmed control that says nothing".
/// A dimmed group in the options bar lay under a layer that took the
/// pointer, so its controls showed no tooltip, where the dimmed Undo and
/// Redo beside them show theirs. The case dims each of the three groups
/// in turn, the fill toggle with Pen, the stroke widths with Text, and
/// the colors with Select, rests the pointer on one control of the group,
/// waits for a tooltip under it, and reads the bar for no hover: a glyph
/// lit or a color ringed; then clicks the control and reads the bar for
/// no change of value.
///
/// Not covered: a dimmed segment's hover wash without its lit glyph,
/// about 5 levels at DISABLED_OPACITY, as faint as the tooltip's shadow
/// over the bar; the tooltip's text; and a group dimmed by the selected
/// annotation rather than by the tool.
#[test]
fn a_dimmed_option_keeps_its_tooltip_and_takes_no_click() {
    let case = "a_dimmed_option_keeps_its_tooltip_and_takes_no_click";
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
    let park = (x0 + 10, y0 + 300);
    move_to(&conn, root, park);
    daemon.settle();

    // (group, the key that dims it, its first and last places among the
    // bar's controls, and the first control's center's offset from its
    // left edge.)
    let groups: [(&str, u32, usize, usize, usize); 3] = [
        ("fill toggle", PEN_KEY, 2, 2, 14),
        ("stroke widths", TEXT_KEY, 0, 0, 14),
        ("colors", SELECT_KEY, 4, 14, 8),
    ];
    for (group, key, place, last, offset) in groups {
        tap(&conn, root, key);
        move_to(&conn, root, park);
        daemon.settle();
        let (w, _, before) = client(&conn, root, editor);
        let row = &before[OPTIONS_ROW * w..(OPTIONS_ROW + 1) * w];
        let shown: Vec<_> = controls(row, 8, w - 8)
            .into_iter()
            .filter(|r| !r.2)
            .collect();
        assert_eq!(
            shown.len(),
            15,
            "the options bar's center row reads {shown:?}"
        );
        let x = shown[place].0 + offset;
        let on = (x0 + x as i16, y0 + OPTIONS_ROW as i16);
        move_to(&conn, root, on);
        // The tooltip opens below and right of the pointer, under the bar.
        let tip_rows = OPTIONS_ROW + 16..OPTIONS_ROW + 56;
        let tip_cols = x..x + 80;
        let shown_tip = daemon.until(&format!("the dimmed {group}'s tooltip"), || {
            let (_, _, now) = client(&conn, root, editor);
            let tip = (tip_rows.clone(), tip_cols.clone());
            (changed(&before, &now, w, tip, TIP_TEXT) >= 30).then_some(now)
        });
        // The group's rows and columns, its colors' 4 px ring boxes in:
        // the tooltip's shadow darkens a full color beside it by up to
        // 16 levels, and a dimmed one by under DIM_HOVER.
        let own = (52..52 + 36, shown[place].0 - 4..shown[last].1 + 5);
        assert_eq!(
            changed(&before, &shown_tip, w, own, DIM_HOVER),
            0,
            "the dimmed {group} changes under the pointer"
        );
        click(&conn, root, on);
        move_to(&conn, root, park);
        daemon.settle();
        let (_, _, after) = client(&conn, root, editor);
        assert_eq!(
            changed(&before, &after, w, (52..52 + 36, 0..w), DIM_HOVER),
            0,
            "a click on the dimmed {group} changes the options bar"
        );
    }
}
