//! The toast's hover actions, driven on a private Xvfb with no window
//! manager and no compositing manager, where the actions show at once.
//!
//! WHY: the class closed here is "a hover action that does not run what
//! it shows". The actions are a scrim over the capture with Copy and
//! Markup at the card's center and a round button in each corner, and a
//! layout change can move a button off its handler. Each case hovers the
//! card of a full-screen capture, reads the button's light fill and the
//! scrim from the screen, clicks the button, and reads its effect: Copy
//! puts the capture's PNG on the clipboard, Markup opens the editor, and
//! Close takes the card off while the pointer, which holds the card's
//! countdown, rests on it. The geometry below mirrors `stage::Card::of`
//! and `ToastAction::rect`: a button placed elsewhere makes the reads and
//! the clicks miss.
//!
//! Not covered: the fade, which only a compositing manager shows; the
//! Keep Open, Show in Folder, and Move to Trash buttons; Wayland,
//! Windows, and macOS. The cases run only with `IRIS_X11_TEST_DISPLAY`
//! set and need Xvfb; without it a case prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::time::{Duration, Instant};

use x11rb::connection::Connection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{click, move_to, paint_root, pixel, rect, windows_of};

/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;
const TOAST: &str = "Screenshot - iris";

/// The toast's card sits BLEED in from its window's top-left corner
/// (theme::CARD_BLEED).
const BLEED: i16 = 44;

/// Spots on the 175x140 card of a 1280x1024 capture, from its top-left
/// corner: the centers of Copy and Markup, two 68x24 buttons 6 apart
/// centered on the card, and a spot on each button's fill left of its
/// label; the center of Close, a 22 px disc 6 in from the top-left
/// corner, and a spot on its fill left of its glyph; and a spot on the
/// scrim clear of every button.
const COPY: (i16, i16) = (51, 70);
const COPY_FILL: (i16, i16) = (23, 70);
const MARKUP: (i16, i16) = (125, 70);
const MARKUP_FILL: (i16, i16) = (97, 70);
const CLOSE: (i16, i16) = (17, 17);
const CLOSE_FILL: (i16, i16) = (9, 17);
const SCRIM: (i16, i16) = (87, 104);

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

fn at(origin: (i16, i16), spot: (i16, i16)) -> (i16, i16) {
    (origin.0 + spot.0, origin.1 + spot.1)
}

fn channels(rgb: u32) -> [u32; 3] {
    [(rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff]
}

/// Capture the screen with the pointer off the toast's corner, and
/// return the toast's window and its card's top-left corner on the
/// screen once the card rests there.
fn toast(s: &Session) -> (u32, (i16, i16)) {
    move_to(&s.conn, s.root, (10, 10));
    s.daemon.send(&["--capture-fullscreen"]);
    let window = s.daemon.until("the toast to open", || {
        windows_of(&s.conn, s.root, TOAST).first().copied()
    });
    s.daemon.settle();
    let (x, y, _, _) = rect(&s.conn, s.root, window);
    (window, (x + BLEED, y + BLEED))
}

/// Rest the pointer on the card's scrim and wait for the button whose
/// fill is at `spot` to show; the scrim darkens the capture under it.
fn hover(s: &Session, card: (i16, i16), spot: (i16, i16), name: &str) {
    let rest = channels(pixel(&s.conn, s.root, at(card, SCRIM)));
    move_to(&s.conn, s.root, at(card, SCRIM));
    s.daemon.until(&format!("the {name} button to show"), || {
        channels(pixel(&s.conn, s.root, at(card, spot)))
            .iter()
            .all(|&c| c >= 0xe0)
            .then_some(())
    });
    let dimmed = channels(pixel(&s.conn, s.root, at(card, SCRIM)));
    assert!(
        dimmed.iter().zip(rest).all(|(&d, r)| d * 10 < r * 8),
        "the hover actions show with no scrim over the capture: \
         {dimmed:?}, at rest {rest:?}"
    );
}

#[test]
fn copy_puts_the_capture_on_the_clipboard_and_close_takes_the_card_off() {
    let Some(s) = session("toast actions: copy, close") else {
        return;
    };
    let (window, card) = toast(&s);
    hover(&s, card, COPY_FILL, "Copy");
    click(&s.conn, s.root, at(card, COPY));
    let png = s.daemon.until("Copy to put a PNG on the clipboard", || {
        x11::clipboard(&s.conn, s.root, b"image/png")
    });
    let copied = image::load_from_memory(&png).unwrap();
    assert_eq!(
        (copied.width(), copied.height()),
        (1280, 1024),
        "Copy put another image on the clipboard"
    );

    // The status line holds the actions' place for a moment after
    // Copy, and a click there is a click on the card, which opens the
    // editor. The pointer rests on the card, which holds its countdown:
    // only Close takes it off.
    std::thread::sleep(APART);
    s.daemon.until("the Close button to show again", || {
        channels(pixel(&s.conn, s.root, at(card, CLOSE_FILL)))
            .iter()
            .all(|&c| c >= 0xe0)
            .then_some(())
    });
    let clicked = Instant::now();
    click(&s.conn, s.root, at(card, CLOSE));
    s.daemon.until("Close to take the card off", || {
        (!windows_of(&s.conn, s.root, TOAST).contains(&window)).then_some(())
    });
    assert!(
        clicked.elapsed() < Duration::from_secs(3),
        "the card stayed {:?} after Close",
        clicked.elapsed()
    );
    s.daemon.settle();
    let editor = format!("{} - iris", capture_name(&s));
    assert!(
        windows_of(&s.conn, s.root, &editor).is_empty(),
        "Close opened the editor"
    );
}

/// The file name of the newest capture.
fn capture_name(s: &Session) -> String {
    s.daemon.until("the capture to land in its folder", || {
        let entries = daemon::entries(s.dir.path())?;
        let path = &entries.first()?.path;
        Some(path.file_name()?.to_string_lossy().into_owned())
    })
}

#[test]
fn markup_opens_the_editor_on_the_capture() {
    let Some(s) = session("toast actions: markup") else {
        return;
    };
    let (_, card) = toast(&s);
    let name = capture_name(&s);
    hover(&s, card, MARKUP_FILL, "Markup");
    click(&s.conn, s.root, at(card, MARKUP));
    let editor = format!("{name} - iris");
    s.daemon.until("Markup to open the editor", || {
        (!windows_of(&s.conn, s.root, &editor).is_empty()).then_some(())
    });
}
