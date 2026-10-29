//! WHY: a capture that another program deletes, or moves out of its
//! folder, leaves an open library window. The window watches the folders
//! that hold its captures, and the read of the store that a change
//! starts drops the capture from `library.json`. The classes closed here
//! are a removal the window reads only once something else makes it
//! read the store, a removal in a folder past the first, and a watch
//! that ends when the window's folders change. Each removal lands once
//! the daemon is idle, after the read that follows the watch's start.
//!
//! The second case closes "an open context menu takes no keys": with
//! the menu open, the arrows and Return did nothing. It opens a card's
//! menu, presses Down and Return, and reads the editor that Open
//! starts.
//!
//! Not covered: a read on a timer, which `tests/idle.rs` rejects; the
//! watches of Wayland, Windows, and macOS, which the tests of
//! `src/bin/iris/sys/watch.rs` cover; the menu's other rows, whose keys
//! `widgets::menu_key` runs as Open's. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need `Xvfb`; without it they print
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

use std::path::{Path, PathBuf};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

use daemon::{enabled, store, stored, xvfb, xvfb_sized, Daemon};
use x11::{click_with, client, extent, move_to, tap, windows_of};

/// The title of the library window.
const LIBRARY: &str = "Library - iris";
/// The test captures' one color, [r, g, b, a].
const CAPTURE: [u8; 4] = [40, 90, 200, 255];
/// Keysyms.
const DOWN: u32 = 0xff54;
const RETURN: u32 = 0xff0d;

/// A capture at `path`, its folder made.
fn png(path: &Path) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbaImage::from_pixel(64, 48, image::Rgba(CAPTURE))
        .save(path)
        .unwrap();
    path.to_path_buf()
}

#[test]
fn a_capture_removed_outside_iris_leaves_the_open_library() {
    if !enabled() {
        return;
    }
    let case = "a_capture_removed_outside_iris_leaves_the_open_library";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let shots = dir.path().join("shots");
    let kept = png(&shots.join("kept.png"));
    // `shots/2026` sorts after `shots`: the second folder watched.
    let deeper = png(&shots.join("2026").join("deeper.png"));
    store(dir.path(), &[deeper.clone(), kept.clone()]);
    daemon.send(&["--library"]);
    daemon.until("the library window to map", || {
        (windows_of(&conn, root, LIBRARY).len() == 1).then_some(())
    });
    daemon.settle();

    std::fs::remove_file(&deeper).unwrap();
    daemon.until(
        "the library to drop the capture deleted from shots/2026",
        || (stored(dir.path())? == [kept.clone()]).then_some(()),
    );
    daemon.settle();

    // shots/2026 left the listing with its capture: the window now
    // watches `shots` alone, under a watch of its own.
    std::fs::rename(&kept, dir.path().join("kept.png")).unwrap();
    daemon.until("the library to drop the capture moved out of shots", || {
        stored(dir.path())?.is_empty().then_some(())
    });
}

#[test]
fn the_arrow_keys_and_return_run_a_context_menu_row() {
    if !enabled() {
        return;
    }
    let case = "the_arrow_keys_and_return_run_a_context_menu_row";
    // A screen the library window fits, so its pixels read back.
    let Some((_xvfb, display)) = xvfb_sized(case, "1600x1200x24") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let one = png(&dir.path().join("shots").join("one.png"));
    store(dir.path(), &[one]);
    daemon.send(&["--library"]);
    let library = daemon.until("the library window to map", || {
        windows_of(&conn, root, LIBRARY).first().copied()
    });
    let [r, g, b, _] = CAPTURE;
    let (left, top, right, bottom) = daemon.until("the card's thumbnail to draw", || {
        extent(&client(&conn, root, library), [r, g, b])
    });
    daemon.settle();
    let at = conn
        .translate_coordinates(library, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let card = (
        at.dst_x + ((left + right) / 2) as i16,
        at.dst_y + ((top + bottom) / 2) as i16,
    );
    click_with(&conn, root, card, 3);
    daemon.settle();
    // The menu opens at the pointer; the pointer leaves it, so only the
    // keys light a row.
    move_to(&conn, root, (at.dst_x + 4, at.dst_y + 4));
    daemon.settle();
    // Down from nothing lights Open, the first row.
    tap(&conn, root, DOWN);
    daemon.settle();
    tap(&conn, root, RETURN);
    daemon.until("Down and Return to run Open", || {
        (!windows_of(&conn, root, "one.png - iris").is_empty()).then_some(())
    });
}
