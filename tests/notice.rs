//! The update notice's Install button, driven on a private Xvfb with no
//! window manager and read back from the screen.
//!
//! WHY: the first case closes "an accent-filled button that still runs
//! shows no hover or press". Install, on the accent, kept its fill under
//! the pointer and while pressed, where Close beside it answers both.
//! Each case seeds `update.json` with a fresh check that found 99.0.0,
//! so the daemon's first update pass announces it with no network call.
//! The first case reads Install's fill at rest, darker under the
//! pointer, darker still while pressed, and at rest again once the held
//! press leaves it; that release installs nothing and the notice stays.
//!
//! The second case closes "a window the pointer leaves and enters again
//! at one spot shows no hover until the pointer moves". A window mapped
//! above another and gone again, as the overlay's startup warmup maps
//! one over the whole screen, sends the window below a leave and an
//! enter with no motion between, and the X11 backend cleared hover on
//! the leave and set none on the enter: the control under a resting
//! pointer read at rest, and the notice, which holds while the pointer
//! rests on it, counted down and left. The case maps a window of its
//! own over the pointer and takes it off, reads Install's hover fill
//! come back with the pointer still, and reads the notice there past
//! its hold. Not covered by it: an enter in a grab mode, which the
//! backend ignores, and Wayland, whose enter already carries a move.
//!
//! Not covered: Install's click, which downloads; Record while a
//! recording runs, the library's other button on the accent; an
//! accent-filled control added elsewhere, which only a case of its own
//! reads; Wayland, Windows, and macOS. The cases run only with
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

use std::path::Path;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, CreateWindowAux, WindowClass};

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{move_to, paint_root, press_and_move, rect, release, until_color, windows_of};

/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;
const NOTICE: &str = "Notice - iris";
/// Install's centre in the notice window at scale 1: the card sits at
/// (BLEED, BLEED) in the default bottom-right corner (theme::CARD_BLEED
/// 44) and is 380 wide; inside its 1px border and 12px padding, Close
/// is the last 24px button and Install the one 4px before it.
const INSTALL: (i16, i16) = (44 + 380 - 13 - 24 - 4 - 12, 44 + 13 + 12);
/// A spot on Install's fill left of its 14px glyph.
const INSTALL_FILL: (i16, i16) = (INSTALL.0 - 9, INSTALL.1);
/// A spot on the card's title, clear of every button.
const TITLE: (i16, i16) = (200, 44 + 13 + 12);
/// theme::ACCENT, and theme::ACCENT_HOVER and ACCENT_PRESS, the accent
/// at 88% and 74% over theme::BG, as [r, g, b].
const ACCENT: [u32; 3] = [10, 132, 255];
const ACCENT_HOVER: [u32; 3] = [12, 120, 228];
const ACCENT_PRESS: [u32; 3] = [15, 106, 197];

/// The notice's hold, notice::FAILURE_HOLD (8 s), with a second to
/// spare: a notice the pointer does not hold has left by then.
const PAST_HOLD: Duration = Duration::from_secs(9);

/// `update.json` of an iris rooted at `dir` as a check a minute ago
/// left it: 99.0.0 found and not yet announced, its asset on a closed
/// port.
fn seed_offer(dir: &Path) {
    let state = dir.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let asset = "http://127.0.0.1:9/iris-99.0.0-x86_64.AppImage";
    let json = format!(
        r#"{{"format":1,"checked_at":{},"channel":"stable","offer":{{"version":"99.0.0","asset_url":"{asset}","asset_name":"iris-99.0.0-x86_64.AppImage","checksum_url":"{asset}.sha256","signature_url":"{asset}.minisig"}},"announced":null}}"#,
        now - 60
    );
    std::fs::write(state.join("update.json"), json).unwrap();
}

/// A daemon on its own Xvfb with a one-colour desktop, and the update
/// notice it opened.
struct Session {
    _xvfb: daemon::Server,
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
    _dir: tempfile::TempDir,
    daemon: Daemon,
    notice: u32,
    /// The notice window's top-left corner on the screen.
    origin: (i16, i16),
}

impl Session {
    fn at(&self, (dx, dy): (i16, i16)) -> (i16, i16) {
        (self.origin.0 + dx, self.origin.1 + dy)
    }

    fn until(&self, spot: (i16, i16), want: [u32; 3], what: &str) {
        until_color(&self.conn, self.root, self.at(spot), want, what);
    }
}

fn session(case: &str) -> Option<Session> {
    if !enabled() {
        return None;
    }
    let (xvfb, display) = xvfb_sized(case, "1280x1024x24")?;
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    move_to(&conn, root, (10, 10));
    let dir = tempfile::tempdir().unwrap();
    seed_offer(dir.path());
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let notice = daemon.until("the update notice to open", || {
        windows_of(&conn, root, NOTICE).first().copied()
    });
    let (x, y, _, _) = rect(&conn, root, notice);
    Some(Session {
        _xvfb: xvfb,
        conn,
        root,
        _dir: dir,
        daemon,
        notice,
        origin: (x, y),
    })
}

#[test]
fn install_on_the_accent_answers_the_pointer() {
    let Some(s) = session("notice: install") else {
        return;
    };
    s.until(INSTALL_FILL, ACCENT, "Install on the accent at rest");
    move_to(&s.conn, s.root, s.at(INSTALL));
    s.until(
        INSTALL_FILL,
        ACCENT_HOVER,
        "Install under the pointer to take the hover fill",
    );
    press_and_move(&s.conn, s.root, s.at(INSTALL), s.at(INSTALL), 1);
    s.until(
        INSTALL_FILL,
        ACCENT_PRESS,
        "the pressed Install to take the press fill",
    );
    x11::glide(&s.conn, s.root, s.at(INSTALL), s.at(TITLE), 8);
    s.until(
        INSTALL_FILL,
        ACCENT,
        "the press dragged off Install to show it at rest",
    );
    release(&s.conn, s.root);
    s.daemon.settle();
    s.until(
        INSTALL_FILL,
        ACCENT,
        "Install at rest after a release off it",
    );
    assert!(
        windows_of(&s.conn, s.root, NOTICE).contains(&s.notice),
        "a release off Install closed the notice"
    );
}

#[test]
fn hover_returns_after_a_window_above_comes_and_goes() {
    let Some(s) = session("notice: crossing") else {
        return;
    };
    // The overlay warmup maps its shell over the screen 400 ms after the
    // daemon starts; past it, the window below is the only one mapped.
    std::thread::sleep(Duration::from_secs(1));
    move_to(&s.conn, s.root, s.at(INSTALL));
    s.until(
        INSTALL_FILL,
        ACCENT_HOVER,
        "Install under the pointer to take the hover fill",
    );
    let cover = s.conn.generate_id().unwrap();
    s.conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            cover,
            s.root,
            0,
            0,
            1280,
            1024,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(0),
        )
        .unwrap();
    s.conn.map_window(cover).unwrap();
    s.conn.flush().unwrap();
    s.until(INSTALL_FILL, [0, 0, 0], "the cover to map over the notice");
    s.conn.destroy_window(cover).unwrap();
    s.conn.flush().unwrap();
    s.until(
        INSTALL_FILL,
        ACCENT_HOVER,
        "Install under the resting pointer to take the hover fill again",
    );
    std::thread::sleep(PAST_HOLD);
    assert!(
        windows_of(&s.conn, s.root, NOTICE).contains(&s.notice),
        "the notice left under the resting pointer"
    );
}
