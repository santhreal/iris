//! The toast card's outline, driven on a private Xvfb with no window
//! manager and no compositing manager.
//!
//! WHY: the class closed here is "a card with no edge on a desktop of
//! its own colour". The capture fills the card to its edge, and the
//! shadow falls mostly below the card, so a capture of a light page over
//! a light desktop, or of a dark one over a dark desktop, showed a card
//! with no edge. A one device pixel outline of mid-gray at 24% runs
//! inside the card's edge. Each case paints the desktop one colour,
//! shows the toast of a capture of that colour, and reads the card's
//! outermost pixel and the pixel inside it at the middle of each side:
//! the outermost one is the capture blended toward mid-gray, darker on
//! white and lighter on black, and the inner one is the capture.
//!
//! Not covered: the outline's corners; the capture flight's outline;
//! a 2x display, where the outline is the same one device pixel;
//! Wayland, Windows, and macOS. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need Xvfb; without them a case
//! returns at once.

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
use x11::{channels, move_to, paint_root, pixel, toast_card, windows_of};

const TOAST: &str = "Screenshot - iris";

/// The 175x140 card of a 1280x1024 capture's toast.
const CARD: (i16, i16) = (175, 140);

/// How far a channel may stray from the blend gpui draws.
const SLACK: u32 = 4;

/// The outline's colour, mid-gray, and its opacity: theme::CARD_EDGE.
const EDGE_GRAY: f32 = 127.5;
const EDGE_ALPHA: f32 = 0.24;

/// Show the toast of a 1280x1024 capture of `rgb` on a desktop of
/// `rgb`, and read each side's outermost pixel and the one inside it.
fn outline(case: &str, rgb: u32) {
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, "1280x1024x24") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, rgb);
    move_to(&conn, root, (10, 10));
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let [r, g, b] = channels(rgb).map(|c| c as u8);
    let shot = dir.path().join("plain.png");
    image::RgbImage::from_pixel(1280, 1024, image::Rgb([r, g, b]))
        .save(&shot)
        .unwrap();
    daemon.send(&[std::ffi::OsStr::new("--toast"), shot.as_os_str()]);
    let window = daemon.until("the toast to open", || {
        windows_of(&conn, root, TOAST).first().copied()
    });
    daemon.settle();
    let card = toast_card(&conn, root, window, CARD);
    let (w, h) = CARD;
    let blend = |c: u32| (c as f32 + (EDGE_GRAY - c as f32) * EDGE_ALPHA).round() as u32;
    let edge = channels(rgb).map(blend);
    // Each side's outermost pixel at its middle, and the pixel one in.
    let sides = [
        ("top", (w / 2, 0), (w / 2, 1)),
        ("bottom", (w / 2, h - 1), (w / 2, h - 2)),
        ("left", (0, h / 2), (1, h / 2)),
        ("right", (w - 1, h / 2), (w - 2, h / 2)),
    ];
    for (side, outer, inner) in sides {
        let read =
            |spot: (i16, i16)| channels(pixel(&conn, root, (card.0 + spot.0, card.1 + spot.1)));
        let (outer, inner) = (read(outer), read(inner));
        assert!(
            outer.iter().zip(edge).all(|(&o, e)| o.abs_diff(e) <= SLACK),
            "{case}: the card's {side} edge shows {outer:?}, not the outline {edge:?} \
             over the capture {rgb:06x}"
        );
        assert!(
            inner
                .iter()
                .zip(channels(rgb))
                .all(|(&i, c)| i.abs_diff(c) <= SLACK),
            "{case}: the pixel inside the card's {side} edge shows {inner:?}, not the \
             capture {rgb:06x}: the outline is wider than one pixel"
        );
    }
}

#[test]
fn a_white_capture_on_a_white_desktop_has_a_darker_edge() {
    outline("toast edge: white", 0xffffff);
}

#[test]
fn a_black_capture_on_a_black_desktop_has_a_lighter_edge() {
    outline("toast edge: black", 0x000000);
}
