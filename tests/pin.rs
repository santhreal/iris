//! WHY: a pin opened at one fixed corner whatever it showed, and a
//! second pin opened on the same pixel as the first and hid it. A pin
//! opens centered on the screen rect its capture came from, so a pin at
//! native scale covers those pixels exactly, and a pin that would open
//! on another pin's origin steps down and right. The case pins two
//! region captures of one rect through the toast's context menu. Not
//! covered: a pin scaled down to its 720x540 cap, a window capture's
//! pin, a pin with no rect (a toast forwarded with `--toast`), which
//! `src/bin/iris/pin/tests.rs` places; a window manager that places
//! windows itself; Wayland, where the compositor places every window.
//! The case runs only with `IRIS_X11_TEST_DISPLAY` set and needs `Xvfb`;
//! without it the case prints that it did not run.

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

use daemon::{enabled, entries, xvfb_sized, Daemon};
use x11::{click, click_with, drag, paint_root, rect, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;
const TOAST: &str = "Screenshot - iris";
const OVERLAY: &str = "Capture - iris";
const PIN: &str = "Pin - iris";
const RETURN: u32 = 0xff0d;

/// The selection both captures drag, corner to corner.
const FROM: (i16, i16) = (200, 200);
const TO: (i16, i16) = (520, 440);

/// The toast's card sits BLEED in from its window's top-left corner.
/// Its context menu is MENU_W wide, MENU_PAD above its first row, rows
/// MENU_ROW_H tall, ROWS of them, and stays 2 px inside the window
/// (theme::CARD_BLEED, widgets::menu). Pin to screen is row PIN_ROW.
const BLEED: i16 = 44;
const MENU_W: i16 = 168;
const MENU_PAD: i16 = 5;
const MENU_ROW_H: i16 = 24;
const ROWS: i16 = 6;
const PIN_ROW: i16 = 3;

/// The pins' step down and right (pin::CASCADE).
const CASCADE: i16 = 24;

#[test]
fn a_pin_opens_over_its_capture_and_a_second_one_cascades() {
    let case = "a_pin_opens_over_its_capture_and_a_second_one_cascades";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });

    let mut pins: Vec<u32> = Vec::new();
    for n in 0..2i16 {
        let size = capture(&conn, root, &daemon, dir.path(), n as usize + 1);
        pin_from_toast(&conn, root, &daemon);
        let pin = daemon.until(&format!("pin {n} to open"), || {
            windows_of(&conn, root, PIN)
                .into_iter()
                .find(|w| !pins.contains(w))
        });
        daemon.settle();
        let (x, y, w, h) = rect(&conn, root, pin);
        let want = (FROM.0 + n * CASCADE, FROM.1 + n * CASCADE);
        assert_eq!(
            ((x, y), (w, h)),
            (want, size),
            "pin {n} opened at {:?}, {}x{}, not at {want:?}, {}x{}",
            (x, y),
            w,
            h,
            size.0,
            size.1
        );
        pins.push(pin);
    }
}

/// Drag FROM to TO on a capture overlay and commit it, wait for the
/// `count`th capture to land, and return its pixel size.
fn capture(
    conn: &RustConnection,
    root: u32,
    daemon: &Daemon,
    dir: &std::path::Path,
    count: usize,
) -> (u16, u16) {
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
    drag(conn, root, FROM, TO, 40);
    tap(conn, root, RETURN);
    let path = daemon.until("the capture to land in its folder", || {
        let entries = entries(dir)?;
        (entries.len() >= count).then(|| entries[0].path.clone())
    });
    let (w, h) = image::image_dimensions(&path).unwrap();
    (w as u16, h as u16)
}

/// Open the resting toast's context menu on its card and click Pin to
/// screen.
fn pin_from_toast(conn: &RustConnection, root: u32, daemon: &Daemon) {
    let toast = daemon.until("the toast to open", || {
        windows_of(conn, root, TOAST).first().copied()
    });
    daemon.settle();
    let (x, y, w, h) = rect(conn, root, toast);
    let press = (BLEED + 20, BLEED + 20);
    click_with(conn, root, (x + press.0, y + press.1), 3);
    daemon.settle();
    let menu_h = ROWS * MENU_ROW_H + 2 * MENU_PAD;
    let menu = (
        press.0.min(w as i16 - MENU_W - 2).max(2),
        press.1.min(h as i16 - menu_h - 2).max(2),
    );
    click(
        conn,
        root,
        (
            x + menu.0 + MENU_W / 2,
            y + menu.1 + MENU_PAD + PIN_ROW * MENU_ROW_H + MENU_ROW_H / 2,
        ),
    );
}
