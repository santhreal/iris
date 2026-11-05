//! The notice card's buttons and poster, driven on a private Xvfb with
//! no window manager and read back from the screen.
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
//! The third case closes "a button's name drawn across the card's
//! edge". The notice's window is the card's size and its margin, so a
//! tooltip at the pointer found room only across the card's top or
//! bottom edge. The case rests the pointer on Close and reads, past a
//! tooltip's 500 ms wait, every pixel the name lightens: all on the
//! buttons' row, some left of the buttons, none on Install. Not covered
//! by it: a title long enough to reach the buttons, which the name then
//! covers.
//!
//! The fourth case closes the same class on a saved recording's notice,
//! whose poster named itself "Open" in a tooltip across the card's
//! bottom edge; with no compositing manager the window is cut to the
//! card, and the tooltip was cut in half on the card's hairline. It
//! records a window, stops, and rests the pointer on the poster and
//! then on Show in Folder: no pixel on the card's hairline or outside
//! it changes. Not covered by it: a notice stacked beyond a toast,
//! whose window holds the margin on both sides.
//!
//! Not covered: Install's click, which downloads; Record while a
//! recording runs, the library's other button on the accent; an
//! accent-filled control added elsewhere, which only a case of its own
//! reads; Wayland, Windows, and macOS. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need Xvfb, and the fourth ffmpeg;
//! without them a case prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;
// The other test files use the rest of the recording harness.
#[allow(dead_code)]
#[path = "support/recording.rs"]
mod recording;

use std::path::Path;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, CreateWindowAux, WindowClass};

use daemon::{enabled, xvfb_sized, Daemon};
use recording::Recording;
use x11::{
    all_windows_of, move_to, paint_root, press_and_move, rect, release, until_color, windows_of,
};

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
/// Close's centre: the last 24px button, inside the card's border and
/// padding.
const CLOSE: (i16, i16) = (44 + 380 - 13 - 12, 44 + 13 + 12);
/// The buttons' row in the notice window: inside the card's top border
/// and padding, one 24px button tall.
const ROW: std::ops::Range<usize> = 57..81;
/// Install's box across the row, the leftmost button.
const INSTALL_X: std::ops::Range<usize> = (INSTALL.0 as usize - 12)..(INSTALL.0 as usize + 12);
/// The inside of the card's 1px hairline in a bottom-corner notice's
/// window, whose card is BLEED (44) in from the window's left and top
/// and MARGIN (12) in from its right and bottom.
const INSIDE_X: std::ops::Range<usize> = 44 + 1..44 + 380 - 1;
const INSIDE_TOP: usize = 44 + 1;
const INSIDE_BOTTOM_MARGIN: usize = 12 + 1;
/// A saved notice's poster centre: 67 by 42, inside the card's border
/// and padding. Its buttons are Show in Folder then Close.
const POSTER: (i16, i16) = (44 + 13 + 33, 44 + 13 + 21);
const REVEAL: (i16, i16) = (CLOSE.0 - 24 - 4, CLOSE.1);
/// A spot on a saved notice's title, which answers no pointer.
const SAVED_TITLE: (i16, i16) = (44 + 13 + 67 + 10 + 40, 44 + 13 + 10);

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

/// The pixels lighter in `after` than in `before` by more than a fade's
/// noise, as indices: text and hairlines that show. A shadow only
/// darkens.
fn lightened(before: &[[u8; 3]], after: &[[u8; 3]]) -> Vec<usize> {
    let level = |p: [u8; 3]| p.iter().map(|&c| c as i32).max().unwrap_or(0);
    (0..before.len())
        .filter(|&i| level(after[i]) > level(before[i]) + 12)
        .collect()
}

/// The pixels that differ between `before` and `after` by more than a
/// redraw's noise, as (x, y).
fn changed(before: &[[u8; 3]], after: &[[u8; 3]], w: usize) -> Vec<(usize, usize)> {
    (0..before.len())
        .filter(|&i| {
            before[i]
                .iter()
                .zip(after[i])
                .any(|(a, b)| a.abs_diff(b) > 6)
        })
        .map(|i| (i % w, i / w))
        .collect()
}

// WHY: the class closed here is "a hover name draws over its
// neighbours": the name sits NAME_GAP from the other buttons and above
// the detail line, and a float shadow's blur dimmed both. Lighter pixels
// place the name; any change past its row's contact band, or on
// Install, is the name drawing on something else. It does not cover a
// toast's corner names, which use the same widgets::hover_name.
#[test]
fn a_buttons_name_shows_on_its_row_clear_of_the_buttons() {
    let Some(s) = session("notice: names") else {
        return;
    };
    // Past the overlay warmup's shell over the screen.
    std::thread::sleep(Duration::from_secs(1));
    s.until(INSTALL_FILL, ACCENT, "Install on the accent at rest");
    let (w, _, before) = x11::client(&s.conn, s.root, s.notice);
    move_to(&s.conn, s.root, s.at(CLOSE));
    // Past the 500 ms a tooltip waits for, with time to draw.
    std::thread::sleep(Duration::from_millis(900));
    s.daemon.settle();
    let (_, _, after) = x11::client(&s.conn, s.root, s.notice);
    let lit = lightened(&before, &after);
    let off_row: Vec<(usize, usize)> = lit
        .iter()
        .map(|&i| (i % w, i / w))
        .filter(|(_, y)| !ROW.contains(y))
        .collect();
    assert!(
        off_row.is_empty(),
        "Close's name shows off the buttons' row, at {:?}",
        &off_row[..off_row.len().min(5)]
    );
    let left_of_buttons = lit.iter().filter(|&&i| i % w < INSTALL_X.start).count();
    assert!(
        left_of_buttons > 20,
        "Close's name does not show left of the buttons"
    );
    let on_install = lit
        .iter()
        .filter(|&&i| INSTALL_X.contains(&(i % w)))
        .count();
    assert_eq!(on_install, 0, "Close's name covers Install");
    let below = ROW.end + 4;
    let spill: Vec<(usize, usize)> = changed(&before, &after, w)
        .into_iter()
        .filter(|&(x, y)| !(ROW.start - 2..below).contains(&y) || INSTALL_X.contains(&x))
        .collect();
    assert!(
        spill.is_empty(),
        "Close's name changed Install or the card past its row, at {:?}",
        &spill[..spill.len().min(5)]
    );
}

#[test]
fn a_hover_on_a_saved_notice_draws_inside_its_card() {
    let Some(rec) = Recording::start("notice: saved") else {
        return;
    };
    let (conn, root) = rec.x();
    // The pick's guide is a notice too, and under load it can still be
    // unmapped, or leaving, when the stop is sent: the saved notice is
    // one that did not exist, mapped or not, before the stop.
    let earlier = all_windows_of(conn, root, NOTICE);
    rec.daemon.send(&["--record-window"]);
    let notice = rec.daemon.until("the saved notice to open", || {
        windows_of(conn, root, NOTICE)
            .into_iter()
            .find(|w| !earlier.contains(w))
    });
    let (x, y, _, _) = rect(conn, root, notice);
    let at = |(dx, dy): (i16, i16)| (x + dx, y + dy);
    // The pointer on the card holds it past its five seconds.
    move_to(conn, root, at(SAVED_TITLE));
    // The poster fades in once ffmpeg decodes it: a frame of the
    // recorded window's blue, red, or green over the slot's grey.
    rec.daemon.until("the poster to show", || {
        let (w, _, shown) = x11::client(conn, root, notice);
        let p = shown[(POSTER.1 as usize - 14) * w + POSTER.0 as usize - 26];
        let (hi, lo) = (p.iter().max().unwrap(), p.iter().min().unwrap());
        (hi - lo > 40).then_some(())
    });
    std::thread::sleep(Duration::from_millis(300));
    rec.daemon.settle();
    for (name, spot) in [("the poster", POSTER), ("Show in Folder", REVEAL)] {
        let (w, h, before) = x11::client(conn, root, notice);
        move_to(conn, root, at(spot));
        // Past the 500 ms a tooltip waits for, with time to draw.
        std::thread::sleep(Duration::from_millis(900));
        rec.daemon.settle();
        let (_, _, after) = x11::client(conn, root, notice);
        let inside_y = INSIDE_TOP..h - INSIDE_BOTTOM_MARGIN;
        let outside: Vec<(usize, usize)> = (0..before.len())
            .filter(|&i| {
                let (a, b) = (before[i], after[i]);
                a.iter().zip(b).any(|(a, b)| a.abs_diff(b) > 12)
            })
            .map(|i| (i % w, i / w))
            .filter(|(px, py)| !INSIDE_X.contains(px) || !inside_y.contains(py))
            .collect();
        assert!(
            outside.is_empty(),
            "a hover on {name} drew on the card's edge or outside it, at {:?}",
            &outside[..outside.len().min(5)]
        );
    }
}
