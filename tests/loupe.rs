//! The capture overlay's loupe color copy, driven on a private Xvfb with
//! no window manager.
//!
//! WHY: the class closed here is "a key does its work with nothing on
//! screen stating it". `c` during a drag copied the loupe's center
//! color to the clipboard and left the loupe as it was, so the copy read
//! as a key the overlay ignored. The case holds a drag over a desktop of
//! one known color, presses `c`, and reads the color's hex on the
//! clipboard and a changed banner under the loupe.
//!
//! Not covered: the banner's wording, which needs text recognition the
//! test hosts lack; a resize drag, which shares the key handler; Wayland,
//! Windows, and macOS. The case runs only with `IRIS_X11_TEST_DISPLAY`
//! set and needs Xvfb; without it the case prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{clipboard_text, paint_root, press_and_move, release, tap, windows_of};

/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;
const OVERLAY: &str = "Capture - iris";
/// XK_c.
const C: u32 = 0x63;
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
