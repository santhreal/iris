//! iris's own pop-ups and the screen a capture reads, driven on a
//! private Xvfb with no window manager and read back from the saved
//! capture and the overlay's frozen frame.
//!
//! WHY: one class closes here, "a surface iris opens is part of the
//! picture it takes", for the pop-ups that outlive the command that
//! opened them. A capture taken while the toast of the last capture, or
//! a notice, rested in its corner held that card: two full-screen
//! captures in a row saved the first one's toast in the second, and a
//! region capture froze the toast into the frame it selects from. The
//! cases rest a toast or a notice in the corner, capture, and read the
//! desktop where the card stood. `tests/home.rs` covers Home and
//! `tests/delay.rs` the countdown card.
//!
//! Each case runs with no compositing manager, where a pop-up closes
//! at once, and under picom fading windows out over two seconds, where
//! a pop-up must draw itself empty before it closes.
//!
//! Not covered: a pinned capture window and the recording chip, which
//! are content a capture may hold; a window capture, which needs a
//! window manager's active window; Wayland, whose portal captures with
//! its own interface, Windows, and macOS. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need Xvfb; without it a case prints
//! that it did not run.

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
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, entries, fading_compositor, xvfb_sized, Daemon};
use x11::{paint_root, pixel, rect, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;
const TOAST: &str = "Screenshot - iris";
const NOTICE: &str = "Notice - iris";
const OVERLAY: &str = "Capture - iris";

/// A private Xvfb with the root painted DESKTOP, picom on it when
/// `compositor`, and a daemon. Fields drop in order: the daemon first.
struct Session {
    daemon: Daemon,
    dir: tempfile::TempDir,
    conn: RustConnection,
    root: u32,
    _picom: Option<daemon::Server>,
    _xvfb: daemon::Server,
}

fn session(case: &str, compositor: bool) -> Option<Session> {
    let (xvfb, display) = xvfb_sized(case, SCREEN)?;
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let picom = if compositor {
        Some(fading_compositor(case, &conn, &display)?)
    } else {
        None
    };
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    Some(Session {
        daemon,
        dir,
        conn,
        root,
        _picom: picom,
        _xvfb: xvfb,
    })
}

/// The captures listed in the store of `dir`, newest first.
fn captures(dir: &Path) -> Vec<PathBuf> {
    entries(dir)
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.path)
        .collect()
}

/// Wait for the pop-up titled `title` to open and rest, and return its
/// window rect: x, y, width, height.
fn resting(conn: &RustConnection, root: u32, daemon: &Daemon, title: &str) -> (i16, i16, u16, u16) {
    let window = daemon.until(&format!("the {title:?} window to open"), || {
        windows_of(conn, root, title).first().copied()
    });
    daemon.settle();
    let at = rect(conn, root, window);
    let (x, y, w, h) = at;
    let centre = (x + w as i16 / 2, y + h as i16 / 2);
    assert_ne!(
        pixel(conn, root, centre),
        DESKTOP,
        "the {title:?} card does not show at {centre:?}"
    );
    at
}

/// The pixels of the saved capture `shot` in `at` unlike the desktop:
/// the count and the first.
fn card_in(shot: &Path, at: (i16, i16, u16, u16)) -> (usize, Option<(i16, i16, [u8; 3])>) {
    let image = image::open(shot).unwrap().to_rgb8();
    let desktop = [(DESKTOP >> 16) as u8, (DESKTOP >> 8) as u8, DESKTOP as u8];
    let (x, y, w, h) = at;
    let mut shown = Vec::new();
    for py in y..y + h as i16 {
        for px in x..x + w as i16 {
            let p = image.get_pixel(px as u32, py as u32).0;
            if p != desktop {
                shown.push((px, py, p));
            }
        }
    }
    (shown.len(), shown.first().copied())
}

/// Capture every display and return the new capture once it reads whole.
fn capture_fullscreen(daemon: &Daemon, dir: &Path) -> PathBuf {
    let before = captures(dir).len();
    daemon.send(&["--capture-fullscreen"]);
    let shot = daemon.until("the capture to land", || {
        let listed = captures(dir);
        (listed.len() > before).then(|| listed[0].clone())
    });
    daemon.until("the capture to read whole", || {
        image::open(&shot).ok().map(drop)
    });
    shot
}

/// Show the toast for a red image, whose card shows unlike the desktop.
fn red_toast(daemon: &Daemon, dir: &Path) {
    let red = dir.join("red.png");
    image::RgbImage::from_pixel(320, 200, image::Rgb([220, 30, 30]))
        .save(&red)
        .unwrap();
    daemon.send(&[std::ffi::OsStr::new("--toast"), red.as_os_str()]);
}

#[test]
fn a_full_screen_capture_leaves_out_a_resting_toast() {
    let case = "a_full_screen_capture_leaves_out_a_resting_toast";
    if !enabled() {
        return;
    }
    for compositor in [false, true] {
        let Some(s) = session(case, compositor) else {
            return;
        };
        red_toast(&s.daemon, s.dir.path());
        let toast = resting(&s.conn, s.root, &s.daemon, TOAST);
        let shot = capture_fullscreen(&s.daemon, s.dir.path());
        let (count, first) = card_in(&shot, toast);
        assert_eq!(
            count, 0,
            "compositor {compositor}: the capture holds {count} pixels of the toast at \
             {toast:?}, first {first:?}"
        );
    }
}

#[test]
fn a_full_screen_capture_leaves_out_a_resting_notice() {
    let case = "a_full_screen_capture_leaves_out_a_resting_notice";
    if !enabled() {
        return;
    }
    for compositor in [false, true] {
        let Some(s) = session(case, compositor) else {
            return;
        };
        // A toast that cannot open reports why in a notice, which stays
        // the eight seconds a failure stays.
        let broken = s.dir.path().join("broken.png");
        std::fs::write(&broken, "not an image").unwrap();
        s.daemon
            .send(&[std::ffi::OsStr::new("--toast"), broken.as_os_str()]);
        let notice = resting(&s.conn, s.root, &s.daemon, NOTICE);
        let shot = capture_fullscreen(&s.daemon, s.dir.path());
        let (count, first) = card_in(&shot, notice);
        assert_eq!(
            count, 0,
            "compositor {compositor}: the capture holds {count} pixels of the notice at \
             {notice:?}, first {first:?}"
        );
    }
}

#[test]
fn a_region_capture_freezes_the_screen_without_a_resting_toast() {
    let case = "a_region_capture_freezes_the_screen_without_a_resting_toast";
    if !enabled() {
        return;
    }
    for compositor in [false, true] {
        let Some(s) = session(case, compositor) else {
            return;
        };
        let (conn, root, daemon) = (&s.conn, s.root, &s.daemon);
        red_toast(daemon, s.dir.path());
        let (x, y, w, h) = resting(conn, root, daemon, TOAST);
        daemon.send(&["--capture"]);
        daemon.until("the overlay to take the focus", || {
            windows_of(conn, root, OVERLAY)
                .contains(&x11::focus(conn))
                .then_some(())
        });
        daemon.until("the frozen frame to land", || {
            daemon.log().contains("frame landed").then_some(())
        });
        daemon.settle();
        // The dimmed frozen frame: the desktop beside the toast's
        // window, and the same where its card stood.
        let beside = pixel(conn, root, (x - 40, y + h as i16 / 2));
        let centre = (x + w as i16 / 2, y + h as i16 / 2);
        let under = pixel(conn, root, centre);
        assert_ne!(
            beside, DESKTOP,
            "compositor {compositor}: the overlay did not dim the frozen frame"
        );
        assert_eq!(
            under, beside,
            "compositor {compositor}: the frozen frame shows {under:06x} where the toast \
             stood at {centre:?}, not the dimmed desktop {beside:06x}"
        );
    }
}
