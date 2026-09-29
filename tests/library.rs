//! WHY: a capture that another program deletes, or moves out of its
//! folder, leaves an open library window. The window watches the folders
//! that hold its captures, and the read of the store that a change
//! starts drops the capture from `library.json`. The classes closed here
//! are a removal the window reads only once something else makes it
//! read the store, a removal in a folder past the first, and a watch
//! that ends when the window's folders change. Each removal lands once
//! the daemon is idle, after the read that follows the watch's start.
//!
//! Not covered: a read on a timer, which `tests/idle.rs` rejects; the
//! watches of Wayland, Windows, and macOS, which the tests of
//! `src/bin/iris/sys/watch.rs` cover. The case runs only with
//! `IRIS_X11_TEST_DISPLAY` set and needs `Xvfb`; without it the case
//! prints that it did not run.

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

use daemon::{enabled, store, stored, xvfb, Daemon};
use x11::windows_of;

/// The title of the library window.
const LIBRARY: &str = "Library - iris";

/// A capture at `path`, its folder made.
fn png(path: &Path) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbaImage::from_pixel(64, 48, image::Rgba([40, 90, 200, 255]))
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
