//! The capture overlay's capture bar, driven on a private Xvfb with no
//! window manager.
//!
//! WHY: the class closed here is "a capture bar button that does not run
//! what it shows". Every button of the bar is clicked here: cancel parks
//! the overlay with nothing saved; the entire screen makes the next press
//! capture the whole monitor; the run button captures a dragged
//! selection; and the record selection choice makes the run button start
//! a region recording. Each case reads the result, a saved image's size
//! or the recording's start, so a button that does nothing, or one wired
//! to another button's action, fails. The bar's geometry below mirrors
//! `src/bin/iris/overlay/bar.rs`: a bar placed elsewhere makes the clicks
//! miss and the cases fail.
//!
//! Not covered: the look of the bar and of the overlay, which the
//! screenshots show; tooltips; a monitor layout of more than one monitor;
//! Wayland, Windows, and macOS. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need Xvfb, and the recording case
//! needs ffmpeg; without them a case prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/recording.rs"]
mod recording;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::{path::Path, time::Duration};

use x11rb::connection::Connection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{click, drag, paint_root, windows_of};

/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;
const OVERLAY: &str = "Capture - iris";
const TOAST: &str = "Screenshot - iris";
const SCREEN: (u32, u32) = (1280, 1024);

/// The bar on a 1280x1024 monitor: 317 px wide (a 1 px border, 6 px of
/// padding, five 36 px buttons, three 9 px dividers, the 80 px run
/// button, and eight 2 px gaps), centered, its bottom 64 px above the
/// monitor's. Its buttons' centers, left to right, all on one row.
const BAR_LEFT: i16 = 482;
const ROW: i16 = 1024 - 64 - 50 + 25;
const CANCEL: (i16, i16) = (BAR_LEFT + 25, ROW);
const ENTIRE_SCREEN: (i16, i16) = (BAR_LEFT + 74, ROW);
const RECORD_SELECTION: (i16, i16) = (BAR_LEFT + 199, ROW);
const RUN: (i16, i16) = (BAR_LEFT + 270, ROW);

/// Longer than a double-click: two clicks this far apart are two
/// single clicks.
const APART: Duration = Duration::from_millis(500);

/// A daemon on its own Xvfb with a one-colour desktop.
struct Session {
    _xvfb: daemon::Server,
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
    dir: tempfile::TempDir,
    daemon: Daemon,
}

fn session(case: &str) -> Option<Session> {
    if !enabled() {
        return None;
    }
    let (xvfb, display) = xvfb_sized(case, "1280x1024x24")?;
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    Some(Session {
        _xvfb: xvfb,
        conn,
        root,
        dir,
        daemon,
    })
}

/// Start a capture and wait until its overlay has the focus and its
/// frozen frame has landed.
fn open_overlay(s: &Session) {
    s.daemon.send(&["--capture"]);
    s.daemon.until("the overlay to take the focus", || {
        windows_of(&s.conn, s.root, OVERLAY)
            .contains(&x11::focus(&s.conn))
            .then_some(())
    });
    s.daemon.until("the frozen frame to land", || {
        s.daemon.log().contains("frame landed").then_some(())
    });
    s.daemon.settle();
}

/// Wait for the first capture to land and its toast to open, and return
/// the capture's pixel size.
fn captured(s: &Session, dir: &Path) -> (u32, u32) {
    let path = s.daemon.until("the capture to land in its folder", || {
        let entries = daemon::entries(dir)?;
        entries.first().map(|e| e.path.clone())
    });
    s.daemon.until("the toast to open", || {
        windows_of(&s.conn, s.root, TOAST).first().copied()
    });
    image::image_dimensions(&path).unwrap()
}

#[test]
fn the_cancel_button_parks_the_overlay_with_nothing_saved() {
    let Some(s) = session("the_cancel_button_parks_the_overlay_with_nothing_saved") else {
        return;
    };
    open_overlay(&s);
    click(&s.conn, s.root, CANCEL);
    s.daemon.until("the overlay to park", || {
        windows_of(&s.conn, s.root, OVERLAY)
            .is_empty()
            .then_some(())
    });
    s.daemon.settle();
    let saved = daemon::entries(s.dir.path()).unwrap_or_default().len();
    assert_eq!(saved, 0, "cancel saved a capture");
}

#[test]
fn the_entire_screen_choice_captures_the_monitor_on_the_next_press() {
    let Some(s) = session("the_entire_screen_choice_captures_the_monitor_on_the_next_press") else {
        return;
    };
    open_overlay(&s);
    click(&s.conn, s.root, ENTIRE_SCREEN);
    std::thread::sleep(APART);
    click(&s.conn, s.root, (300, 300));
    assert_eq!(captured(&s, s.dir.path()), SCREEN);
}

#[test]
fn the_run_button_captures_the_dragged_selection() {
    let Some(s) = session("the_run_button_captures_the_dragged_selection") else {
        return;
    };
    open_overlay(&s);
    drag(&s.conn, s.root, (200, 200), (600, 500), 20);
    std::thread::sleep(APART);
    click(&s.conn, s.root, RUN);
    assert_eq!(captured(&s, s.dir.path()), (400, 300));
}

#[test]
fn the_record_selection_choice_makes_the_run_button_record() {
    let case = "the_record_selection_choice_makes_the_run_button_record";
    if !recording::on_path("ffmpeg") {
        eprintln!("{case} did not run: no ffmpeg on PATH");
        return;
    }
    let Some(s) = session(case) else {
        return;
    };
    open_overlay(&s);
    click(&s.conn, s.root, RECORD_SELECTION);
    std::thread::sleep(APART);
    drag(&s.conn, s.root, (200, 200), (520, 440), 20);
    std::thread::sleep(APART);
    click(&s.conn, s.root, RUN);
    s.daemon.until("the region recording to start", || {
        s.daemon
            .log()
            .contains("iris: record: segment")
            .then_some(())
    });
    std::thread::sleep(Duration::from_millis(500));
    // A live recording: the toggle stops it.
    s.daemon.send(&["--record-window"]);
    s.daemon.until("the recording to be saved", || {
        s.daemon
            .log()
            .contains("iris: recording saved")
            .then_some(())
    });
    let shots = daemon::entries(s.dir.path())
        .unwrap_or_default()
        .iter()
        .filter(|e| e.path.extension().is_some_and(|x| x == "png"))
        .count();
    assert_eq!(shots, 0, "the record choice saved a screenshot");
}
