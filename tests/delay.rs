//! The delayed capture (`iris --delay N`), driven on a private Xvfb and
//! read back from the screen and the saved capture.
//!
//! WHY: four classes close here. "A delay shows nothing until the shot":
//! the capture fired after N seconds with nothing on screen, so a delay
//! read as a command that did nothing. A card counts the seconds down;
//! the first case reads its title change twice. "A surface iris opens is
//! part of the picture it takes": the card stands where the capture
//! reads, so it must leave the screen before the grab; the first case
//! reads the saved capture where the card stood, with no compositing
//! manager and under picom with a two-second fade, which would hold a
//! closed card's last frame on screen. "A cancel that does not cancel":
//! the second case clicks the card's close button and reads that no
//! capture lands. "A repeated delay captures twice": the third case
//! sends a second delay over the first and reads one capture. A zero
//! delay captures once and opens no card.
//!
//! Not covered: the countdown title's rounding, which the notice unit
//! tests read; a card the window manager places away from its corner;
//! Wayland, Windows, and macOS. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need Xvfb, and the first needs picom
//! for its second pass; without them they print that they did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::path::Path;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConnectionExt as _, EventMask, ImageFormat,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, entries, fading_compositor, xvfb_sized, Daemon};
use x11::{click, paint_root, rect, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;
const NOTICE: &str = "Notice - iris";
/// The card in its window, at scale 1: at (BLEED, BLEED) in the default
/// bottom-right corner (theme::CARD_BLEED), 380 wide, and 50 high with
/// a title and no detail (notice::W, notice::card_height(0)).
const CARD: (i16, i16, u16, u16) = (44, 44, 380, 50);
/// The card's window: the card, BLEED on its inner sides, and the 12px
/// margin on its outer ones (notice::place).
const CARD_WINDOW: (u16, u16) = (380 + 44 + 12, 50 + 44 + 12);
/// The close button's centre in the card window: the card's right edge
/// less its 1px border, 12px padding, and half the 24px button.
const CLOSE: (i16, i16) = (44 + 380 - 13 - 12, 44 + 13 + 12);

/// A private Xvfb with the root painted DESKTOP: its server, display
/// name, and connection.
fn screen(case: &str) -> Option<(daemon::Server, String, RustConnection, u32)> {
    let (xvfb, display) = xvfb_sized(case, SCREEN)?;
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    Some((xvfb, display, conn, root))
}

/// The pixels of the card of the notice `window` as the screen shows
/// them.
fn card_pixels(conn: &RustConnection, root: u32, window: u32) -> Option<Vec<u8>> {
    let (x, y, _, _) = rect(conn, root, window);
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            x + CARD.0,
            y + CARD.1,
            CARD.2,
            CARD.3,
            !0,
        )
        .ok()?
        .reply()
        .ok()?;
    Some(image.data)
}

/// The files in the screenshot folder of the daemon of `dir`.
fn shots(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("shots")).unwrap().count()
}

#[test]
fn a_delayed_capture_counts_down_and_leaves_its_card_out() {
    let case = "a_delayed_capture_counts_down_and_leaves_its_card_out";
    if !enabled() {
        return;
    }
    for compositor in [false, true] {
        let Some((_xvfb, display, conn, root)) = screen(case) else {
            return;
        };
        let _picom = if compositor {
            let Some(picom) = fading_compositor(case, &conn, &display) else {
                return;
            };
            Some(picom)
        } else {
            None
        };
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        let sent = Instant::now();
        daemon.send(&["--delay", "3"]);
        let window = daemon.until("the countdown card to open", || {
            windows_of(&conn, root, NOTICE).first().copied()
        });
        let (x, y, w, h) = rect(&conn, root, window);
        assert_eq!((w, h), CARD_WINDOW, "the countdown card's window size");

        // Every new picture of the card until it closes, and when it
        // first showed. With no compositor the card arrives and leaves at
        // once, so a new picture is a new second, or a first frame.
        let mut seen: Vec<(Vec<u8>, Duration)> = Vec::new();
        let closed = loop {
            if windows_of(&conn, root, NOTICE).is_empty() {
                break sent.elapsed();
            }
            assert!(
                sent.elapsed() < Duration::from_secs(10),
                "compositor {compositor}: the countdown card stayed past its deadline; \
                 daemon log:\n{}",
                daemon.log()
            );
            if let Some(pixels) = card_pixels(&conn, root, window) {
                let blank = pixels.chunks(4).all(|p| p == &pixels[..4]);
                if !blank && !seen.iter().any(|(s, _)| s == &pixels) {
                    seen.push((pixels, sent.elapsed()));
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            closed >= Duration::from_secs(3),
            "compositor {compositor}: the card closed {closed:?} after a 3 second delay"
        );
        if !compositor {
            let span = seen.last().map_or(Duration::ZERO, |s| s.1 - seen[0].1);
            assert!(
                seen.len() >= 3 && span >= Duration::from_millis(1500),
                "the card showed {} pictures over {span:?} of a 3 second delay, \
                 not one a second",
                seen.len()
            );
        }

        let shot = daemon.until("the delayed capture to land", || {
            let listed = entries(dir.path())?;
            listed.first().map(|e| e.path.clone())
        });
        let image = daemon.until("the capture to read whole", || {
            Some(image::open(&shot).ok()?.to_rgb8())
        });
        let desktop = [(DESKTOP >> 16) as u8, (DESKTOP >> 8) as u8, DESKTOP as u8];
        let mut shown = Vec::new();
        for py in y..y + h as i16 {
            for px in x..x + w as i16 {
                let p = image.get_pixel(px as u32, py as u32).0;
                if p != desktop {
                    shown.push((px, py, p));
                }
            }
        }
        assert!(
            shown.is_empty(),
            "compositor {compositor}: the capture holds {} pixels of the card window \
             ({x}, {y}, {w}x{h}), first {:?}",
            shown.len(),
            shown.first()
        );
    }
}

#[test]
fn the_close_button_cancels_a_delayed_capture() {
    let case = "the_close_button_cancels_a_delayed_capture";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display, conn, root)) = screen(case) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let sent = Instant::now();
    daemon.send(&["--delay", "2"]);
    let window = daemon.until("the countdown card to open", || {
        windows_of(&conn, root, NOTICE).first().copied()
    });
    daemon.until("the card to draw", || {
        let pixels = card_pixels(&conn, root, window)?;
        (!pixels.chunks(4).all(|p| p == &pixels[..4])).then_some(())
    });
    let (x, y, _, _) = rect(&conn, root, window);
    click(&conn, root, (x + CLOSE.0, y + CLOSE.1));
    daemon.until("the card to close", || {
        windows_of(&conn, root, NOTICE).is_empty().then_some(())
    });
    let closed = sent.elapsed();
    assert!(
        closed < Duration::from_secs(2),
        "the card closed {closed:?} after the send, not at the click"
    );
    std::thread::sleep(Duration::from_millis(3500).saturating_sub(sent.elapsed()));
    daemon.settle();
    assert_eq!(
        shots(dir.path()),
        0,
        "a cancelled delay captured; daemon log:\n{}",
        daemon.log()
    );
}

#[test]
fn a_newer_delay_replaces_the_pending_one() {
    let case = "a_newer_delay_replaces_the_pending_one";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display, conn, root)) = screen(case) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let sent = Instant::now();
    daemon.send(&["--delay", "3"]);
    daemon.until("the first countdown card to open", || {
        windows_of(&conn, root, NOTICE).first().copied()
    });
    daemon.send(&["--delay", "1"]);
    daemon.until("the second delay's capture to land", || {
        (shots(dir.path()) > 0).then_some(())
    });
    let landed = sent.elapsed();
    assert!(
        landed < Duration::from_secs(3),
        "the capture landed {landed:?} after the first send, at the first delay's deadline"
    );
    std::thread::sleep(Duration::from_millis(4500).saturating_sub(sent.elapsed()));
    daemon.settle();
    assert_eq!(
        shots(dir.path()),
        1,
        "two delays captured more than once; daemon log:\n{}",
        daemon.log()
    );
}

#[test]
fn a_zero_delay_captures_at_once_with_no_card() {
    let case = "a_zero_delay_captures_at_once_with_no_card";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display, conn, root)) = screen(case) else {
        return;
    };
    // Every window the daemon creates, reported to this connection.
    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
    )
    .unwrap();
    conn.flush().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&["--delay", "0"]);
    daemon.until("the capture to land", || {
        (shots(dir.path()) > 0).then_some(())
    });
    daemon.settle();
    let mut cards = 0;
    while let Some(event) = conn.poll_for_event().unwrap() {
        if let Event::CreateNotify(created) = event {
            cards += usize::from((created.width, created.height) == CARD_WINDOW);
        }
    }
    assert_eq!(cards, 0, "a zero delay opened a countdown card");
    assert_eq!(shots(dir.path()), 1, "a zero delay did not capture once");
}
