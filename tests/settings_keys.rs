//! Settings driven from the keyboard, read from the screen and from the
//! config each key writes.
//!
//! WHY: the class closed here is "a setting the keyboard cannot reach".
//! The window took Ctrl+1 through Ctrl+5 and nothing else: Tab moved no
//! focus, Ctrl+Tab changed no pane, and no control showed which one a
//! key would run. The case switches panes with Ctrl+Tab, walks the
//! Capture pane with Tab, flips a switch and picks a pop-up option with
//! Space, Down, and Return, and reads each change from config.toml. It
//! reads the focus ring above the control under it, that an off switch
//! inside the ring keeps its gray track, that the ring moves off the
//! control Tab left, that a pane change clears it, and that a mouse
//! press clears it.
//!
//! Not covered: Space on Choose…, which opens the system folder picker,
//! and on Check Now, which reaches the network; the traversal over every
//! pane, which `settings::tests` walks. The case runs only with
//! `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without it it prints that
//! it did not run.

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

use iris_lib::config::Config;
use x11rb::connection::Connection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{chord, click, client, move_to, rect, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
const SETTINGS: &str = "Settings - iris";
/// Keysyms.
const TAB: u32 = 0xff09;
const SHIFT: u32 = 0xffe1;
const CONTROL: u32 = 0xffe3;
const DOWN: u32 = 0xff54;
const RETURN: u32 = 0xff0d;
const SPACE: u32 = 0x20;
/// The Settings window's width and the General and Capture panes'
/// heights, logical px (see `settings::panes`).
const WIDTH: f32 = 600.0;
const GENERAL_H: f32 = 52.0 + 40.0 + 36.0 + 20.0 + 20.0 + 110.0 + 20.0 + 36.0;
const CAPTURE_H: f32 = 463.0;
/// Row centers on the Capture pane, logical px: Screen flash, the first
/// row under the toolbar and the inset, and Dismiss after, two rows into
/// the Floating Thumbnail card under the first card, the gap, and the
/// header.
const FLASH_Y: f32 = 52.0 + 20.0 + 18.0;
const DISMISS_Y: f32 = 52.0 + 20.0 + 110.0 + 20.0 + 20.0 + 74.0 + 18.0;
/// Half the height of a toggle and of a pop-up button, logical px.
const TOGGLE_HALF: f32 = 10.0;
const BUTTON_HALF: f32 = 12.0;
/// A point on a switch's track right of its knob when off, logical px
/// from the window's right edge: inside the card inset and row padding.
const OFF_TRACK_FROM_RIGHT: f32 = 20.0 + 12.0 + 6.0;

/// The config `dir`'s daemon stored, or `None` while it cannot be read
/// whole, as mid-write.
fn stored(dir: &Path) -> Option<Config> {
    let text = std::fs::read_to_string(dir.join("config").join("config.toml")).ok()?;
    toml::from_str(&text).ok()
}

/// The focus ring's tint: accent at 45% over the row's dark gray, as
/// [b, g, r].
fn ring_tint(p: [u8; 3]) -> bool {
    p[0] > 90 && p[0] as i32 > p[2] as i32 + 50
}

/// The accent fill of a lit menu row, as [b, g, r].
fn accent(p: [u8; 3]) -> bool {
    p[0] > 200 && p[2] < 60
}

struct Rig {
    daemon: Daemon,
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
    settings: u32,
    scale: f32,
}

impl Rig {
    /// Whether the focus ring shows over the control whose row is
    /// centered at `y` and that is `half` tall above its center: a line
    /// of ring tint in the 3px band above the control, in the right half
    /// of the window where every control sits.
    fn ring_at(&self, y: f32, half: f32) -> bool {
        let (w, _, px) = client(&self.conn, self.root, self.settings);
        let s = self.scale;
        let mut rows = ((y - half - 2.5) * s) as usize..((y - half - 0.5) * s) as usize;
        rows.any(|row| {
            (w / 2..w).filter(|&x| ring_tint(px[row * w + x])).count() >= (10.0 * s) as usize
        })
    }

    /// Whether a menu row is lit over the row centered at `y`: most of
    /// 60 px of the line 9 px above its center are accent fill.
    fn menu_lit_at(&self, y: f32) -> bool {
        let (w, _, px) = client(&self.conn, self.root, self.settings);
        let row = ((y - 9.0) * self.scale) as usize;
        (w / 2..w).filter(|&x| accent(px[row * w + x])).count() >= (50.0 * self.scale) as usize
    }

    /// Wait for the window to take the height of a pane `h` tall.
    fn wait_for_pane(&self, what: &str, h: f32) {
        self.daemon.until(what, || {
            let (_, ch, _) = client(&self.conn, self.root, self.settings);
            ((ch as f32 - h * self.scale).abs() < 1.0).then_some(())
        });
        self.daemon.settle();
    }

    fn wait_for_ring(&self, what: &str, y: f32, half: f32) {
        self.daemon
            .until(what, || self.ring_at(y, half).then_some(()));
        self.daemon.settle();
    }
}

#[test]
fn the_keyboard_reaches_and_runs_every_kind_of_control() {
    let case = "the_keyboard_reaches_and_runs_every_kind_of_control";
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
    let defaults = Config::default();
    assert!(
        defaults.flash_on_capture,
        "the screen flash is on by default"
    );
    assert_eq!(defaults.toast_duration_ms, 5_000);
    daemon.send(&["--settings"]);
    let settings = daemon.until("Settings to open", || {
        windows_of(&conn, root, SETTINGS).first().copied()
    });
    daemon.settle();
    let (w, ..) = client(&conn, root, settings);
    let rig = Rig {
        daemon,
        conn,
        root,
        settings,
        scale: w as f32 / WIDTH,
    };
    // The keyboard follows the pointer: rest it in the window's left
    // inset, clear of every control.
    let (x0, y0, ..) = rect(&rig.conn, rig.root, rig.settings);
    let blank = (x0 + 5, y0 + 100);
    move_to(&rig.conn, rig.root, blank);
    rig.daemon.settle();

    // Ctrl+Tab shows the next pane, with no ring on it.
    chord(&rig.conn, rig.root, &[CONTROL, TAB]);
    rig.wait_for_pane("Ctrl+Tab to show the Capture pane", CAPTURE_H);
    assert!(
        !rig.ring_at(FLASH_Y, TOGGLE_HALF),
        "a ring shows before any Tab"
    );

    // Tab rings the first control, and Space flips it.
    tap(&rig.conn, rig.root, TAB);
    rig.wait_for_ring("Tab to ring the Screen flash switch", FLASH_Y, TOGGLE_HALF);
    tap(&rig.conn, rig.root, SPACE);
    rig.daemon.until("Space to turn the screen flash off", || {
        stored(dir.path())
            .filter(|c| !c.flash_on_capture)
            .map(|_| ())
    });
    // The ringed switch reads off: its track, right of the knob, is the
    // dim neutral gray of an off switch, untinted by the ring around it,
    // and not the white knob of an on switch.
    rig.daemon
        .until("the ringed switch's track to read off", || {
            let (w, _, px) = client(&rig.conn, rig.root, rig.settings);
            let x = w - (OFF_TRACK_FROM_RIGHT * rig.scale) as usize;
            let p = px[(FLASH_Y * rig.scale) as usize * w + x];
            (p[0] < 150 && (p[0] as i32 - p[2] as i32).abs() < 8).then_some(())
        });
    assert!(
        rig.ring_at(FLASH_Y, TOGGLE_HALF),
        "the ring left the switch Space flipped"
    );

    // Five more Tabs reach Dismiss after, and the ring leaves the switch.
    for _ in 0..5 {
        tap(&rig.conn, rig.root, TAB);
    }
    rig.wait_for_ring(
        "Tab to ring the Dismiss after button",
        DISMISS_Y,
        BUTTON_HALF,
    );
    assert!(
        !rig.ring_at(FLASH_Y, TOGGLE_HALF),
        "the ring stays on the switch Tab left"
    );

    // Space opens the menu on the selected option; Down and Return pick
    // the next one.
    tap(&rig.conn, rig.root, SPACE);
    rig.daemon.until("Space to open the menu", || {
        rig.menu_lit_at(DISMISS_Y).then_some(())
    });
    rig.daemon.settle();
    tap(&rig.conn, rig.root, DOWN);
    tap(&rig.conn, rig.root, RETURN);
    rig.daemon.until("Down and Return to pick 8 seconds", || {
        stored(dir.path())
            .filter(|c| c.toast_duration_ms == 8_000)
            .map(|_| ())
    });
    rig.wait_for_ring(
        "the ring to return to the closed button",
        DISMISS_Y,
        BUTTON_HALF,
    );

    // Ctrl+Shift+Tab and Ctrl+Tab come back to the pane with no ring.
    chord(&rig.conn, rig.root, &[CONTROL, SHIFT, TAB]);
    rig.wait_for_pane("Ctrl+Shift+Tab to show the General pane", GENERAL_H);
    chord(&rig.conn, rig.root, &[CONTROL, TAB]);
    rig.wait_for_pane("Ctrl+Tab to show the Capture pane again", CAPTURE_H);
    assert!(
        !rig.ring_at(DISMISS_Y, BUTTON_HALF),
        "the ring survives a pane change"
    );

    // A mouse press takes the ring away.
    tap(&rig.conn, rig.root, TAB);
    rig.wait_for_ring("Tab to ring the switch again", FLASH_Y, TOGGLE_HALF);
    click(&rig.conn, rig.root, blank);
    rig.daemon.until("a press to clear the ring", || {
        (!rig.ring_at(FLASH_Y, TOGGLE_HALF)).then_some(())
    });
    assert_eq!(
        stored(dir.path()).map(|c| (c.flash_on_capture, c.toast_duration_ms)),
        Some((false, 8_000)),
        "a press on the window's inset changed a setting"
    );
}
