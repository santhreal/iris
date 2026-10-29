//! A Settings pop-up button's menu, read from the config a pick writes.
//!
//! WHY: the class closed here is "a floating menu placed from an anchor
//! it does not have". A menu's offset assumed its anchor at its button's
//! top-left, but the anchor followed the button in its parent's flow,
//! at the button's bottom-left, so every pop-up opened one control
//! height low: the row under the pointer was the option after the
//! selected one, and a press and release on the button picked it. The
//! case opens the Dismiss after pop-up, presses where its button was,
//! and reads the value unchanged; then opens it again, presses one row
//! lower, and reads the next option. The editor's toolbar menus share
//! the placement and are read by the history cases.
//!
//! Not covered: a menu that the window edge pushes off its button; a
//! pop-up with no selected row, which opens under its button. The case
//! runs only with `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without
//! them it prints that it did not run.

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

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{chord, click, client, windows_of};

const SCREEN: &str = "1280x1024x24";
const SETTINGS: &str = "Settings - iris";
/// Keysyms.
const TWO: u32 = 0x32;
const CONTROL: u32 = 0xffe3;
/// The Settings window's width and the Capture pane's height, logical
/// px: the toolbar (52), the insets (2 x 20), a card of three rows (3 x
/// 36 + 2 hairlines), the group gap (20), and the Floating Thumbnail
/// header (20) over a card of six rows (6 x 36 + 5 hairlines).
const WIDTH: f32 = 600.0;
const CAPTURE_H: f32 = 463.0;
/// The center of the Dismiss after row, logical px: under the toolbar,
/// the inset, the first card, the gap, the header, and two rows of the
/// second card with their hairlines.
const DISMISS_Y: f32 = 52.0 + 20.0 + 110.0 + 20.0 + 20.0 + 74.0 + 18.0;
/// The card's inset from the window's right edge, logical px.
const INSET: f32 = 20.0;
/// A menu row's height, logical px.
const ROW_H: f32 = 24.0;
/// A pop-up button's least width, logical px.
const BUTTON_W: f32 = 110.0;

/// The toast's dismiss delay in `dir`'s config, ms, or `None` while the
/// config cannot be read whole, as mid-write.
fn dismiss_ms(dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(dir.join("config").join("config.toml")).ok()?;
    let table: toml::Table = text.parse().ok()?;
    Some(match table.get("toast_duration_ms") {
        Some(v) => u32::try_from(v.as_integer()?).ok()?,
        None => iris_lib::config::Config::default().toast_duration_ms,
    })
}

/// The accent fill of the menu row under the pointer, as [b, g, r].
fn accent(p: [u8; 3]) -> bool {
    p[0] > 200 && p[2] < 60
}

/// Whether a menu row is lit under `p`: most of 60 px of the line 9 px
/// above it, over the row's label ink, are accent fill.
fn lit(p: (usize, usize), w: usize, px: &[[u8; 3]]) -> bool {
    (p.0 - 30..p.0 + 30)
        .filter(|&x| accent(px[(p.1 - 9) * w + x]))
        .count()
        >= 50
}

#[test]
fn a_pop_up_opens_with_its_selected_row_over_its_button() {
    let case = "a_pop_up_opens_with_its_selected_row_over_its_button";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let initial = dismiss_ms(dir.path());
    assert_eq!(
        initial,
        Some(5_000),
        "the default dismiss delay is 5 seconds"
    );
    daemon.send(&["--settings"]);
    let settings = daemon.until("Settings to open", || {
        windows_of(&conn, root, SETTINGS).first().copied()
    });
    daemon.settle();
    chord(&conn, root, &[CONTROL, TWO]);
    let scale = daemon.until("the Capture pane to show", || {
        let (w, h, _) = client(&conn, root, settings);
        let scale = w as f32 / WIDTH;
        ((h as f32 - CAPTURE_H * scale).abs() < 1.0).then_some(scale)
    });
    daemon.settle();
    // The button: the run of non-card pixels left of the card's right
    // padding on the row's center line.
    let y = (DISMISS_Y * scale) as usize;
    let (left, right) = {
        let (w, _, px) = client(&conn, root, settings);
        let row = &px[y * w..(y + 1) * w];
        let card = row[w / 2];
        let right = (0..w - (INSET * scale) as usize - 1)
            .rev()
            .find(|&x| row[x] != card)
            .expect("the Dismiss after row has a button");
        let mut left = right;
        let mut gap = 0;
        for x in (0..right).rev() {
            if row[x] == card {
                gap += 1;
                if gap == 8 {
                    break;
                }
            } else {
                gap = 0;
                left = x;
            }
        }
        (left, right)
    };
    assert!(
        (right - left) as f32 >= BUTTON_W * scale - 2.0,
        "the Dismiss after button spans {left}..={right}: a pop-up button is at least \
         {BUTTON_W} px wide"
    );
    let at = conn
        .translate_coordinates(settings, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let button = (at.dst_x + ((left + right) / 2) as i16, at.dst_y + y as i16);
    let local = ((left + right) / 2, y);
    let lit_at = |p: (usize, usize)| {
        let (w, _, px) = client(&conn, root, settings);
        lit(p, w, &px)
    };
    // A press on the button opens its menu with a row under the pointer.
    click(&conn, root, button);
    daemon.until("the menu to open under the pointer", || {
        lit_at(local).then_some(())
    });
    // A press where the button was picks the selected row: no change.
    click(&conn, root, button);
    daemon.until("the menu to close", || (!lit_at(local)).then_some(()));
    daemon.settle();
    assert_eq!(
        dismiss_ms(dir.path()),
        initial,
        "a press where the button was picked another option than the selected one"
    );
    // One row lower is the next option, 8 seconds.
    click(&conn, root, button);
    daemon.until("the menu to open again", || lit_at(local).then_some(()));
    let next = (button.0, button.1 + (ROW_H * scale) as i16);
    click(&conn, root, next);
    daemon.until("the next option to be written", || {
        (dismiss_ms(dir.path()) == Some(8_000)).then_some(())
    });
}
