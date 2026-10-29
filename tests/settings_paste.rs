//! A paste into a Settings text field, read from the config it writes.
//!
//! WHY: the class closed here is "a text entry takes keys but not the
//! clipboard", on the Settings side; `paste.rs` reads the editor's side.
//! Both entries edit their text through one function, whose unit tests
//! check each key exactly. A Settings field appended typed characters
//! and dropped Ctrl+V, so a folder path or a template could only be
//! typed. The case puts two lines on the clipboard, opens the file name
//! field, pastes, commits with Return, and reads the template as one
//! line in config.toml.
//!
//! Not covered: Cmd+V on macOS, which reaches the same key handler. The
//! case runs only with `IRIS_X11_TEST_DISPLAY` set and needs Xvfb;
//! without them it prints that it did not run.

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
use x11::{chord, click, client, control_left_of, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
const SETTINGS: &str = "Settings - iris";
/// Keysyms.
const V: u32 = 0x76;
const RETURN: u32 = 0xff0d;
const CONTROL: u32 = 0xffe3;
/// The Settings window's width, logical px.
const WIDTH: f32 = 600.0;
/// The center of the File name row on the General pane, logical px:
/// under the toolbar (52), the inset (20), the login card (36), the
/// group gap (20), the Storage header (20), and two rows of the storage
/// card with their hairlines (2 x 36 + 2), half a row down.
const TEMPLATE_Y: f32 = 52.0 + 20.0 + 36.0 + 20.0 + 20.0 + 74.0 + 18.0;
/// The card's inset from the window's right edge, logical px.
const INSET: f32 = 20.0;
/// The file name field's width, logical px.
const FIELD_W: f32 = 200.0;
/// The clipboard: a tab and two lines, the second ending in a line
/// break; and the template the field holds before and after the paste.
const CLIPBOARD: &str = "\tteam\nnotes\n";
const BEFORE: &str = "{date}_{time}";
const AFTER: &str = "{date}_{time} team notes";

/// The file name template in `dir`'s config, or `None` while the config
/// cannot be read whole, as mid-write.
fn template(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("config").join("config.toml")).ok()?;
    let table: toml::Table = text.parse().ok()?;
    Some(match table.get("screenshot_template") {
        Some(v) => v.as_str()?.to_string(),
        None => iris_lib::config::Config::default().screenshot_template,
    })
}

/// The accent of a focus ring, as [b, g, r].
fn accent(p: [u8; 3]) -> bool {
    p[0] > 200 && p[2] < 60
}

#[test]
fn a_paste_into_a_settings_field_writes_the_clipboard_as_one_line() {
    let case = "a_paste_into_a_settings_field_writes_the_clipboard_as_one_line";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    // The clipboard is this process's, on the case's X server. This is
    // the file's one case, so no other case reads the environment while
    // it changes.
    std::env::set_var("DISPLAY", &display);
    std::env::remove_var("WAYLAND_DISPLAY");
    iris_lib::clipboard::set_text(CLIPBOARD).unwrap();
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    assert_eq!(template(dir.path()).as_deref(), Some(BEFORE));
    daemon.send(&["--settings"]);
    let settings = daemon.until("Settings to open", || {
        windows_of(&conn, root, SETTINGS).first().copied()
    });
    daemon.settle();
    // The field: the control left of the card's right padding on the
    // row's center line.
    let (w, _, px) = client(&conn, root, settings);
    let scale = w as f32 / WIDTH;
    let y = (TEMPLATE_Y * scale) as usize;
    let row = &px[y * w..(y + 1) * w];
    let card = row[w / 2 - (40.0 * scale) as usize];
    let (left, right) = control_left_of(row, card, w - (INSET * scale) as usize - 1)
        .expect("the File name row has a field");
    assert!(
        (right - left) as f32 >= FIELD_W * scale - 4.0,
        "the File name field spans {left}..={right}: it is {FIELD_W} px wide"
    );
    let at = conn
        .translate_coordinates(settings, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    click(
        &conn,
        root,
        (at.dst_x + ((left + right) / 2) as i16, at.dst_y + y as i16),
    );
    // The open field draws an accent border and focus ring.
    let ring = (y - (20.0 * scale) as usize, y + (20.0 * scale) as usize);
    daemon.until("the field to open for typing", || {
        let (w, _, px) = client(&conn, root, settings);
        (ring.0..ring.1)
            .flat_map(|y| (left..=right).map(move |x| (x, y)))
            .any(|(x, y)| accent(px[y * w + x]))
            .then_some(())
    });
    chord(&conn, root, &[CONTROL, V]);
    tap(&conn, root, RETURN);
    daemon.settle();
    assert_eq!(
        template(dir.path()).as_deref(),
        Some(AFTER),
        "the field committed without the pasted text as one line"
    );
}
