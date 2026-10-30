//! A paste, the caret keys, and clicks in a Settings text field, read
//! from the config it writes.
//!
//! WHY: the classes closed here are "a text entry takes keys but not
//! the clipboard", "a text entry edits only at its end", "a click does
//! not place the caret", and "the caret leaves a long value's field",
//! on the Settings side; `paste.rs` and
//! `history.rs` read the editor's side. Both entries edit their text
//! through one function and place the caret through another, whose unit
//! tests check each key and position exactly. A Settings field
//! appended typed characters and dropped Ctrl+V, so a folder path or a
//! template could only be typed; it ignored Left, Home, End, and
//! Delete, so a fix at the start of a value meant deleting it all; a
//! click left the caret at the end of the text wherever it landed; a
//! click in the open field committed the edit and reopened it; and a
//! value longer than the field drew its caret outside it. The case puts
//! two lines on the clipboard, opens the file name field, moves the
//! caret back a word and two characters, pastes, reads the caret bar at
//! the field's start after Home, types at the start and the end,
//! deletes a character, and commits with Return. It then clicks the
//! text's first character to open the field, types, clicks there again
//! with the field open, types, and commits, reading the caret bar after
//! each click and the template from config.toml after the second click
//! and each commit. Last, it types past the field's right edge, reads
//! the caret bar near that edge, clicks the first character in view,
//! types, and commits.
//!
//! Not covered: Cmd+V and the Cmd and Alt caret keys on macOS, which
//! reach the same key handler. The case runs only with
//! `IRIS_X11_TEST_DISPLAY` set and needs Xvfb; without them it prints
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

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{chord, click, client, field_left_of, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
const SETTINGS: &str = "Settings - iris";
/// Keysyms.
const A: u32 = 0x61;
const B: u32 = 0x62;
const V: u32 = 0x76;
const S: u32 = 0x73;
const X: u32 = 0x78;
const Z: u32 = 0x7a;
const LEFT: u32 = 0xff51;
const HOME: u32 = 0xff50;
const END: u32 = 0xff57;
const DELETE: u32 = 0xffff;
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
/// break; and the template the field holds before and after the edit.
const CLIPBOARD: &str = "\tteam\nnotes\n";
const BEFORE: &str = "{date}_{time}";
const AFTER: &str = "s{date} team notes_{time}z";
/// The template after an a typed at a click on its start, and a b at a
/// second click there.
const CLICKED: &str = "bas{date} team notes_{time}z";

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
fn a_paste_and_the_caret_keys_edit_a_settings_field_at_the_caret() {
    let case = "a_paste_and_the_caret_keys_edit_a_settings_field_at_the_caret";
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
    // The field: the text field left of the card's right padding on the
    // row's center line. The file name preview sits left of it; its ink
    // moves with the clock, so the card's fill is read in the padding.
    let (w, _, px) = client(&conn, root, settings);
    let scale = w as f32 / WIDTH;
    let y = (TEMPLATE_Y * scale) as usize;
    let row = &px[y * w..(y + 1) * w];
    let card = row[w - ((INSET + 6.0) * scale) as usize];
    let before = w - (INSET * scale) as usize - 1;
    let (left, right) = field_left_of(row, card, before, (FIELD_W * scale) as usize)
        .unwrap_or_else(|| panic!("the File name row has no field {FIELD_W} px wide"));
    let at = conn
        .translate_coordinates(settings, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    // A click past the end of the text opens the field with the caret
    // after it.
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
    // {date}_{time}| to {date}_{|time}, then {date}|_{time}, where the
    // clipboard goes in as one line.
    chord(&conn, root, &[CONTROL, LEFT]);
    tap(&conn, root, LEFT);
    tap(&conn, root, LEFT);
    chord(&conn, root, &[CONTROL, V]);
    // The caret: the accent column inside the field's border on its
    // center line, at the start once Home lands.
    tap(&conn, root, HOME);
    let start = left + (12.0 * scale) as usize;
    let caret_at_start = |what: &str| {
        daemon.until(what, || {
            let (w, _, px) = client(&conn, root, settings);
            (left + 3..right - 2)
                .find(|&x| accent(px[y * w + x]))
                .filter(|&x| x < start)
        });
    };
    caret_at_start("the caret to be drawn at the field's start");
    // An s at the start, an x after it that Left and Delete remove, and
    // a z at the end.
    for keysym in [S, X, LEFT, DELETE, END, Z, RETURN] {
        tap(&conn, root, keysym);
    }
    daemon.settle();
    assert_eq!(
        template(dir.path()).as_deref(),
        Some(AFTER),
        "the field committed without the edits at the caret"
    );
    // A click on the first character of the text opens the field with
    // the caret before it, and a click there with the field open moves
    // the caret back before the a typed at the start and leaves the
    // edit open, uncommitted.
    let first = (
        at.dst_x + (left + (9.0 * scale) as usize) as i16,
        at.dst_y + y as i16,
    );
    click(&conn, root, first);
    caret_at_start("a click to open the field with the caret at its start");
    tap(&conn, root, A);
    std::thread::sleep(std::time::Duration::from_millis(300));
    click(&conn, root, first);
    caret_at_start("a click in the open field to move the caret to its start");
    daemon.settle();
    assert_eq!(
        template(dir.path()).as_deref(),
        Some(AFTER),
        "a click in the open field committed it"
    );
    for keysym in [B, RETURN] {
        tap(&conn, root, keysym);
    }
    daemon.settle();
    assert_eq!(
        template(dir.path()).as_deref(),
        Some(CLICKED),
        "the field committed without the edits at the clicked caret"
    );
    // Twelve z typed at the end run the text past the field's right
    // edge, and the field moves the text left to keep the caret in
    // view near that edge. A click on the first character in view then
    // puts the caret there, past the start of the text, where an x goes
    // in.
    click(
        &conn,
        root,
        (at.dst_x + ((left + right) / 2) as i16, at.dst_y + y as i16),
    );
    std::thread::sleep(std::time::Duration::from_millis(300));
    tap(&conn, root, END);
    for _ in 0..12 {
        tap(&conn, root, Z);
    }
    let near_end = right - (12.0 * scale) as usize;
    daemon.until("the caret to stay in view after a long text", || {
        let (w, _, px) = client(&conn, root, settings);
        (left + 3..right - 2)
            .rev()
            .find(|&x| accent(px[y * w + x]))
            .filter(|&x| x > near_end)
    });
    click(&conn, root, first);
    let near_start = left + (16.0 * scale) as usize;
    daemon.until("a click on the moved text to put the caret in view", || {
        let (w, _, px) = client(&conn, root, settings);
        (left + 3..right - 2)
            .find(|&x| accent(px[y * w + x]))
            .filter(|&x| x < near_start)
    });
    for keysym in [X, RETURN] {
        tap(&conn, root, keysym);
    }
    daemon.settle();
    let long = template(dir.path()).unwrap_or_default();
    assert!(
        long.replacen('x', "", 1) == format!("{CLICKED}{}", "z".repeat(12))
            && long.find('x') > Some(0),
        "a click on the moved text put the x of {long:?} at the start or \
         the z typed at the end were lost"
    );
}
