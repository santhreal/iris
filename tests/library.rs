//! WHY: a capture that another program deletes, or moves out of its
//! folder, leaves an open library window. The window watches the folders
//! that hold its captures, and the read of the store that a change
//! starts drops the capture from `library.json`. The classes closed here
//! are a removal the window reads only once something else makes it
//! read the store, a removal in a folder past the first, and a watch
//! that ends when the window's folders change. Each removal lands once
//! the daemon is idle, after the read that follows the watch's start.
//!
//! The second case closes "an open context menu takes no keys": with
//! the menu open, the arrows and Return did nothing. It opens a card's
//! menu, presses Down and Return, and reads the editor that Open
//! starts.
//!
//! The third case closes "Home, End, and the page keys do nothing in
//! the grid": End then Return opens the last capture in the editor.
//!
//! The fourth case closes "the card menu opens only under a pointer":
//! Pin to Screen and Show in Folder had no keyboard path. `Shift+F10`
//! and the Menu key each open the menu, Down to Pin to Screen and
//! Return pin the capture; a menu opened on another card than the
//! selected one runs Open on the wrong capture.
//!
//! The fifth case closes "the shortcut sheet overflows a small window
//! and passes the pointer through": at the library's 640x480 minimum
//! the sheet's top and bottom rows fell outside the window, and a wheel
//! over the dim layer scrolled the grid under it. The card now ends
//! inside the window, the wheel over the dim layer moves nothing, and
//! the wheel over the card scrolls its rows.
//!
//! Not covered: a read on a timer, which `tests/idle.rs` rejects; the
//! watches of Wayland, Windows, and macOS, which the tests of
//! `src/bin/iris/sys/watch.rs` cover; the menu's other rows, whose keys
//! `widgets::menu_key` runs as Open's. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need `Xvfb`; without it they print
//! that they did not run.

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
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt as _};

use daemon::{enabled, store, stored, xvfb, xvfb_sized, Daemon};
use x11::{chord, click_with, client, extent, move_to, tap, windows_of};

/// The title of the library window.
const LIBRARY: &str = "Library - iris";
/// The test captures' one color, [r, g, b, a].
const CAPTURE: [u8; 4] = [40, 90, 200, 255];
/// Keysyms.
const DOWN: u32 = 0xff54;
const RETURN: u32 = 0xff0d;
const END: u32 = 0xff57;
const HOME: u32 = 0xff50;
const SHIFT: u32 = 0xffe1;
const F10: u32 = 0xffc7;
const MENU: u32 = 0xff67;
const SLASH: u32 = 0x2f;
/// The wheel's down button.
const WHEEL_DOWN: u8 = 5;
/// The shortcut sheet's card, theme::BG_ELEV, as [b, g, r].
const SHEET: [u8; 3] = [43, 43, 43];
/// The title of a pinned window.
const PIN: &str = "Pin - iris";

/// A capture at `path`, its folder made.
fn png(path: &Path) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbaImage::from_pixel(64, 48, image::Rgba(CAPTURE))
        .save(path)
        .unwrap();
    path.to_path_buf()
}

#[test]
fn a_capture_removed_outside_iris_leaves_the_open_library() {
    if !enabled() {
        return;
    }
    let case = "a_capture_removed_outside_iris_leaves_the_open_library";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let shots = dir.path().join("shots");
    let kept = png(&shots.join("kept.png"));
    // `shots/2026` sorts after `shots`: the second folder watched.
    let deeper = png(&shots.join("2026").join("deeper.png"));
    store(dir.path(), &[deeper.clone(), kept.clone()]);
    daemon.send(&["--library"]);
    daemon.until("the library window to map", || {
        (windows_of(&conn, root, LIBRARY).len() == 1).then_some(())
    });
    daemon.settle();

    std::fs::remove_file(&deeper).unwrap();
    daemon.until(
        "the library to drop the capture deleted from shots/2026",
        || (stored(dir.path())? == [kept.clone()]).then_some(()),
    );
    daemon.settle();

    // shots/2026 left the listing with its capture: the window now
    // watches `shots` alone, under a watch of its own.
    std::fs::rename(&kept, dir.path().join("kept.png")).unwrap();
    daemon.until("the library to drop the capture moved out of shots", || {
        stored(dir.path())?.is_empty().then_some(())
    });
}

#[test]
fn the_arrow_keys_and_return_run_a_context_menu_row() {
    if !enabled() {
        return;
    }
    let case = "the_arrow_keys_and_return_run_a_context_menu_row";
    // A screen the library window fits, so its pixels read back.
    let Some((_xvfb, display)) = xvfb_sized(case, "1600x1200x24") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let one = png(&dir.path().join("shots").join("one.png"));
    store(dir.path(), &[one]);
    daemon.send(&["--library"]);
    let library = daemon.until("the library window to map", || {
        windows_of(&conn, root, LIBRARY).first().copied()
    });
    let [r, g, b, _] = CAPTURE;
    let (left, top, right, bottom) = daemon.until("the card's thumbnail to draw", || {
        extent(&client(&conn, root, library), [r, g, b])
    });
    daemon.settle();
    let at = conn
        .translate_coordinates(library, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let card = (
        at.dst_x + ((left + right) / 2) as i16,
        at.dst_y + ((top + bottom) / 2) as i16,
    );
    click_with(&conn, root, card, 3);
    daemon.settle();
    // The menu opens at the pointer; the pointer leaves it, so only the
    // keys light a row.
    move_to(&conn, root, (at.dst_x + 4, at.dst_y + 4));
    daemon.settle();
    // Down from nothing lights Open, the first row.
    tap(&conn, root, DOWN);
    daemon.settle();
    tap(&conn, root, RETURN);
    daemon.until("Down and Return to run Open", || {
        (!windows_of(&conn, root, "one.png - iris").is_empty()).then_some(())
    });
}

#[test]
fn end_and_return_open_the_last_capture() {
    if !enabled() {
        return;
    }
    let case = "end_and_return_open_the_last_capture";
    let Some((_xvfb, display)) = xvfb_sized(case, "1600x1200x24") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let shots = dir.path().join("shots");
    let listed: Vec<_> = ["first.png", "middle.png", "last.png"]
        .iter()
        .map(|name| png(&shots.join(name)))
        .collect();
    store(dir.path(), &listed);
    daemon.send(&["--library"]);
    let library = daemon.until("the library window to map", || {
        windows_of(&conn, root, LIBRARY).first().copied()
    });
    let [r, g, b, _] = CAPTURE;
    daemon.until("the cards' thumbnails to draw", || {
        extent(&client(&conn, root, library), [r, g, b])
    });
    daemon.settle();
    let at = conn
        .translate_coordinates(library, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    // With no window manager the keyboard follows the pointer.
    move_to(&conn, root, (at.dst_x + 4, at.dst_y + 4));
    daemon.settle();
    tap(&conn, root, END);
    daemon.settle();
    tap(&conn, root, RETURN);
    daemon.until("End and Return to open the last capture", || {
        (!windows_of(&conn, root, "last.png - iris").is_empty()).then_some(())
    });
    for other in ["first.png - iris", "middle.png - iris"] {
        assert!(windows_of(&conn, root, other).is_empty(), "{other} opened");
    }
}

#[test]
fn shift_f10_and_the_menu_key_open_the_selected_cards_menu() {
    if !enabled() {
        return;
    }
    let case = "shift_f10_and_the_menu_key_open_the_selected_cards_menu";
    let Some((_xvfb, display)) = xvfb_sized(case, "1600x1200x24") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let shots = dir.path().join("shots");
    let listed: Vec<_> = ["first.png", "middle.png", "last.png"]
        .iter()
        .map(|name| png(&shots.join(name)))
        .collect();
    store(dir.path(), &listed);
    daemon.send(&["--library"]);
    let library = daemon.until("the library window to map", || {
        windows_of(&conn, root, LIBRARY).first().copied()
    });
    let [r, g, b, _] = CAPTURE;
    daemon.until("the cards' thumbnails to draw", || {
        extent(&client(&conn, root, library), [r, g, b])
    });
    daemon.settle();
    let at = conn
        .translate_coordinates(library, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    // With no window manager the keyboard follows the pointer, which
    // rests on the library away from the cards and the pins.
    move_to(&conn, root, (at.dst_x + 4, at.dst_y + 4));
    daemon.settle();
    // Open, Quick Look, Pin to Screen: Down from nothing lights Open.
    let pin_from_menu = |opener: &[u32], pins: usize, what: &str| {
        chord(&conn, root, opener);
        daemon.settle();
        for _ in 0..3 {
            tap(&conn, root, DOWN);
        }
        daemon.settle();
        tap(&conn, root, RETURN);
        daemon.until(what, || {
            (windows_of(&conn, root, PIN).len() == pins).then_some(())
        });
    };
    tap(&conn, root, HOME);
    daemon.settle();
    pin_from_menu(&[SHIFT, F10], 1, "Shift+F10, Down, and Return to pin");
    tap(&conn, root, END);
    daemon.settle();
    pin_from_menu(&[MENU], 2, "the Menu key, Down, and Return to pin");
    // The menu opens on the selected card: Open runs on the last
    // capture, not the first.
    chord(&conn, root, &[SHIFT, F10]);
    daemon.settle();
    tap(&conn, root, DOWN);
    daemon.settle();
    tap(&conn, root, RETURN);
    daemon.until("the menu's Open to open the selected capture", || {
        (!windows_of(&conn, root, "last.png - iris").is_empty()).then_some(())
    });
    for other in ["first.png - iris", "middle.png - iris"] {
        assert!(windows_of(&conn, root, other).is_empty(), "{other} opened");
    }
}

#[test]
fn the_shortcut_sheet_fits_a_minimum_window_and_holds_the_wheel() {
    if !enabled() {
        return;
    }
    let case = "the_shortcut_sheet_fits_a_minimum_window_and_holds_the_wheel";
    let Some((_xvfb, display)) = xvfb_sized(case, "1600x1200x24") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    // Enough cards that the grid scrolls in a 640x480 window.
    let shots = dir.path().join("shots");
    let listed: Vec<_> = (0..12)
        .map(|i| png(&shots.join(format!("shot-{i}.png"))))
        .collect();
    store(dir.path(), &listed);
    daemon.send(&["--library"]);
    let library = daemon.until("the library window to map", || {
        windows_of(&conn, root, LIBRARY).first().copied()
    });
    conn.configure_window(library, &ConfigureWindowAux::new().width(640).height(480))
        .unwrap();
    conn.flush().unwrap();
    let [r, g, b, _] = CAPTURE;
    daemon.until("the 640x480 grid to draw", || {
        let shot = client(&conn, root, library);
        (shot.0 == 640 && shot.1 == 480)
            .then(|| extent(&shot, [r, g, b]))
            .flatten()
    });
    daemon.settle();
    let at = conn
        .translate_coordinates(library, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    // With no window manager the keyboard follows the pointer.
    move_to(&conn, root, (at.dst_x + 6, at.dst_y + 240));
    daemon.settle();
    tap(&conn, root, SLASH);
    // The dim layer covers every thumbnail.
    daemon.until("the shortcut sheet to open", || {
        extent(&client(&conn, root, library), [r, g, b])
            .is_none()
            .then_some(())
    });
    daemon.settle();
    let (w, h, px) = client(&conn, root, library);
    // The card's left edge: the first run of 8 card pixels on the
    // middle row, which no dimmed label under the layer draws.
    let row = &px[h / 2 * w..(h / 2 + 1) * w];
    let card_left = (0..w - 8)
        .find(|&x| row[x..x + 8].iter().all(|&p| p == SHEET))
        .expect("the sheet's card on the middle row");
    for y in [2, h - 3] {
        assert_ne!(
            px[y * w + w / 2],
            SHEET,
            "the sheet's card reaches the window edge at y {y} of {h}"
        );
    }
    // A wheel over the dim layer moves nothing under it.
    let beside = (at.dst_x + (card_left / 2) as i16, at.dst_y + (h / 2) as i16);
    move_to(&conn, root, beside);
    daemon.settle();
    let before = client(&conn, root, library).2;
    for _ in 0..5 {
        click_with(&conn, root, beside, WHEEL_DOWN);
    }
    daemon.settle();
    let after = client(&conn, root, library).2;
    assert!(
        before == after,
        "a wheel over the dim layer moved the window"
    );
    // A wheel over the card scrolls its rows and nothing beside it.
    let over = (at.dst_x + (w / 2) as i16, at.dst_y + (h / 2) as i16);
    move_to(&conn, root, over);
    daemon.settle();
    for _ in 0..5 {
        click_with(&conn, root, over, WHEEL_DOWN);
    }
    daemon.settle();
    let scrolled = client(&conn, root, library).2;
    let column = |px: &[[u8; 3]], x: usize| (0..h).map(|y| px[y * w + x]).collect::<Vec<_>>();
    assert_ne!(
        column(&scrolled, w / 2),
        column(&after, w / 2),
        "the wheel over the card left its rows in place"
    );
    for x in 0..card_left.saturating_sub(2) {
        assert_eq!(
            column(&scrolled, x),
            column(&after, x),
            "the wheel over the card moved the window beside it at x {x}"
        );
    }
}
