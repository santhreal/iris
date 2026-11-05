//! The editor's file name and image size in its toolbar, read from the
//! screen at the size the window opens at and at its least size.
//!
//! WHY: the class closed here is "the toolbar truncates the title at the
//! window's least size". At 760 px wide the editor's tools and buttons
//! left the title 68 px, so a file name of the default template showed as
//! "2026-11…" and the image's size as "1800 × 11…". The case opens an
//! editor on a capture named by the default template, reads where the
//! name's ink and the size's ink end at the size the editor opens at,
//! resizes the window to the least size its WM_NORMAL_HINTS state, and
//! reads both again: each ends where it ended before.
//!
//! Not covered: the Windows caption buttons, 32 px wider than the round
//! buttons drawn here, which the least size leaves room for without this
//! case reading it; macOS, whose traffic lights leave the title more
//! room; a desktop button layout with the buttons on the leading side; a
//! file name longer than the template's, which truncates. The case runs
//! only with `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without it the
//! case prints that it did not run.

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
use x11rb::properties::WmSizeHints;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _};

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{client, extent, windows_of, Compositor};

const SCREEN: &str = "1600x1200x24";
/// A file name of the default `{date}_{time}` template.
const NAME: &str = "2026-11-05_17-23-04.png";
const EDITOR: &str = "2026-11-05_17-23-04.png - iris";
/// The image's size and its one color, [r, g, b].
const SIZE: (u32, u32) = (1800, 1120);
const IMAGE: [u8; 3] = [60, 140, 200];
/// The rows, from the window's top, of the name's ink and of the size's
/// ink below it in the 52 px toolbar: the glyphs' bodies, clear of the
/// name's descenders.
const NAME_ROWS: std::ops::Range<usize> = 11..25;
const SIZE_ROWS: std::ops::Range<usize> = 29..41;
/// Columns with no ink that end a line of text: wider than a space in
/// either line, narrower than the space between the title and the tools.
const GAP: usize = 12;

/// The first and last columns of the text that starts the toolbar on
/// `rows`: ink is a pixel with a channel over 90, which the light name,
/// the dimmer size, and no part of the toolbar's dark ground has.
fn line_ink(
    &(w, _, ref px): &(usize, usize, Vec<[u8; 3]>),
    rows: std::ops::Range<usize>,
) -> (usize, usize) {
    let inked = |x: usize| rows.clone().any(|y| px[y * w + x].iter().any(|&c| c > 90));
    let first = (0..w / 2).find(|&x| inked(x)).expect("ink in the toolbar");
    let (mut last, mut gap) = (first, 0);
    for x in first + 1..w {
        if inked(x) {
            last = x;
            gap = 0;
        } else {
            gap += 1;
            if gap == GAP {
                break;
            }
        }
    }
    (first, last)
}

#[test]
fn the_editor_shows_its_whole_title_at_its_least_size() {
    if !enabled() {
        return;
    }
    let case = "the_editor_shows_its_whole_title_at_its_least_size";
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    // With a compositing manager running, iris draws its own window
    // buttons in the toolbar, beside the title.
    let selection = conn
        .intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let _compositor = Compositor::start(&conn, root, selection);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(NAME);
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
    daemon.settle();
    let opened = client(&conn, root, editor);
    let name = line_ink(&opened, NAME_ROWS);
    let size = line_ink(&opened, SIZE_ROWS);

    let (least_w, least_h) = WmSizeHints::get_normal_hints(&conn, editor)
        .unwrap()
        .reply()
        .unwrap()
        .and_then(|hints| hints.min_size)
        .expect("the editor states a least size");
    assert!(
        (least_w as usize) < opened.0,
        "the editor opens at its least width {least_w}, so the case reads one size"
    );
    conn.configure_window(
        editor,
        &ConfigureWindowAux::new()
            .width(least_w as u32)
            .height(least_h as u32),
    )
    .unwrap();
    conn.flush().unwrap();
    daemon.until("the editor to draw at its least size", || {
        let shot = client(&conn, root, editor);
        (shot.0 == least_w as usize && shot.1 == least_h as usize)
            .then(|| extent(&shot, IMAGE))
            .flatten()
    });
    daemon.settle();
    let least = client(&conn, root, editor);
    assert_eq!(
        line_ink(&least, NAME_ROWS),
        name,
        "the file name's ink at {least_w}x{least_h}, against at {}x{}",
        opened.0,
        opened.1
    );
    assert_eq!(
        line_ink(&least, SIZE_ROWS),
        size,
        "the image size's ink at {least_w}x{least_h}, against at {}x{}",
        opened.0,
        opened.1
    );
}
