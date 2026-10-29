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
//! The second case closes "an open pop-up takes no keys" and "a key and
//! the pointer each hold a highlight": Down and Return, then Up and
//! Space, pick options; a key darkens the row under the pointer, and a
//! move of the pointer within its row takes the highlight back.
//!
//! Not covered: a menu that the window edge pushes off its button; a
//! pop-up with no selected row, which opens under its button. The
//! editor's toolbar menus take keys in the history cases. The cases run
//! only with `IRIS_X11_TEST_DISPLAY` set and need Xvfb; without them
//! they print that they did not run.

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
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, xvfb_sized, Daemon, Server};
use x11::{chord, click, client, control_left_of, move_to, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
const SETTINGS: &str = "Settings - iris";
/// Keysyms.
const TWO: u32 = 0x32;
const CONTROL: u32 = 0xffe3;
const UP: u32 = 0xff52;
const DOWN: u32 = 0xff54;
const RETURN: u32 = 0xff0d;
const SPACE: u32 = 0x20;
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

/// Settings on the Capture pane with the Dismiss after button found.
struct Rig {
    daemon: Daemon,
    conn: RustConnection,
    root: u32,
    settings: u32,
    scale: f32,
    /// The button's center on the screen and in the window.
    button: (i16, i16),
    local: (usize, usize),
    dir: tempfile::TempDir,
    _xvfb: Server,
}

impl Rig {
    fn open(case: &str) -> Option<Rig> {
        if !enabled() {
            return None;
        }
        let (xvfb, display) = xvfb_sized(case, SCREEN)?;
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        assert_eq!(
            dismiss_ms(dir.path()),
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
        // The button: the control left of the card's right padding on
        // the row's center line.
        let y = (DISMISS_Y * scale) as usize;
        let (left, right) = {
            let (w, _, px) = client(&conn, root, settings);
            let row = &px[y * w..(y + 1) * w];
            control_left_of(row, row[w / 2], w - (INSET * scale) as usize - 1)
                .expect("the Dismiss after row has a button")
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
        Some(Rig {
            button: (at.dst_x + ((left + right) / 2) as i16, at.dst_y + y as i16),
            local: ((left + right) / 2, y),
            daemon,
            conn,
            root,
            settings,
            scale,
            dir,
            _xvfb: xvfb,
        })
    }

    /// Whether the menu row `rows` below the button's row is lit.
    fn lit_at(&self, rows: i32) -> bool {
        let (w, _, px) = client(&self.conn, self.root, self.settings);
        let y = self.local.1 as i32 + rows * (ROW_H * self.scale) as i32;
        lit((self.local.0, y as usize), w, &px)
    }

    /// Press the button and wait for its menu, its selected row lit
    /// over the button.
    fn open_menu(&self, what: &str) {
        click(&self.conn, self.root, self.button);
        self.daemon.until(what, || self.lit_at(0).then_some(()));
    }

    fn wait_for_ms(&self, what: &str, ms: u32) {
        self.daemon.until(what, || {
            (dismiss_ms(self.dir.path()) == Some(ms)).then_some(())
        });
    }
}

#[test]
fn a_pop_up_opens_with_its_selected_row_over_its_button() {
    let Some(rig) = Rig::open("a_pop_up_opens_with_its_selected_row_over_its_button") else {
        return;
    };
    // A press on the button opens its menu with a row under the pointer.
    rig.open_menu("the menu to open under the pointer");
    // A press where the button was picks the selected row: no change.
    click(&rig.conn, rig.root, rig.button);
    rig.daemon
        .until("the menu to close", || (!rig.lit_at(0)).then_some(()));
    rig.daemon.settle();
    assert_eq!(
        dismiss_ms(rig.dir.path()),
        Some(5_000),
        "a press where the button was picked another option than the selected one"
    );
    // One row lower is the next option, 8 seconds.
    rig.open_menu("the menu to open again");
    let next = (rig.button.0, rig.button.1 + (ROW_H * rig.scale) as i16);
    click(&rig.conn, rig.root, next);
    rig.wait_for_ms("the next option to be written", 8_000);
}

#[test]
fn the_arrow_keys_move_a_pop_ups_highlight_and_return_picks_it() {
    let Some(rig) = Rig::open("the_arrow_keys_move_a_pop_ups_highlight_and_return_picks_it") else {
        return;
    };
    rig.open_menu("the menu to open under the pointer");
    // The pointer rests on the selected row, moved within it as a hand
    // on the mouse moves it.
    move_to(&rig.conn, rig.root, (rig.button.0 + 1, rig.button.1));
    rig.daemon.settle();
    // Down lights the next row, and the row under the pointer goes
    // dark: one row is lit at a time.
    tap(&rig.conn, rig.root, DOWN);
    rig.daemon
        .until("Down to light the next row", || rig.lit_at(1).then_some(()));
    rig.daemon.settle();
    assert!(
        !rig.lit_at(0),
        "the row under the pointer stays lit beside the row Down lit"
    );
    tap(&rig.conn, rig.root, RETURN);
    rig.wait_for_ms("Return to pick 8 seconds", 8_000);
    // 8 seconds is now selected and opens over the button.
    rig.open_menu("the menu to open again");
    move_to(&rig.conn, rig.root, (rig.button.0 - 1, rig.button.1));
    rig.daemon.settle();
    tap(&rig.conn, rig.root, UP);
    tap(&rig.conn, rig.root, UP);
    rig.daemon.until("Up twice to light the row two above", || {
        rig.lit_at(-2).then_some(())
    });
    rig.daemon.settle();
    assert!(
        !rig.lit_at(0),
        "the row under the pointer stays lit beside the row Up lit"
    );
    // A move of the pointer within its row takes the highlight back.
    move_to(&rig.conn, rig.root, (rig.button.0 + 2, rig.button.1));
    rig.daemon.until("the pointer's move to light its row", || {
        rig.lit_at(0).then_some(())
    });
    rig.daemon.settle();
    assert!(
        !rig.lit_at(-2),
        "the row Up lit stays lit beside the row under the moved pointer"
    );
    // One row up from the pointer's row is 5 seconds.
    tap(&rig.conn, rig.root, UP);
    rig.daemon
        .until("Up to light the row above", || rig.lit_at(-1).then_some(()));
    tap(&rig.conn, rig.root, SPACE);
    rig.wait_for_ms("Space to pick 5 seconds", 5_000);
}
