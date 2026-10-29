//! The capture overlay's keys during a drag, driven on a private Xvfb
//! with no window manager.
//!
//! WHY: the class closed here is "a key during a drag does its work
//! with nothing on screen stating it, or the next motion undoes it".
//! `c` during a drag copied the loupe's center color to the clipboard
//! and left the loupe as it was, so the copy read as a key the overlay
//! ignored. The first case holds a drag over a desktop of one known
//! color, presses `c`, and reads the color's hex on the clipboard and a
//! changed banner under the loupe. Space held during a drag moves the
//! selection at its size, and an arrow key during a drag moves it for
//! good: before, every pointer motion rebuilt the rect from the press
//! point. The second case commits two drags and reads each capture's
//! size: one moved with Space and an arrow key, then resized, and one
//! moved into the screen's corner and released with Space still held.
//!
//! Not covered: the banner's wording, which needs text recognition the
//! test hosts lack; a resize drag, which shares the key handler; Space
//! with Shift's square; Wayland, Windows, and macOS. The cases run only
//! with `IRIS_X11_TEST_DISPLAY` set and need Xvfb; without it they
//! print that they did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::{path::Path, time::Duration};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{
    clipboard_text, glide, hold, let_go, paint_root, press_and_move, release, tap, windows_of,
};

/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;
const OVERLAY: &str = "Capture - iris";
const TOAST: &str = "Screenshot - iris";
/// XK_c.
const C: u32 = 0x63;
/// XK_space.
const SPACE: u32 = 0x20;
/// XK_Right.
const RIGHT: u32 = 0xff53;
/// XK_Return.
const RETURN: u32 = 0xff0d;
/// Where the drag holds the pointer. The loupe opens 24 px right of and
/// below it, 152 px square, and its banner 4 px under that.
const HOLD: (i16, i16) = (400, 300);
/// The strip of the screen the loupe's banner covers: x, y, w, h.
const BANNER: (i16, i16, u16, u16) = (HOLD.0 + 24, HOLD.1 + 24 + 152 + 4, 200, 24);

/// The screen's [b, g, r] pixels in `at`.
fn strip(conn: &impl Connection, root: u32, at: (i16, i16, u16, u16)) -> Vec<[u8; 3]> {
    let image = conn
        .get_image(ImageFormat::Z_PIXMAP, root, at.0, at.1, at.2, at.3, !0)
        .unwrap()
        .reply()
        .unwrap();
    image.data.chunks(4).map(|p| [p[0], p[1], p[2]]).collect()
}

#[test]
fn c_during_a_drag_copies_the_loupe_color_and_states_it() {
    let case = "c_during_a_drag_copies_the_loupe_color_and_states_it";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, "1280x1024x24") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&["--capture"]);
    daemon.until("the overlay to take the focus", || {
        windows_of(&conn, root, OVERLAY)
            .contains(&x11::focus(&conn))
            .then_some(())
    });
    daemon.until("the frozen frame to land", || {
        daemon.log().contains("frame landed").then_some(())
    });
    daemon.settle();

    press_and_move(&conn, root, (200, 200), HOLD, 20);
    // The banner reads the held pixel once two reads 100 ms apart agree
    // and it shows more than the dimmed desktop.
    let mut last = strip(&conn, root, BANNER);
    let shown = daemon.until("the loupe's banner to rest", || {
        std::thread::sleep(Duration::from_millis(100));
        let now = strip(&conn, root, BANNER);
        let rests = now == last && now.iter().any(|p| *p != now[0]);
        last = now.clone();
        rests.then_some(now)
    });

    tap(&conn, root, C);
    let copied = daemon.until("the color on the clipboard", || {
        clipboard_text(&conn, root).filter(|t| t.starts_with('#'))
    });
    assert_eq!(copied, format!("#{DESKTOP:06x}"));
    daemon.until("the banner to state the copy", || {
        (strip(&conn, root, BANNER) != shown).then_some(())
    });
    release(&conn, root);
}

/// Start a capture and wait until its overlay has the focus and the
/// `count`th frozen frame has landed.
fn open_overlay(conn: &impl Connection, root: u32, daemon: &Daemon, count: usize) {
    daemon.send(&["--capture"]);
    daemon.until("the overlay to take the focus", || {
        windows_of(conn, root, OVERLAY)
            .contains(&x11::focus(conn))
            .then_some(())
    });
    daemon.until("the frozen frame to land", || {
        (daemon.log().matches("frame landed").count() >= count).then_some(())
    });
    daemon.settle();
}

/// Commit the overlay's selection, wait for its toast to open and the
/// overlay to park, and return the pixel size of the `count`th capture
/// of the daemon of `dir`.
fn commit(
    conn: &impl Connection,
    root: u32,
    daemon: &Daemon,
    dir: &Path,
    count: usize,
) -> (u32, u32) {
    tap(conn, root, RETURN);
    let path = daemon.until("the capture to land in its folder", || {
        let entries = daemon::entries(dir)?;
        (entries.len() >= count).then(|| entries[0].path.clone())
    });
    daemon.until("the toast to open", || {
        windows_of(conn, root, TOAST).first().copied()
    });
    daemon.settle();
    image::image_dimensions(&path).unwrap()
}

#[test]
fn space_and_the_arrow_keys_move_a_selection_mid_drag() {
    let case = "space_and_the_arrow_keys_move_a_selection_mid_drag";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, "1280x1024x24") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });

    // A 200x100 drag, moved 100, 50 with Space and 1 right with an
    // arrow, then resized to the release point: (301, 250) to (700, 550).
    open_overlay(&conn, root, &daemon, 1);
    press_and_move(&conn, root, (200, 200), (400, 300), 20);
    hold(&conn, root, SPACE);
    glide(&conn, root, (400, 300), (500, 350), 20);
    let_go(&conn, root, SPACE);
    tap(&conn, root, RIGHT);
    glide(&conn, root, (500, 350), (700, 550), 20);
    release(&conn, root);
    assert_eq!(commit(&conn, root, &daemon, dir.path(), 1), (399, 300));

    // The same drag moved past the screen's top-left corner stops at
    // the edges, and released with Space held keeps its size.
    open_overlay(&conn, root, &daemon, 2);
    press_and_move(&conn, root, (200, 200), (400, 300), 20);
    hold(&conn, root, SPACE);
    glide(&conn, root, (400, 300), (0, 0), 40);
    release(&conn, root);
    let_go(&conn, root, SPACE);
    assert_eq!(commit(&conn, root, &daemon, dir.path(), 2), (200, 100));
}
