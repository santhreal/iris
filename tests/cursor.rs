//! The cursor iris's controls show, read through XFixes on a private
//! Xvfb with no window manager.
//!
//! WHY: the class closed here is "a control shows another cursor than
//! the arrow". Every button, toggle, menu row, and card showed the hand a
//! web page shows over a link, where a native app's controls show the
//! arrow; and a window that never asked for another cursor set none, so
//! X showed the root's. The cases read the arrow, the cursor theme's
//! `left_ptr`, over a window of their own, then hover controls each built
//! by another constructor and read the arrow over each: the capture
//! bar's mode buttons and its run button, dimmed and runnable, whose
//! overlay shows the crosshair everywhere else; the toast's corner disc
//! and its Copy pill; and the library's toolbar buttons. `tests/pin.rs`
//! reads the pin's surface and close control.
//!
//! Not covered: the menus, the settings controls, the editor's options
//! bar, and the notice's poster; Wayland, where the compositor draws the
//! cursor; Windows and macOS. The cases run only with
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

use x11rb::connection::Connection;
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{cursor, drag, move_to, paint_root, rect, windows_of, CursorImage};

const DESKTOP: u32 = 0x5b7fa6;
const OVERLAY: &str = "Capture - iris";
const TOAST: &str = "Screenshot - iris";
const LIBRARY: &str = "Library - iris";

/// The capture bar on a 1280x1024 monitor (`tests/capture_bar.rs`): its
/// left edge and its buttons' row, and the centers of cancel, entire
/// screen, record selection, and run.
const BAR_LEFT: i16 = 482;
const ROW: i16 = 1024 - 64 - 50 + 25;
const BAR: [(&str, i16); 4] = [
    ("cancel", 25),
    ("entire screen", 74),
    ("record selection", 199),
    ("run", 270),
];
const RUN: (i16, i16) = (BAR_LEFT + 270, ROW);

/// The toast's card sits BLEED in from its window's top-left corner; on
/// it, the centers of the Close disc and the Copy pill
/// (`tests/toast_actions.rs`).
const BLEED: i16 = 44;
const CLOSE: (i16, i16) = (17, 17);
const COPY: (i16, i16) = (51, 70);

/// The library's toolbar buttons' centres, 28 px apart by 8, from the
/// window's top-right corner at scale 1. With no compositing manager the
/// window takes the platform's frame and draws no window controls, so
/// New Screenshot is the trailing button, 12 px in.
const TOOLBAR: [(&str, i16); 3] = [
    ("New Screenshot", 12 + 14),
    ("Record", 12 + 28 + 8 + 14),
    ("Keyboard Shortcuts", 12 + 2 * (28 + 8) + 14),
];
const TOOLBAR_ROW: i16 = 26;

/// A daemon on its own 1280x1024 Xvfb with a one-colour desktop, and the
/// arrow its cursor theme draws.
struct Session {
    daemon: Daemon,
    conn: RustConnection,
    root: u32,
    arrow: CursorImage,
    _dir: tempfile::TempDir,
    _xvfb: daemon::Server,
}

fn session(case: &str) -> Option<Session> {
    if !enabled() {
        return None;
    }
    let (xvfb, display) = xvfb_sized(case, "1280x1024x24")?;
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let arrow = x11::arrow(&conn, screen, (0, 0));
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    Some(Session {
        daemon,
        conn,
        root,
        arrow,
        _dir: dir,
        _xvfb: xvfb,
    })
}

impl Session {
    /// Rest the pointer on `at` and read the cursor X shows there.
    fn cursor_at(&self, at: (i16, i16)) -> CursorImage {
        move_to(&self.conn, self.root, at);
        self.daemon.settle();
        cursor(&self.conn)
    }

    /// Assert the arrow shows over `what` at `at`.
    fn arrow_at(&self, what: &str, at: (i16, i16)) {
        assert!(
            self.cursor_at(at) == self.arrow,
            "{what} at {at:?} shows another cursor than the arrow"
        );
    }
}

#[test]
fn the_capture_bar_shows_the_arrow_over_the_crosshair() {
    let Some(s) = session("the_capture_bar_shows_the_arrow_over_the_crosshair") else {
        return;
    };
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
    // The screen outside the bar selects: the crosshair, not the arrow,
    // so the reads below tell the two apart.
    assert!(
        s.cursor_at((300, 300)) != s.arrow,
        "the overlay shows the arrow over the screen, not the crosshair"
    );
    for (button, x) in BAR {
        s.arrow_at(&format!("the bar's {button} button"), (BAR_LEFT + x, ROW));
    }
    // A selection makes the run button runnable, drawn and hovered as
    // another control than the dimmed one.
    drag(&s.conn, s.root, (200, 200), (600, 500), 20);
    s.daemon.settle();
    s.arrow_at("the bar's runnable run button", RUN);
}

#[test]
fn the_toast_shows_the_arrow_over_its_buttons() {
    let Some(s) = session("the_toast_shows_the_arrow_over_its_buttons") else {
        return;
    };
    s.daemon.send(&["--capture-fullscreen"]);
    let toast = s.daemon.until("the toast to open", || {
        windows_of(&s.conn, s.root, TOAST).first().copied()
    });
    s.daemon.settle();
    let (x, y, _, _) = rect(&s.conn, s.root, toast);
    let card = (x + BLEED, y + BLEED);
    for (button, spot) in [("Close disc", CLOSE), ("Copy pill", COPY)] {
        s.arrow_at(
            &format!("the toast's {button}"),
            (card.0 + spot.0, card.1 + spot.1),
        );
    }
}

#[test]
fn the_library_shows_the_arrow_over_its_toolbar_buttons() {
    let Some(s) = session("the_library_shows_the_arrow_over_its_toolbar_buttons") else {
        return;
    };
    s.daemon.send(&["--library"]);
    let library = s.daemon.until("the library window to map", || {
        windows_of(&s.conn, s.root, LIBRARY).first().copied()
    });
    s.daemon.settle();
    let (x, y, w, _) = rect(&s.conn, s.root, library);
    for (button, from_right) in TOOLBAR {
        s.arrow_at(
            &format!("the library's {button} button"),
            (x + w as i16 - from_right, y + TOOLBAR_ROW),
        );
    }
}
