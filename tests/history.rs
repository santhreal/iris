//! Undo and redo across the editor's steps that rebuild the image, read
//! from the saved file.
//!
//! WHY: the class closed here is "a step that rebuilds the image cannot
//! be undone". A crop, a rotation, a flip, and Clear all cleared the
//! history: Undo then did nothing, and every mark drawn before the step
//! was flattened or gone for good. Each is now one history step that
//! swaps the image and its marks with the ones on its other side. The
//! cases draw a rectangle, take the step, and undo it, or undo and redo a
//! crop, then read the saved file: the original image with its
//! rectangle, or the crop with the rectangle in it. One case saves right
//! after Clear: the canvas draws marks from the list Clear empties, so
//! only the file shows a composite that still holds them. Rotation and
//! Clear run from the More menu, placed from the menu's geometry, so a
//! menu drawn away from its button fails the cases.
//!
//! The last case closes "an open toolbar menu takes no keys": with the
//! More or Copy menu open, Return saved and closed the editor and the
//! arrows did nothing. It runs Rotate Right from the More menu with Up
//! and Space, passing over the disabled Clear Markup, and with Down and
//! Return, then Copy Path from the Copy menu, and reads the image and the
//! clipboard.
//!
//! One case closes "the toolbar menus open only under a pointer": no
//! key opened the More or Copy menu, so Rotate, the flips, Copy Text,
//! Copy File, and Copy Path had no keyboard path. `Shift+F10` and the
//! Menu key each open the More menu and run Rotate Right; `Left` then
//! runs Copy Path from the Copy menu, and `Right` from the More menu
//! runs Copy Image.
//!
//! One case closes "keys act on the image under the shortcut sheet":
//! with the sheet open, Return saved and closed the editor, Delete
//! removed the selected mark, and the arrows moved it. It presses those
//! keys, a tool key, and Ctrl+Z under the sheet, closes the sheet, and
//! reads the canvas as it was; Ctrl+W under the sheet still closes the
//! editor.
//!
//! Two cases close "looking at a capture rewrites it": the editor's save
//! ran with nothing changed, so closing an image only viewed re-encoded
//! the file, moved its library entry to the top, and took the clipboard.
//! One closes with Return before any edit; the other draws, undoes the
//! mark, and closes from the window manager. Both read the file's bytes,
//! the library store, and the clipboard's owner after the daemon goes
//! idle.
//! A third case closes "an edit moves a capture to Today": the save
//! re-added the file's library entry at the top with the save's time.
//! It saves over the older of two stored captures and reads the store.
//!
//! One case closes "Copy Image copies the image without its marks": the
//! Copy menu's Copy Image read the file on disk, which holds no mark
//! until Done saves it. The case draws the rectangle, types a label it
//! leaves open, runs Copy Image, and reads the PNG on the clipboard and
//! the file's modification time.
//!
//! One case closes "Copy Text reads the file on disk": the More menu's
//! Copy Text read the saved file, so a crop did not narrow what it read
//! and a blur did not hide what it covered. The case draws the
//! rectangle, crops, runs Copy Text with a tesseract first on `PATH`
//! that saves the image it is given, and reads that image and the
//! clipboard.
//!
//! One case closes "Escape drops unsaved work at once": Escape over
//! marks closed the editor and discarded them with no second chance. It
//! draws a mark and reads that the first Escape keeps the editor and
//! shows the prompt, that a new mark removes the prompt and the prompt
//! later leaves on its own, each time leaving the next Escape to ask
//! again, and that a second Escape while the prompt shows closes the
//! editor with the file, the library, and the clipboard untouched.
//! Another closes "a status message never leaves": Copied and the
//! other reports stayed on the status line until the editor closed.
//! A third reads that the report of an image that fails to decode
//! stays: the editor has nothing else to show.
//!
//! Three cases close the Select tool's keys and clicks. "Selecting a
//! mark is an edit": the click that selected a mark pushed a move that
//! moved nothing, so Undo then did nothing and closing saved the file.
//! The case selects the rectangle, undoes once, and reads the file
//! untouched. "The arrow keys do not move a mark": it nudges the
//! selected rectangle with Shift+Right twice, undoes one nudge, and reads
//! the rectangle 10 pixels right in the saved file. "Ctrl+C does nothing
//! in the editor": it reads the PNG Ctrl+C puts on the clipboard.
//!
//! Not covered: the two flips, which go through the rotation's path;
//! redo after a rotation or Clear, which goes through the crop's swap;
//! the Library's context menu, whose keys `widgets::menu_key` runs as
//! these menus'; the arrow keys on a pending crop, which share the
//! nudge's key path; Windows and macOS. The cases run only with
//! `IRIS_X11_TEST_DISPLAY` set and need Xvfb; without them they print
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

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, entries, store, stored, xvfb_sized, Daemon, Server};
use x11::{
    chord, click, client, clipboard, clipboard_owner, clipboard_text, drag, extent, hold, let_go,
    move_to, press_and_move, release, tap, windows_of, wm_close,
};

const SCREEN: &str = "1280x1024x24";
/// The editor's title on `plain.png`.
const EDITOR: &str = "plain.png - iris";
/// The image's size and its one color, [r, g, b].
const SIZE: (u32, u32) = (600, 400);
const IMAGE: [u8; 3] = [60, 140, 200];
/// Keysyms.
const R: u32 = 0x72;
const C: u32 = 0x63;
const Z: u32 = 0x7a;
const RETURN: u32 = 0xff0d;
const CONTROL: u32 = 0xffe3;
const SHIFT: u32 = 0xffe1;
const UP: u32 = 0xff52;
const DOWN: u32 = 0xff54;
const SPACE: u32 = 0x20;
const T: u32 = 0x74;
const A: u32 = 0x61;
const ESCAPE: u32 = 0xff1b;
const V: u32 = 0x76;
const RIGHT: u32 = 0xff53;
const LEFT: u32 = 0xff51;
const F10: u32 = 0xffc7;
const END: u32 = 0xff57;
const HOME: u32 = 0xff50;
const MENU: u32 = 0xff67;
const SLASH: u32 = 0x2f;
const DELETE: u32 = 0xffff;
const W: u32 = 0x77;
const BRACKET_LEFT: u32 = 0x5b;
const BRACKET_RIGHT: u32 = 0x5d;
const F: u32 = 0x66;
const THREE: u32 = 0x33;
const B: u32 = 0x62;
const BACKSPACE: u32 = 0xff08;
const D: u32 = 0x64;
const E: u32 = 0x65;
const S: u32 = 0x73;
const Y: u32 = 0x79;
const X: u32 = 0x78;
/// The rectangle's corners and the crop's, in image pixels.
const RECT: [(f32, f32); 2] = [(100.0, 100.0), (200.0, 180.0)];
const CROP: [(f32, f32); 2] = [(50.0, 50.0), (300.0, 250.0)];
/// Where a label is typed, in image pixels.
const LABEL: (f32, f32) = (380.0, 280.0);
/// A second rectangle's corners, in image pixels.
const SECOND: [(f32, f32); 2] = [(300.0, 220.0), (400.0, 300.0)];
/// A third rectangle's corners, in image pixels.
const THIRD: [(f32, f32); 2] = [(420.0, 60.0), (520.0, 140.0)];
/// Where the status line shows, from `widgets::status_pill`: centered,
/// 10 px over the window's bottom edge; the span covers the middle of
/// its text.
fn status_left(w: usize) -> usize {
    (w - STATUS_W) / 2
}
const STATUS_BOTTOM: usize = 10;
const STATUS_H: usize = 22;
const STATUS_W: usize = 160;
/// Past the editor's 160 ms outro.
const OUTRO_PAST: std::time::Duration = std::time::Duration::from_millis(600);
/// How long a status message shows: `widgets::STATUS_FOR`.
const PROMPT_FOR: std::time::Duration = std::time::Duration::from_secs(4);
/// The More menu's geometry, from `theme` and `widgets::menu`: the
/// menu drops 4 px under the 28 px button centered in the 52 px toolbar,
/// from the button's left edge, kept 8 px inside the window's right
/// edge, and is 168 px wide with 5 px of padding, 24 px rows, and 9 px
/// separators.
const MENU_TOP: i16 = (52 - 28) / 2 + 28 + 4;
const MENU_W: i16 = 168;
const MENU_MARGIN: i16 = 8;
const MENU_PAD: i16 = 5;
const MENU_ROW: i16 = 24;
const MENU_SEPARATOR: i16 = 9;
/// The toolbar's middle row, where the More button's dots are, and a row
/// of Done's fill above its label: the button is 28 px tall from y 12.
const TOOLBAR_MID: usize = 26;
const DONE_FILL: usize = 16;

/// Done's fill, `theme::ACCENT` (#0a84ff), as [b, g, r].
fn accent([b, g, r]: [u8; 3]) -> bool {
    b > 240 && (120..145).contains(&g) && r < 30
}

/// The marks' color, `#ff453a`, over the image.
fn red(p: [u8; 3]) -> bool {
    p[0] > IMAGE[0] + 80 && p[2] < IMAGE[2] - 60
}

/// The rows of the More menu above an item: Rotate Right has one,
/// Clear Markup has four and a separator.
#[derive(Clone, Copy)]
struct MenuItem {
    rows: i16,
    separators: i16,
}
const ROTATE: MenuItem = MenuItem {
    rows: 1,
    separators: 0,
};
const CLEAR: MenuItem = MenuItem {
    rows: 4,
    separators: 1,
};

/// Copy Text is the More menu's first row.
const COPY_TEXT: MenuItem = MenuItem {
    rows: 0,
    separators: 0,
};

/// An editor open on `plain.png` with the image's place on the screen.
struct Session {
    daemon: Daemon,
    conn: RustConnection,
    root: u32,
    editor: u32,
    file: PathBuf,
    /// The file's modification time before the editor saves it.
    written: std::time::SystemTime,
    origin: (usize, usize),
    scale: f32,
    at: (i16, i16),
    _dir: tempfile::TempDir,
    _xvfb: Server,
}

impl Session {
    fn open(case: &str) -> Option<Session> {
        Self::open_with(case, |_, _| {})
    }

    /// `open`, with `env` run on the daemon's command and the session's
    /// directory before the daemon starts.
    fn open_with(case: &str, env: impl FnOnce(&Path, &mut Command)) -> Option<Session> {
        if !enabled() {
            return None;
        }
        let (xvfb, display) = xvfb_sized(case, SCREEN)?;
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plain.png");
        image::RgbImage::from_pixel(SIZE.0, SIZE.1, image::Rgb(IMAGE))
            .save(&file)
            .unwrap();
        let written = std::fs::metadata(&file).unwrap().modified().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
            env(dir.path(), cmd);
        });
        daemon.send(&[OsStr::new("--annotate"), file.as_os_str()]);
        let editor = daemon.until("the editor to open", || {
            windows_of(&conn, root, EDITOR).first().copied()
        });
        let (origin, scale) = daemon.until("the editor to draw the image", || {
            let (left, top, right, _) = extent(&client(&conn, root, editor), IMAGE)?;
            let scale = (right + 1 - left) as f32 / SIZE.0 as f32;
            (scale > 0.25).then_some(((left, top), scale))
        });
        daemon.settle();
        let at = conn
            .translate_coordinates(editor, root, 0, 0)
            .unwrap()
            .reply()
            .unwrap();
        Some(Session {
            daemon,
            conn,
            root,
            editor,
            file,
            written,
            origin,
            scale,
            at: (at.dst_x, at.dst_y),
            _dir: dir,
            _xvfb: xvfb,
        })
    }

    /// `p`, in the image as first shown, on the screen.
    fn screen_at(&self, p: (f32, f32)) -> (i16, i16) {
        (
            self.at.0 + (self.origin.0 as f32 + p.0 * self.scale) as i16,
            self.at.1 + (self.origin.1 as f32 + p.1 * self.scale) as i16,
        )
    }

    /// Select the tool on `key` and drag it across `corners`.
    fn draw(&self, key: u32, corners: [(f32, f32); 2]) {
        tap(&self.conn, self.root, key);
        self.daemon.settle();
        let (from, to) = (self.screen_at(corners[0]), self.screen_at(corners[1]));
        drag(&self.conn, self.root, from, to, 6);
        self.daemon.settle();
    }

    fn keys(&self, keysyms: &[u32]) {
        chord(&self.conn, self.root, keysyms);
        self.daemon.settle();
    }

    fn canvas(&self) -> (usize, usize, Vec<[u8; 3]>) {
        client(&self.conn, self.root, self.editor)
    }

    /// The More button's center: the first light pixels left of Done's
    /// fill are its rightmost dot, 5 px right of its middle.
    fn more_button(&self) -> (i16, i16) {
        let (w, _, px) = self.canvas();
        let row = |y: usize| &px[y * w..(y + 1) * w];
        // Done is the rightmost control; the selected tool has the same
        // fill in the middle of the bar. Its left edge is found above
        // its label.
        let fill = row(DONE_FILL);
        let right = (0..w)
            .rev()
            .find(|&x| accent(fill[x]))
            .expect("Done's fill is on the toolbar");
        let done = (0..right)
            .rev()
            .find(|&x| !accent(fill[x]))
            .expect("Done's fill has a left edge");
        let mid = row(TOOLBAR_MID);
        let dots = (0..done)
            .rev()
            .find(|&x| mid[x].iter().all(|&c| c > 150))
            .expect("the More button's dots are left of Done");
        (self.at.0 + dots as i16 - 5, self.at.1 + TOOLBAR_MID as i16)
    }

    /// Open the More menu and wait for it to drop under its button.
    fn open_more(&self) {
        click(&self.conn, self.root, self.more_button());
        self.daemon.settle();
        // The first row's label: light pixels across the panel's first
        // row, where the options bar under a misplaced menu has none.
        let left = self.more_menu_left();
        let first = MENU_TOP + MENU_PAD..MENU_TOP + MENU_PAD + MENU_ROW;
        self.daemon
            .until("the More menu to drop 4 px under its button", || {
                let (w, _, px) = self.canvas();
                first
                    .clone()
                    .flat_map(|y| (left..left + MENU_W).map(move |x| (x, y)))
                    .any(|(x, y)| px[y as usize * w + x as usize].iter().all(|&c| c > 150))
                    .then_some(())
            });
    }

    /// Open the More menu and run `item`.
    fn more(&self, item: MenuItem) {
        self.open_more();
        click(&self.conn, self.root, self.more_row(item));
        self.daemon.settle();
    }

    /// The center of row `item` in the open More menu.
    fn more_row(&self, item: MenuItem) -> (i16, i16) {
        let y = MENU_TOP
            + MENU_PAD
            + item.rows * MENU_ROW
            + item.separators * MENU_SEPARATOR
            + MENU_ROW / 2;
        let x = self.more_menu_left() + MENU_W / 2;
        (self.at.0 + x, self.at.1 + y)
    }

    /// The More menu's left edge in the editor window: the More
    /// button's left edge, moved left to keep the menu inside the
    /// window's right edge.
    fn more_menu_left(&self) -> i16 {
        let w = self.canvas().0 as i16;
        let button = self.more_button().0 - self.at.0 - 14;
        button.min(w - MENU_MARGIN - MENU_W)
    }

    /// The Undo and Redo buttons' centers. Ink is any pixel off the
    /// bar's fill in its middle rows; left of Copy's chevron, the runs
    /// of inked columns over 9 px apart are the Copy label, then the
    /// Redo and Undo glyphs.
    fn history_buttons(&self) -> [(i16, i16); 2] {
        let chevron = (self.copy_button().0 - self.at.0) as usize;
        let (w, _, px) = self.canvas();
        let inked = |x: usize| {
            (TOOLBAR_MID - 7..=TOOLBAR_MID + 7)
                .any(|y| px[y * w + x].iter().any(|&c| c.abs_diff(43) > 24))
        };
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for x in (0..chevron).rev().filter(|&x| inked(x)) {
            match runs.last_mut() {
                Some(run) if run.0 - x <= 9 => run.0 = x,
                _ => runs.push((x, x)),
            }
        }
        let label = runs.iter().position(|r| r.1 - r.0 > 20);
        let glyphs = label.and_then(|i| runs.get(i + 1..i + 3));
        let Some(&[redo, undo]) = glyphs else {
            panic!("the Copy label, Redo, and Undo left of Copy's chevron: {runs:?}");
        };
        assert!(
            redo.1 - redo.0 <= 18 && undo.1 - undo.0 <= 18,
            "Redo and Undo are 16 px glyphs: {runs:?}"
        );
        [undo, redo].map(|(left, right)| {
            (
                self.at.0 + ((left + right) / 2) as i16,
                self.at.1 + TOOLBAR_MID as i16,
            )
        })
    }

    /// With the Text tool, type `keysyms` as a label at LABEL and
    /// commit it with Return. The caret blinks while the entry is open,
    /// so the keys go in on a pause rather than on an idle daemon.
    fn label(&self, keysyms: &[u32]) {
        self.type_label(keysyms);
        self.keys(&[RETURN]);
    }

    /// With the Text tool, type `keysyms` as a label at LABEL and leave
    /// its entry open.
    fn type_label(&self, keysyms: &[u32]) {
        self.keys(&[T]);
        click(&self.conn, self.root, self.screen_at(LABEL));
        std::thread::sleep(std::time::Duration::from_millis(300));
        for &keysym in keysyms {
            tap(&self.conn, self.root, keysym);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    /// Reopen the label at LABEL with a press on its first character,
    /// the Text tool on, and type `keysyms` after its text: the press
    /// puts the caret before the character and End moves it after the
    /// text.
    fn reopen(&self, keysyms: &[u32]) {
        click(&self.conn, self.root, self.label_start());
        std::thread::sleep(std::time::Duration::from_millis(300));
        tap(&self.conn, self.root, END);
        for &keysym in keysyms {
            tap(&self.conn, self.root, keysym);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    /// A point on the first character of a label at LABEL, nearer its
    /// left edge than its right.
    fn label_start(&self) -> (i16, i16) {
        self.screen_at((LABEL.0 + 2.0, LABEL.1 - 5.0))
    }

    /// The rightmost canvas column the red of a label at LABEL reaches.
    fn label_end(&self) -> Option<usize> {
        let (lx, ly) = self.screen_at(LABEL);
        let (lx, ly) = ((lx - self.at.0) as usize, (ly - self.at.1) as usize);
        let (w, _, px) = self.canvas();
        (lx - 2..lx + 120).rev().find(|&x| {
            (ly - 25..ly + 8).any(|y| {
                let [b, g, r] = px[y * w + x];
                red([r, g, b])
            })
        })
    }

    /// The rightmost canvas column the red of an open label at LABEL
    /// reaches over one whole caret blink, its caret bar included.
    fn label_reach(&self) -> Option<usize> {
        let start = std::time::Instant::now();
        let mut reach = None;
        while start.elapsed() < std::time::Duration::from_millis(1200) {
            reach = reach.max(self.label_end());
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        reach
    }

    /// Open the Copy menu.
    fn open_copy(&self) {
        click(&self.conn, self.root, self.copy_button());
        self.daemon.settle();
    }

    /// The Copy button's chevron: the first light pixels left of the
    /// 28 px More button.
    fn copy_button(&self) -> (i16, i16) {
        let more = self.more_button();
        let (w, _, px) = self.canvas();
        let mid = &px[TOOLBAR_MID * w..(TOOLBAR_MID + 1) * w];
        let more_left = (more.0 - self.at.0 - 14) as usize;
        let chevron = (0..more_left)
            .rev()
            .find(|&x| mid[x].iter().all(|&c| c > 150))
            .expect("the Copy button is left of the More button");
        (
            self.at.0 + chevron as i16 - 2,
            self.at.1 + TOOLBAR_MID as i16,
        )
    }

    /// Whether the image stands on its side: taller than wide.
    fn on_its_side(&self) -> Option<bool> {
        let (left, top, right, bottom) = extent(&self.canvas(), IMAGE)?;
        Some(bottom - top > right - left)
    }

    /// The pixels where the status line shows.
    fn status_line(&self) -> Vec<[u8; 3]> {
        let (w, h, px) = self.canvas();
        let px = &px;
        let left = status_left(w);
        (h - STATUS_BOTTOM - STATUS_H..h - STATUS_BOTTOM)
            .flat_map(|y| (left..left + STATUS_W).map(move |x| px[y * w + x]))
            .collect()
    }

    /// Press Escape and assert that the editor is still open past its
    /// outro.
    fn escape_keeps_open(&self, what: &str) {
        tap(&self.conn, self.root, ESCAPE);
        std::thread::sleep(OUTRO_PAST);
        let open = windows_of(&self.conn, self.root, EDITOR);
        assert!(
            !open.is_empty(),
            "{what} closed the editor over an unsaved mark"
        );
    }

    /// Save with Return and return the saved file.
    fn save(self) -> image::RgbImage {
        self.save_with(&[RETURN])
    }

    /// Save with the chord `keysyms` and return the saved file.
    fn save_with(self, keysyms: &[u32]) -> image::RgbImage {
        chord(&self.conn, self.root, keysyms);
        // The save lands after the window closes, over the original file.
        self.daemon.until("the editor to save and close", || {
            if !windows_of(&self.conn, self.root, EDITOR).is_empty() {
                return None;
            }
            let modified = std::fs::metadata(&self.file).ok()?.modified().ok()?;
            if modified == self.written {
                return None;
            }
            // A file still being written does not decode.
            Some(image::open(&self.file).ok()?.to_rgb8())
        })
    }
}

/// Assert that `img` holds the rectangle with its sides at `x0` and `x1`
/// and its top and bottom at `y0` and `y1`: on the row halfway down, red
/// within 2 px of each side and nowhere else. The pointer places each
/// corner to a screen pixel, which is not always an image pixel.
fn assert_rect(img: &image::RgbImage, (x0, x1): (u32, u32), (y0, y1): (u32, u32), what: &str) {
    let y = (y0 + y1) / 2;
    let reds: Vec<u32> = (0..img.width())
        .filter(|&x| red(img.get_pixel(x, y).0))
        .collect();
    let near = |edge: u32| reds.iter().any(|&x| x.abs_diff(edge) <= 2);
    let stray = reds
        .iter()
        .any(|&x| x.abs_diff(x0) > 4 && x.abs_diff(x1) > 4);
    assert!(
        near(x0) && near(x1) && !stray,
        "{what}: the rectangle's sides should be red near x {x0} and {x1}, \
         and the row is red at {reds:?}"
    );
}

/// Assert that `img` is the original image with the rectangle on it.
fn assert_original(img: &image::RgbImage, what: &str) {
    assert_eq!(img.dimensions(), SIZE, "{what}: the saved file's size");
    assert_rect(
        img,
        (RECT[0].0 as u32, RECT[1].0 as u32),
        (RECT[0].1 as u32, RECT[1].1 as u32),
        what,
    );
}

/// Draw the rectangle and crop.
fn cropped(case: &str) -> Option<Session> {
    let session = Session::open(case)?;
    session.draw(R, RECT);
    session.draw(C, CROP);
    session.keys(&[RETURN]);
    Some(session)
}

#[test]
fn undo_restores_the_image_and_marks_a_crop_cut() {
    let Some(session) = cropped("undo_restores_the_image_and_marks_a_crop_cut") else {
        return;
    };
    session.keys(&[CONTROL, Z]);
    assert_original(&session.save(), "after undoing the crop");
}

#[test]
fn redo_cuts_the_image_again_with_the_marks_in_it() {
    let Some(session) = cropped("redo_cuts_the_image_again_with_the_marks_in_it") else {
        return;
    };
    session.keys(&[CONTROL, Z]);
    session.keys(&[CONTROL, SHIFT, Z]);
    assert_cropped(&session.save(), "the file saved after redoing the crop");
}

/// Assert that `img` is the crop with the rectangle in it.
fn assert_cropped(img: &image::RgbImage, what: &str) {
    let want = (
        (CROP[1].0 - CROP[0].0) as u32,
        (CROP[1].1 - CROP[0].1) as u32,
    );
    let (w, h) = img.dimensions();
    assert!(
        w.abs_diff(want.0) <= 2 && h.abs_diff(want.1) <= 2,
        "{what} is {w}x{h}, not about {}x{}",
        want.0,
        want.1
    );
    let off = |p: (f32, f32)| ((p.0 - CROP[0].0) as u32, (p.1 - CROP[0].1) as u32);
    let (a, b) = (off(RECT[0]), off(RECT[1]));
    assert_rect(img, (a.0, b.0), (a.1, b.1), what);
}

/// A tesseract that copies the image it reads to `read.png` beside
/// itself and recognizes one line, OCR_LINE.
const FAKE_TESSERACT: &str =
    "#!/bin/sh\ncp \"$1\" \"$(dirname \"$0\")/read.png\"\necho 'Iris read this'\n";
const OCR_LINE: &str = "Iris read this";

#[test]
fn copy_text_reads_the_image_as_shown() {
    let Some(session) = Session::open_with("copy_text_reads_the_image_as_shown", |dir, cmd| {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("bin");
        std::fs::create_dir(&bin).unwrap();
        let tool = bin.join("tesseract");
        std::fs::write(&tool, FAKE_TESSERACT).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::iter::once(bin).chain(std::env::split_paths(&path));
        cmd.env("PATH", std::env::join_paths(path).unwrap());
    }) else {
        return;
    };
    session.draw(R, RECT);
    session.draw(C, CROP);
    session.keys(&[RETURN]);
    session.more(COPY_TEXT);
    session
        .daemon
        .until("Copy Text to copy the recognized line", || {
            (clipboard_text(&session.conn, session.root)? == OCR_LINE).then_some(())
        });
    let read = image::open(session._dir.path().join("bin/read.png"))
        .unwrap()
        .to_rgb8();
    assert_cropped(&read, "the image Copy Text read");
    // The image went to tesseract through the cache, and left it.
    let left: Vec<_> = std::fs::read_dir(session._dir.path().join("cache"))
        .map(|d| d.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        left.iter()
            .all(|n| !n.to_string_lossy().starts_with("ocr-")),
        "the cache still holds {left:?}"
    );
}

#[test]
fn undo_turns_a_rotated_image_and_its_marks_back() {
    let Some(session) = Session::open("undo_turns_a_rotated_image_and_its_marks_back") else {
        return;
    };
    session.draw(R, RECT);
    session.more(ROTATE);
    session
        .daemon
        .until("Rotate Right to stand the image on its side", || {
            session.on_its_side()?.then_some(())
        });
    session.keys(&[CONTROL, Z]);
    assert_original(&session.save(), "after undoing the rotation");
}

/// Draw the rectangle and run Clear Markup.
fn cleared(case: &str) -> Option<Session> {
    let session = Session::open(case)?;
    session.draw(R, RECT);
    session.more(CLEAR);
    // Over the image only: the selected color's swatch is the marks'
    // color too.
    let (x0, y0) = session.origin;
    let (x1, y1) = (
        x0 + (SIZE.0 as f32 * session.scale) as usize,
        y0 + (SIZE.1 as f32 * session.scale) as usize,
    );
    session
        .daemon
        .until("Clear Markup to remove the mark from the canvas", || {
            let (w, _, px) = session.canvas();
            let marked = (y0..y1).any(|y| {
                px[y * w + x0..y * w + x1]
                    .iter()
                    .any(|&[b, g, r]| red([r, g, b]))
            });
            (!marked).then_some(())
        });
    Some(session)
}

#[test]
fn undo_brings_back_the_marks_clear_removed() {
    let Some(session) = cleared("undo_brings_back_the_marks_clear_removed") else {
        return;
    };
    session.keys(&[CONTROL, Z]);
    assert_original(&session.save(), "after undoing Clear Markup");
}

#[test]
fn the_file_saved_after_clear_has_no_marks() {
    let Some(session) = cleared("the_file_saved_after_clear_has_no_marks") else {
        return;
    };
    let saved = session.save();
    let marks = saved.pixels().filter(|p| red(p.0)).count();
    assert_eq!(
        (saved.dimensions(), marks),
        (SIZE, 0),
        "the file saved after Clear Markup: its size and its marked pixels"
    );
}

#[test]
fn the_arrow_keys_and_return_run_a_toolbar_menu_row() {
    let Some(session) = Session::open("the_arrow_keys_and_return_run_a_toolbar_menu_row") else {
        return;
    };
    // Return with nothing lit closes the menu and leaves the editor
    // open and the file as it was.
    session.open_more();
    session.keys(&[RETURN]);
    assert!(
        !windows_of(&session.conn, session.root, EDITOR).is_empty(),
        "Return in the open More menu closed the editor"
    );
    assert_eq!(
        std::fs::metadata(&session.file)
            .unwrap()
            .modified()
            .unwrap(),
        session.written,
        "Return in the open More menu saved the file"
    );

    // Up from nothing lights Discard, the last row; then Keyboard
    // Shortcuts; then, over the disabled Clear Markup of an unmarked
    // image, Flip Vertical, Flip Horizontal, and Rotate Right.
    session.open_more();
    for _ in 0..5 {
        session.keys(&[UP]);
    }
    session.keys(&[SPACE]);
    session
        .daemon
        .until("Up and Space to run Rotate Right", || {
            session.on_its_side()?.then_some(())
        });

    // Down from nothing lights Copy Text, then Rotate Right.
    session.open_more();
    session.keys(&[DOWN]);
    session.keys(&[DOWN]);
    session.keys(&[RETURN]);
    session
        .daemon
        .until("Down and Return to run Rotate Right", || {
            (!session.on_its_side()?).then_some(())
        });

    // Copy Path is the Copy menu's third row.
    session.open_copy();
    for _ in 0..3 {
        session.keys(&[DOWN]);
    }
    session.keys(&[RETURN]);
    let path = session.file.canonicalize().unwrap();
    session
        .daemon
        .until("Down and Return to run Copy Path", || {
            (clipboard_text(&session.conn, session.root)? == path.to_string_lossy()).then_some(())
        });
    assert!(
        !windows_of(&session.conn, session.root, EDITOR).is_empty(),
        "a menu row run from the keyboard closed the editor"
    );
}

#[test]
fn shift_f10_and_the_menu_key_open_the_toolbar_menus() {
    let Some(session) = Session::open("shift_f10_and_the_menu_key_open_the_toolbar_menus") else {
        return;
    };
    // With no window manager the keyboard follows the pointer.
    move_to(
        &session.conn,
        session.root,
        session.screen_at((300.0, 200.0)),
    );
    session.daemon.settle();
    // Down from nothing lights Copy Text, then Rotate Right.
    session.keys(&[SHIFT, F10]);
    session.keys(&[DOWN]);
    session.keys(&[DOWN]);
    session.keys(&[RETURN]);
    session
        .daemon
        .until("Shift+F10, Down, and Return to run Rotate Right", || {
            session.on_its_side()?.then_some(())
        });
    session.keys(&[MENU]);
    session.keys(&[DOWN]);
    session.keys(&[DOWN]);
    session.keys(&[RETURN]);
    session
        .daemon
        .until("the Menu key, Down, and Return to run Rotate Right", || {
            (!session.on_its_side()?).then_some(())
        });
    // Copy Path is the Copy menu's third row.
    session.keys(&[SHIFT, F10]);
    session.keys(&[LEFT]);
    for _ in 0..3 {
        session.keys(&[DOWN]);
    }
    session.keys(&[RETURN]);
    let path = session.file.canonicalize().unwrap();
    session
        .daemon
        .until("Left from the More menu to run Copy Path", || {
            (clipboard_text(&session.conn, session.root)? == path.to_string_lossy()).then_some(())
        });
    // Copy Image is its first.
    session.keys(&[MENU]);
    session.keys(&[RIGHT]);
    session.keys(&[DOWN]);
    session.keys(&[RETURN]);
    session
        .daemon
        .until("Right from the More menu to run Copy Image", || {
            clipboard(&session.conn, session.root, b"image/png").map(|_| ())
        });
    assert!(
        !windows_of(&session.conn, session.root, EDITOR).is_empty(),
        "a menu row run from the keyboard closed the editor"
    );
}

#[test]
fn the_shortcut_sheet_holds_the_keys_meant_for_the_image() {
    let Some(session) = selected("the_shortcut_sheet_holds_the_keys_meant_for_the_image") else {
        return;
    };
    // A selected rectangle: Delete, the arrows, the options bar's keys,
    // and Return each change it or close the editor.
    let before = session.canvas();
    session.keys(&[SLASH]);
    session.daemon.until("/ to open the shortcut sheet", || {
        (session.canvas() != before).then_some(())
    });
    for key in [DELETE, DOWN, RIGHT, BRACKET_RIGHT, THREE, F, RETURN, R, Z] {
        session.keys(&[key]);
    }
    session.keys(&[CONTROL, Z]);
    session.keys(&[ESCAPE]);
    session.daemon.until("Escape to close the sheet", || {
        (session.canvas() == before).then_some(())
    });
    // Ctrl+W under the sheet closes the editor.
    session.keys(&[SLASH]);
    session.keys(&[CONTROL, W]);
    session
        .daemon
        .until("Ctrl+W under the sheet to close the editor", || {
            windows_of(&session.conn, session.root, EDITOR)
                .is_empty()
                .then_some(())
        });
}

// WHY: `]` and `[` step the markup color through the swatches, so a
// rectangle drawn after `] ] [` from the default red is orange in the
// saved file. A binding that ignores either key, runs the wrong way,
// or changes only the swatch and not the color the next mark takes
// saves another color here. The order and the wrap at both ends are
// covered by the color_keys_walk_the_swatches_in_order_and_wrap unit
// test.
#[test]
fn bracket_keys_step_the_color_the_next_mark_takes() {
    let Some(session) = Session::open("bracket_keys_step_the_color_the_next_mark_takes") else {
        return;
    };
    // With no window manager the keyboard follows the pointer.
    move_to(
        &session.conn,
        session.root,
        session.screen_at((300.0, 300.0)),
    );
    session.daemon.settle();
    for key in [BRACKET_RIGHT, BRACKET_RIGHT, BRACKET_LEFT] {
        session.keys(&[key]);
    }
    session.draw(R, RECT);
    let saved = session.save();
    // `#ff9f0a`, and `#ff453a` for the default red, over the image.
    let orange = |[r, g, b]: [u8; 3]| r > 230 && (140..180).contains(&g) && b < 40;
    let red = |[r, g, _]: [u8; 3]| r > 230 && g < 100;
    let y = ((RECT[0].1 + RECT[1].1) / 2.0) as u32;
    let row: Vec<[u8; 3]> = (0..saved.width())
        .map(|x| saved.get_pixel(x, y).0)
        .collect();
    let near = |edge: f32| (0..row.len()).any(|x| orange(row[x]) && (x as f32 - edge).abs() <= 2.0);
    assert_eq!(
        (
            near(RECT[0].0),
            near(RECT[1].0),
            row.iter().any(|&p| red(p))
        ),
        (true, true, false),
        "the rectangle drawn after ] ] [: orange at its left side, at its \
         right side, and red anywhere on the row"
    );
}

/// Wait for the editor to close and the daemon to finish what the close
/// started, then assert that the file, the library store, and the
/// clipboard are as before.
fn assert_untouched(session: Session, before: &[u8], how: &str) {
    session.daemon.until("the editor to close", || {
        windows_of(&session.conn, session.root, EDITOR)
            .is_empty()
            .then_some(())
    });
    session.daemon.settle();
    assert!(
        std::fs::read(&session.file).unwrap() == before,
        "{how} rewrote the unchanged file"
    );
    assert_eq!(
        stored(session._dir.path()).unwrap_or_default(),
        Vec::<PathBuf>::new(),
        "{how} added the unchanged file to the library"
    );
    assert_eq!(
        clipboard_owner(&session.conn),
        x11rb::NONE,
        "{how} took the clipboard"
    );
}

#[test]
fn closing_before_any_edit_leaves_the_file_alone() {
    let Some(session) = Session::open("closing_before_any_edit_leaves_the_file_alone") else {
        return;
    };
    let before = std::fs::read(&session.file).unwrap();
    tap(&session.conn, session.root, RETURN);
    assert_untouched(session, &before, "Return before any edit");
}

#[test]
fn closing_with_every_edit_undone_leaves_the_file_alone() {
    let Some(session) = Session::open("closing_with_every_edit_undone_leaves_the_file_alone")
    else {
        return;
    };
    let before = std::fs::read(&session.file).unwrap();
    session.draw(R, RECT);
    session.keys(&[CONTROL, Z]);
    wm_close(&session.conn, session.editor);
    assert_untouched(
        session,
        &before,
        "the window manager's close after undoing the mark",
    );
}

#[test]
fn a_save_keeps_the_capture_in_its_place_in_the_library() {
    let Some(session) = Session::open("a_save_keeps_the_capture_in_its_place_in_the_library")
    else {
        return;
    };
    let dir = session._dir.path();
    let newer = dir.join("newer.png");
    image::RgbImage::from_pixel(8, 6, image::Rgb(IMAGE))
        .save(&newer)
        .unwrap();
    store(dir, &[newer, session.file.clone()]);
    let taken: Vec<_> = entries(dir)
        .unwrap()
        .into_iter()
        .map(|e| (e.path, e.created_ms))
        .collect();
    session.draw(R, RECT);
    tap(&session.conn, session.root, RETURN);
    let after = session.daemon.until("the save to refresh the library", || {
        let after = entries(dir)?;
        after
            .iter()
            .any(|e| e.path == session.file && e.revision() != (taken[1].1, 0))
            .then_some(after)
    });
    assert_eq!(
        after
            .iter()
            .map(|e| (e.path.clone(), e.created_ms))
            .collect::<Vec<_>>(),
        taken,
        "the library after the save: its order and the capture times"
    );
}

#[test]
fn copy_image_copies_the_marks_drawn_before_it() {
    let Some(session) = Session::open("copy_image_copies_the_marks_drawn_before_it") else {
        return;
    };
    session.draw(R, RECT);
    // A label still being typed when Copy Image runs. The caret blinks
    // while it is open, so the daemon does not go idle until the copy
    // commits it: the steps wait on the screen instead.
    let (conn, root) = (&session.conn, session.root);
    tap(conn, root, T);
    session.daemon.settle();
    let copy = session.copy_button();
    click(conn, root, session.screen_at(LABEL));
    std::thread::sleep(std::time::Duration::from_millis(300));
    tap(conn, root, A);
    tap(conn, root, A);
    let (lx, ly) = session.screen_at(LABEL);
    let (lx, ly) = ((lx - session.at.0) as usize, (ly - session.at.1) as usize);
    session
        .daemon
        .until("the label to show as it is typed", || {
            let (w, _, px) = session.canvas();
            let shown = (lx..lx + 40)
                .flat_map(|x| (ly.saturating_sub(20)..ly + 20).map(move |y| (x, y)))
                .filter(|&(x, y)| {
                    let [b, g, r] = px[y * w + x];
                    red([r, g, b])
                })
                .count();
            (shown > 10).then_some(())
        });
    // Copy Image is the Copy menu's first row.
    click(conn, root, copy);
    std::thread::sleep(std::time::Duration::from_millis(300));
    tap(conn, root, DOWN);
    tap(conn, root, RETURN);
    let copied = session
        .daemon
        .until("Copy Image to put a PNG on the clipboard", || {
            let png = clipboard(&session.conn, session.root, b"image/png")?;
            Some(image::load_from_memory(&png).ok()?.to_rgb8())
        });
    assert_original(&copied, "the copied image");
    let (x, y) = (LABEL.0 as u32, LABEL.1 as u32);
    let label = (x..x + 60)
        .flat_map(|px| (y.saturating_sub(30)..y + 30).map(move |py| (px, py)))
        .filter(|&(px, py)| red(copied.get_pixel(px, py).0))
        .count();
    assert!(
        label > 20,
        "the copied image holds {label} red pixels of the label typed at {LABEL:?}"
    );
    assert_eq!(
        std::fs::metadata(&session.file)
            .unwrap()
            .modified()
            .unwrap(),
        session.written,
        "Copy Image saved the file"
    );
}

// WHY: a committed label reopens for typing on a Text tool press or a
// Select tool double-click. A change commits as one undo step in the
// label's place, an emptied label is removed, and Escape or no change
// leaves the label as it was. A reopen that adds a second label, keeps
// the old one drawn under the entry, loses the label on Escape, or
// folds the change into another step shows another label here.
#[test]
fn a_label_reopens_for_editing_as_one_undo_step() {
    let Some(session) = Session::open("a_label_reopens_for_editing_as_one_undo_step") else {
        return;
    };
    let (conn, root) = (&session.conn, session.root);
    let lx = (session.screen_at(LABEL).0 - session.at.0) as usize;
    let right = || session.label_end();
    let inside = session.label_start();
    let pause = || std::thread::sleep(std::time::Duration::from_millis(300));
    session.label(&[A, B]);
    let ab = right();
    // A Text tool press on the label reopens it, with the caret where
    // it pressed; End moves the caret after the text.
    click(conn, root, inside);
    pause();
    tap(conn, root, END);
    tap(conn, root, C);
    session.keys(&[RETURN]);
    let abc = right();
    click(conn, root, inside);
    pause();
    tap(conn, root, END);
    tap(conn, root, Z);
    tap(conn, root, Z);
    session.keys(&[ESCAPE]);
    let escaped = right();
    // A Select tool double-click reopens it; emptied, it goes.
    session.keys(&[V]);
    click(conn, root, inside);
    click(conn, root, inside);
    pause();
    tap(conn, root, END);
    for _ in 0..3 {
        tap(conn, root, BACKSPACE);
    }
    session
        .daemon
        .until("Backspace to empty the reopened label", || {
            right().is_none_or(|x| x < lx + 8).then_some(())
        });
    session.keys(&[RETURN]);
    let removed = right();
    session.keys(&[CONTROL, Z]);
    let undone = right();
    session.keys(&[CONTROL, Z]);
    let undone_twice = right();
    assert_eq!(
        (
            abc > ab.map(|x| x + 2),
            escaped == abc,
            removed,
            undone == abc,
            undone_twice == ab,
        ),
        (true, true, None, true, true),
        "the label's right end typed as ab ({ab:?}), reopened to abc ({abc:?}), \
         after Escape ({escaped:?}), emptied, after Undo ({undone:?}), and \
         after a second Undo ({undone_twice:?})"
    );
}

// WHY: Undo, Redo, and Clear Markup settle a reopened label before
// they change the marks. One that runs past the open entry undoes,
// redoes, or clears the mark under it and leaves the entry on screen,
// standing for a mark that is gone.
#[test]
fn undo_redo_and_clear_markup_commit_a_reopened_label_first() {
    let Some(session) = Session::open("undo_redo_and_clear_markup_commit_a_reopened_label_first")
    else {
        return;
    };
    let (conn, root) = (&session.conn, session.root);
    let [undo, redo] = session.history_buttons();
    session.label(&[A, B]);
    let ab = session.label_end();
    // The toolbar's Undo settles the reopened abc, then undoes it.
    session.reopen(&[C]);
    click(conn, root, undo);
    session.daemon.until("Undo to settle abc and undo it", || {
        (session.label_end() == ab).then_some(())
    });
    session.keys(&[CONTROL, SHIFT, Z]);
    let abc = session.label_end();
    // Clear Markup settles the reopened abcd, then clears it. The
    // caret blinks under the menu, so the daemon never goes idle.
    session.reopen(&[D]);
    click(conn, root, session.more_button());
    std::thread::sleep(std::time::Duration::from_millis(500));
    click(conn, root, session.more_row(CLEAR));
    session
        .daemon
        .until("Clear Markup to settle abcd and clear it", || {
            session.label_end().is_none().then_some(())
        });
    session.keys(&[CONTROL, Z]);
    let abcd = session.label_end();
    // The toolbar's Redo settles the reopened abcde: a new step, so
    // nothing is left to redo and the Clear Markup undone stays undone.
    session.reopen(&[E]);
    click(conn, root, redo);
    session
        .daemon
        .until("Redo to settle abcde and redo nothing", || {
            (session.label_end() > abcd.map(|x| x + 2)).then_some(())
        });
    session.keys(&[CONTROL, Z]);
    let undone = session.label_end();
    assert_eq!(
        (
            abc > ab.map(|x| x + 2),
            abcd > abc.map(|x| x + 2),
            undone == abcd,
        ),
        (true, true, true),
        "the label's right end typed as ab ({ab:?}), after Redo ({abc:?}), \
         after the Undo of Clear Markup ({abcd:?}), and after the Undo of \
         abcde ({undone:?})"
    );
}

// WHY: Left, Delete, and End act at the caret of an open label, and the
// caret bar is drawn where the caret is. An entry that only appends at
// its end and pops from it ignores the three, so the label keeps the
// letter Delete removes and ends wider than the same text typed whole;
// a bar drawn after the whole text instead of at the caret still
// reaches past the label's end with the caret at its start.
#[test]
fn caret_keys_edit_a_label_where_the_caret_is() {
    let Some(session) = Session::open("caret_keys_edit_a_label_where_the_caret_is") else {
        return;
    };
    let (conn, root) = (&session.conn, session.root);
    session.label(&[B, C, D, E]);
    let typed = session.label_end();
    session.keys(&[CONTROL, Z]);
    // abcd with the caret after it, then back at its start.
    session.type_label(&[A, B, C, D]);
    let at_end = session.label_reach();
    for _ in 0..4 {
        tap(conn, root, LEFT);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    let at_start = session.label_reach();
    // The a deleted, then e typed at the end.
    for keysym in [DELETE, END, E] {
        tap(conn, root, keysym);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    session.keys(&[RETURN]);
    let edited = session.label_end();
    assert!(typed.is_some(), "the label bcde was drawn");
    assert!(
        at_start < at_end,
        "the open label reaches column {at_end:?} with the caret after \
         abcd and {at_start:?} with it at the start"
    );
    assert_eq!(
        edited, typed,
        "a label typed as abcd, edited at its start and end, ends where \
         bcde typed whole does"
    );
}

// WHY: Shift with an arrow selects label text, a typed key replaces the
// selection, a Shift-click selects to the click, Ctrl+A selects all of
// it, Ctrl+C copies it as text instead of copying the image, and
// Ctrl+X cuts it. An entry with no selection types this case's e
// between the b and the c, so the label ends wider than abe; one that
// moves the caret on a Shift-click or runs Copy Image for Ctrl+C
// leaves no text on the clipboard; one that ignores Ctrl+A or Ctrl+X
// leaves the label after Return.
#[test]
fn a_selection_in_a_label_is_replaced_copied_and_cut() {
    let Some(session) = Session::open("a_selection_in_a_label_is_replaced_copied_and_cut") else {
        return;
    };
    let (conn, root) = (&session.conn, session.root);
    let pause = || std::thread::sleep(std::time::Duration::from_millis(300));
    session.label(&[A, B, E]);
    let typed = session.label_end();
    session.keys(&[CONTROL, Z]);
    // abcd with cd selected, then e typed over it.
    session.type_label(&[A, B, C, D]);
    chord(conn, root, &[SHIFT, LEFT]);
    chord(conn, root, &[SHIFT, LEFT]);
    tap(conn, root, E);
    pause();
    session.keys(&[RETURN]);
    let replaced = session.label_end();
    // Reopened with the caret at its start, all of it selected with a
    // Shift-click on its end, and copied.
    let typed_end = typed.expect("the label abe was drawn") as i16;
    click(conn, root, session.label_start());
    pause();
    hold(conn, root, SHIFT);
    click(
        conn,
        root,
        (session.at.0 + typed_end - 1, session.label_start().1),
    );
    let_go(conn, root, SHIFT);
    pause();
    chord(conn, root, &[CONTROL, C]);
    let copied = session.daemon.until("Ctrl+C to copy the label's text", || {
        clipboard_text(conn, root).filter(|text| !text.is_empty())
    });
    // Home drops the selection; Ctrl+A selects all of it again, and the
    // cut empties the label, which Return removes.
    tap(conn, root, HOME);
    chord(conn, root, &[CONTROL, A]);
    // Cut, the emptied label is removed on Return.
    chord(conn, root, &[CONTROL, X]);
    pause();
    session.keys(&[RETURN]);
    let cut = session.label_end();
    let held = clipboard_text(conn, root);
    session.keys(&[CONTROL, Z]);
    let undone = session.label_end();
    assert_eq!(
        replaced, typed,
        "abcd with cd selected and e typed ends where abe typed whole does"
    );
    assert_eq!(
        (copied.as_str(), cut, held.as_deref(), undone),
        ("abe", None, Some("abe"), typed),
        "a Shift-click and Ctrl+C copy the label's text, Ctrl+A and Ctrl+X \
         cut it so Return removes the label, and one Undo brings it back"
    );
}

// WHY: a double-click on a label's word selects the word, and a drag
// across a label selects what it passes over, as in any text field. An
// entry that only places the caret types this case's e between the c
// and the d, so the label ends wider than ab e, and leaves nothing
// selected for Ctrl+X to cut after the drag.
#[test]
fn a_double_click_or_a_drag_selects_label_text() {
    let Some(session) = Session::open("a_double_click_or_a_drag_selects_label_text") else {
        return;
    };
    let (conn, root) = (&session.conn, session.root);
    let pause = || std::thread::sleep(std::time::Duration::from_millis(300));
    session.label(&[A, B, SPACE, E]);
    let typed = session.label_end();
    session.keys(&[CONTROL, Z]);
    session.label(&[A, B, SPACE, C, D]);
    let y = session.label_start().1;
    let at = |x: Option<usize>| session.at.0 + x.expect("the label was drawn") as i16;
    // A double-click on the d reopens the label with cd selected, and
    // an e replaces it.
    let on_d = (at(session.label_end()) - 3, y);
    click(conn, root, on_d);
    click(conn, root, on_d);
    pause();
    tap(conn, root, E);
    pause();
    session.keys(&[RETURN]);
    let replaced = session.label_end();
    // A drag from the label's end to its start, past the 400 ms
    // double-click interval so its press is one click, reopens the
    // label with all of it selected, and Ctrl+X cuts it.
    std::thread::sleep(std::time::Duration::from_millis(500));
    press_and_move(conn, root, (at(replaced) - 1, y), session.label_start(), 4);
    release(conn, root);
    pause();
    chord(conn, root, &[CONTROL, X]);
    let cut = session.daemon.until("Ctrl+X to cut the dragged text", || {
        clipboard_text(conn, root).filter(|text| !text.is_empty())
    });
    session.keys(&[RETURN]);
    let gone = session.label_end();
    assert!(typed.is_some(), "the label ab e was drawn");
    assert_eq!(
        replaced, typed,
        "ab cd with cd double-clicked and e typed ends where ab e typed whole does"
    );
    assert_eq!(
        (cut.as_str(), gone),
        ("ab e", None),
        "a drag across the label selects all of it for Ctrl+X to cut"
    );
}

// WHY: a press on a label's text puts the caret where it lands, both
// when the press reopens the label and when the label is open. An
// entry that reopens with the caret after the text types this case's b
// after cd, and a press on the open label that commits and reopens it
// makes two undo steps of one edit, so Undo leaves bcd instead of cd.
#[test]
fn a_press_on_a_label_puts_the_caret_where_it_lands() {
    let Some(session) = Session::open("a_press_on_a_label_puts_the_caret_where_it_lands") else {
        return;
    };
    let (conn, root) = (&session.conn, session.root);
    let pause = || std::thread::sleep(std::time::Duration::from_millis(300));
    session.label(&[A, B, C, D]);
    let typed = session.label_end();
    session.keys(&[CONTROL, Z]);
    // cd, with the caret after it while it is open.
    session.type_label(&[C, D]);
    let at_end = session.label_reach();
    session.keys(&[RETURN]);
    let cd = session.label_end();
    // A press on the c reopens cd with the caret before it.
    click(conn, root, session.label_start());
    pause();
    let at_start = session.label_reach();
    // A b typed there, then a press on the b with the label open, and
    // an a typed before it.
    tap(conn, root, B);
    pause();
    click(conn, root, session.label_start());
    pause();
    tap(conn, root, A);
    pause();
    session.keys(&[RETURN]);
    let edited = session.label_end();
    session.keys(&[CONTROL, Z]);
    let undone = session.label_end();
    assert!(
        typed.is_some() && cd.is_some(),
        "the labels abcd and cd were drawn"
    );
    assert!(
        at_start < at_end,
        "the open label reaches column {at_end:?} with the caret after cd \
         and {at_start:?} after a press on its c"
    );
    assert_eq!(
        (edited, undone),
        (typed, cd),
        "a label typed as cd, edited at two presses on its start, ends where \
         abcd typed whole does, and one Undo returns it to cd"
    );
}

// WHY: Undo, Redo, and Copy Image run from their keys with a label
// open, as from their toolbar buttons, each settling the label first.
// An entry that takes the keys as typing leaves them dead until the
// label is committed by hand. Ctrl+S and Ctrl+W are below.
#[test]
fn undo_redo_and_copy_keys_run_with_a_label_open() {
    let Some(session) = Session::open("undo_redo_and_copy_keys_run_with_a_label_open") else {
        return;
    };
    let (conn, root) = (&session.conn, session.root);
    // Past `end` by a letter: more than the caret after it reaches.
    let past = |x: Option<usize>, end: Option<usize>| x > end.map(|x| x + 8);
    session.label(&[A, B]);
    let ab = session.label_end();
    session.reopen(&[C]);
    chord(conn, root, &[CONTROL, Z]);
    session
        .daemon
        .until("Ctrl+Z to settle abc and undo it", || {
            (session.label_end() == ab).then_some(())
        });
    // Reopened with no change, the label settles as no step, and the
    // undone abc is still there to redo.
    session.reopen(&[]);
    chord(conn, root, &[CONTROL, SHIFT, Z]);
    let abc = session.daemon.until("Ctrl+Shift+Z to redo abc", || {
        Some(session.label_end()).filter(|&end| past(end, ab))
    });
    session.keys(&[CONTROL, Z]);
    session.reopen(&[]);
    chord(conn, root, &[CONTROL, Y]);
    session.daemon.until("Ctrl+Y to redo abc", || {
        (session.label_end() == abc).then_some(())
    });
    session.reopen(&[D]);
    chord(conn, root, &[CONTROL, C]);
    let copied = session
        .daemon
        .until("Ctrl+C to put a PNG on the clipboard", || {
            let png = clipboard(conn, root, b"image/png")?;
            Some(image::load_from_memory(&png).ok()?.to_rgb8())
        });
    // The copied label's end, in canvas pixels, is abcd's on screen:
    // the composite's text ends 4 px short of the stage's here, and a
    // copy of abc would end about 16 px short.
    let copied = label_columns(&copied)
        .last()
        .map(|&x| (session.screen_at((x as f32, LABEL.1)).0 - session.at.0) as usize);
    session.daemon.settle();
    let abcd = session.label_end();
    assert!(
        past(abcd, abc) && copied.zip(abcd).is_some_and(|(c, s)| c.abs_diff(s) <= 6),
        "the copied label ends at {copied:?}, abcd on screen at {abcd:?}, abc at {abc:?}"
    );
}

// WHY: Ctrl+S and Ctrl+W save and close the editor with a label open,
// as Done does, and the saved file holds the label. An entry that
// takes the keys as typing leaves the editor open.
#[test]
fn ctrl_s_saves_with_a_label_open() {
    saves_with_a_label_open("ctrl_s_saves_with_a_label_open", S);
}

#[test]
fn ctrl_w_saves_with_a_label_open() {
    saves_with_a_label_open("ctrl_w_saves_with_a_label_open", W);
}

fn saves_with_a_label_open(case: &str, key: u32) {
    let Some(session) = Session::open(case) else {
        return;
    };
    session.type_label(&[A, B]);
    let saved = session.save_with(&[CONTROL, key]);
    let ink = label_columns(&saved).len();
    assert!(
        ink > 5,
        "the saved file holds {ink} red columns of the label"
    );
}

/// The image columns with red in the rows of a label at LABEL.
fn label_columns(img: &image::RgbImage) -> Vec<u32> {
    let (x, y) = (LABEL.0 as u32, LABEL.1 as u32);
    (x.saturating_sub(2)..(x + 120).min(img.width()))
        .filter(|&px| (y - 20..y + 6).any(|py| red(img.get_pixel(px, py).0)))
        .collect()
}

/// Draw the rectangle and select it with the Select tool: a click on its
/// left side.
fn selected(case: &str) -> Option<Session> {
    let session = Session::open(case)?;
    session.draw(R, RECT);
    tap(&session.conn, session.root, V);
    session.daemon.settle();
    let side = (RECT[0].0, (RECT[0].1 + RECT[1].1) / 2.0);
    click(&session.conn, session.root, session.screen_at(side));
    session.daemon.settle();
    Some(session)
}

#[test]
fn selecting_a_mark_adds_no_undo_step() {
    let Some(session) = selected("selecting_a_mark_adds_no_undo_step") else {
        return;
    };
    let before = std::fs::read(&session.file).unwrap();
    // The one undo step is the rectangle's.
    session.keys(&[CONTROL, Z]);
    wm_close(&session.conn, session.editor);
    assert_untouched(
        session,
        &before,
        "closing after selecting the mark and one Undo",
    );
}

#[test]
fn the_arrow_keys_nudge_the_selected_mark_one_undo_step_each() {
    let Some(session) = selected("the_arrow_keys_nudge_the_selected_mark_one_undo_step_each")
    else {
        return;
    };
    session.keys(&[SHIFT, RIGHT]);
    session.keys(&[SHIFT, RIGHT]);
    session.keys(&[CONTROL, Z]);
    let saved = session.save();
    assert_rect(
        &saved,
        (RECT[0].0 as u32 + 10, RECT[1].0 as u32 + 10),
        (RECT[0].1 as u32, RECT[1].1 as u32),
        "the rectangle after two Shift+Right nudges and one Undo",
    );
}

// WHY: a selection is the Select tool's. Another tool clears it, so the
// keys that act on a selected mark (Delete, the arrows, the options
// bar's) act on the next mark once a drawing tool is picked. A
// selection that survives the switch deletes, moves, or restyles the
// first rectangle here.
#[test]
fn picking_a_drawing_tool_clears_the_selection() {
    let Some(session) = selected("picking_a_drawing_tool_clears_the_selection") else {
        return;
    };
    session.keys(&[R]);
    session.keys(&[DELETE]);
    session.keys(&[SHIFT, RIGHT]);
    session.keys(&[BRACKET_RIGHT]);
    session.draw(R, SECOND);
    let saved = session.save();
    assert_rect(
        &saved,
        (RECT[0].0 as u32, RECT[1].0 as u32),
        (RECT[0].1 as u32, RECT[1].1 as u32),
        "the first rectangle after R, Delete, Shift+Right, and ]",
    );
    let y = ((SECOND[0].1 + SECOND[1].1) / 2.0) as u32;
    let x = SECOND[0].0 as u32;
    let [r, g, b] = (x - 3..x + 4)
        .map(|x| saved.get_pixel(x, y).0)
        .max_by_key(|p| p[0])
        .unwrap();
    assert!(
        r > 230 && (140..180).contains(&g) && b < 40,
        "the second rectangle's left side is [{r}, {g}, {b}], not orange"
    );
}

/// The ink across the image's vertical edge at `x` on `rows`: the mean
/// over the rows of each pixel's red channel above the image's, as a
/// share of the most it can be, summed within 8 px of `x`. A stroke's
/// antialiasing spreads its ink, not its amount, so this is the
/// stroke's width in image pixels.
fn edge_ink(img: &image::RgbImage, x: u32, rows: std::ops::Range<u32>) -> f32 {
    let n = rows.len() as f32;
    let total: f32 = rows
        .flat_map(|y| (x - 8..=x + 8).map(move |x| (x, y)))
        .map(|(x, y)| {
            let r = img.get_pixel(x, y).0[0];
            r.saturating_sub(IMAGE[0]) as f32 / (255 - IMAGE[0]) as f32
        })
        .sum();
    total / n
}

// WHY: the options bar sets the color, stroke width, and fill of the
// next mark and of the selected one, each change one undo step. A
// change that skips the selected mark, leaves its old shape on the
// canvas or in the pixels a copy or save takes, folds into another
// step, or bypasses the history (so Undo takes back the rectangle
// itself) copies or saves another image here. The reference is a
// second rectangle drawn fresh with the same options.
#[test]
fn the_options_bar_restyles_the_selected_mark_one_undo_step_each() {
    let Some(session) = selected("the_options_bar_restyles_the_selected_mark_one_undo_step_each")
    else {
        return;
    };
    // Orange, the widest stroke, and fill.
    session.keys(&[BRACKET_RIGHT]);
    session.keys(&[THREE]);
    session.keys(&[F]);
    let center = |s: &Session| {
        let (w, _, px) = s.canvas();
        let x = (s.origin.0 as f32 + 150.0 * s.scale) as usize;
        let y = (s.origin.1 as f32 + 140.0 * s.scale) as usize;
        px[y * w + x]
    };
    session
        .daemon
        .until("F to fill the selected rectangle", || {
            let [b, g, r] = center(&session);
            (r > 230 && (140..180).contains(&g) && b < 40).then_some(())
        });
    // The copied image is the one shown: filled orange, no red left.
    session.keys(&[CONTROL, C]);
    let copied = session
        .daemon
        .until("Ctrl+C to put a PNG on the clipboard", || {
            let png = clipboard(&session.conn, session.root, b"image/png")?;
            Some(image::load_from_memory(&png).ok()?.to_rgb8())
        });
    let [r, g, b] = copied.get_pixel(150, 140).0;
    assert_eq!(
        (
            r > 230 && (140..180).contains(&g) && b < 40,
            copied.pixels().any(|p| p.0[0] > 230 && p.0[1] < 100),
        ),
        (true, false),
        "the copied image: the restyled rectangle's center orange, and red anywhere"
    );
    // Undo takes back the fill only.
    session.keys(&[CONTROL, Z]);
    session.daemon.until("Undo to take back the fill", || {
        let [b, g, r] = center(&session);
        ([r, g, b] == IMAGE).then_some(())
    });
    // Undo cleared the selection: F turns fill off for the next mark.
    session.keys(&[F]);
    session.draw(R, SECOND);
    let saved = session.save();
    let orange = |[r, g, b]: [u8; 3]| r > 230 && (140..180).contains(&g) && b < 40;
    let y = ((RECT[0].1 + RECT[1].1) / 2.0) as u32;
    let row = |x: std::ops::Range<f32>| {
        (x.start as u32..x.end as u32)
            .map(|x| saved.get_pixel(x, y).0)
            .collect::<Vec<_>>()
    };
    let left = row(RECT[0].0 - 3.0..RECT[0].0 + 4.0);
    let right = row(RECT[1].0 - 3.0..RECT[1].0 + 4.0);
    let rect = edge_ink(&saved, RECT[0].0 as u32, 110..170);
    let second = edge_ink(&saved, SECOND[0].0 as u32, 230..290);
    assert_eq!(
        (
            left.iter().any(|&p| orange(p)),
            right.iter().any(|&p| orange(p)),
            saved.pixels().any(|p| p.0[0] > 230 && p.0[1] < 100),
            saved.get_pixel(150, y).0,
            (rect - second).abs() < 0.5,
        ),
        (true, true, false, IMAGE, true),
        "the restyled rectangle: orange at its left and right sides, red \
         anywhere, its center, and its stroke {rect} px wide as the fresh \
         rectangle's {second} px"
    );
}

// WHY: selecting a mark shows its color, stroke width, and fill in the
// options bar, so a change steps from the mark's own setting and the
// next mark is drawn as the selected one was. A selection that leaves
// the bar as it was steps the mark from the last-used color here (to
// green, not orange) and draws the third rectangle thick, filled, or
// green.
#[test]
fn selecting_a_mark_shows_its_style_in_the_options_bar() {
    let Some(session) = Session::open("selecting_a_mark_shows_its_style_in_the_options_bar") else {
        return;
    };
    // Red, the middle stroke, no fill.
    session.draw(R, RECT);
    // Yellow, the widest stroke, fill.
    session.keys(&[BRACKET_RIGHT]);
    session.keys(&[BRACKET_RIGHT]);
    session.keys(&[THREE]);
    session.keys(&[F]);
    session.draw(R, SECOND);
    // Select the red rectangle, step its color, and deselect it.
    tap(&session.conn, session.root, V);
    session.daemon.settle();
    let side = (RECT[0].0, (RECT[0].1 + RECT[1].1) / 2.0);
    click(&session.conn, session.root, session.screen_at(side));
    session.daemon.settle();
    session.keys(&[BRACKET_RIGHT]);
    session.keys(&[ESCAPE]);
    session.draw(R, THIRD);
    let saved = session.save();
    let orange = |[r, g, b]: [u8; 3]| r > 230 && (140..180).contains(&g) && b < 40;
    let at = |x: f32, y: f32| saved.get_pixel(x as u32, y as u32).0;
    let side_of = |c: [(f32, f32); 2]| {
        let y = (c[0].1 + c[1].1) / 2.0;
        (c[0].0 as u32 - 3..c[0].0 as u32 + 4).any(|x| orange(saved.get_pixel(x, y as u32).0))
    };
    let center = |c: [(f32, f32); 2]| at((c[0].0 + c[1].0) / 2.0, (c[0].1 + c[1].1) / 2.0);
    let [r, g, b] = center(SECOND);
    let rect = edge_ink(&saved, RECT[0].0 as u32, 110..170);
    let third = edge_ink(&saved, THIRD[0].0 as u32, 70..130);
    assert_eq!(
        (
            side_of(RECT),
            r > 230 && g > 200 && b < 40,
            side_of(THIRD),
            center(THIRD),
            (rect - third).abs() < 0.5,
        ),
        (true, true, true, IMAGE, true),
        "the first rectangle's side orange, the second's center yellow, the \
         third's side orange, the third's center, and the third's stroke \
         {third} px wide as the first's {rect} px"
    );
}

#[test]
fn ctrl_c_copies_the_image_as_shown() {
    let Some(session) = Session::open("ctrl_c_copies_the_image_as_shown") else {
        return;
    };
    session.draw(R, RECT);
    session.keys(&[CONTROL, C]);
    let copied = session
        .daemon
        .until("Ctrl+C to put a PNG on the clipboard", || {
            let png = clipboard(&session.conn, session.root, b"image/png")?;
            Some(image::load_from_memory(&png).ok()?.to_rgb8())
        });
    assert_original(&copied, "the image Ctrl+C copied");
}

#[test]
fn escape_over_unsaved_changes_asks_before_it_discards() {
    let Some(session) = Session::open("escape_over_unsaved_changes_asks_before_it_discards") else {
        return;
    };
    let before = std::fs::read(&session.file).unwrap();
    let blank = session.status_line();
    session.draw(R, RECT);
    let asked = std::time::Instant::now();
    session.escape_keeps_open("the first Escape");
    session.daemon.until("the discard prompt to show", || {
        (session.status_line() != blank).then_some(())
    });
    // A new mark is work the prompt did not warn about: it removes the
    // prompt, well before the prompt would leave on its own.
    session.draw(R, SECOND);
    assert_eq!(
        session.status_line(),
        blank,
        "the discard prompt outlived the new mark"
    );
    session.escape_keeps_open("the first Escape after a new mark");
    assert!(
        asked.elapsed() < PROMPT_FOR,
        "the steps after the first Escape took {:?}, past the prompt's {PROMPT_FOR:?}",
        asked.elapsed()
    );
    // The prompt leaves on its own, and the next press asks again.
    session
        .daemon
        .until("the discard prompt to show again", || {
            (session.status_line() != blank).then_some(())
        });
    session.daemon.until("the discard prompt to leave", || {
        (session.status_line() == blank).then_some(())
    });
    session.escape_keeps_open("the first Escape after the prompt left");
    tap(&session.conn, session.root, ESCAPE);
    assert_untouched(session, &before, "the second Escape");
}

#[test]
fn a_copy_report_leaves_the_status_line() {
    let Some(session) = Session::open("a_copy_report_leaves_the_status_line") else {
        return;
    };
    let blank = session.status_line();
    // Copy Path is the Copy menu's third row.
    session.open_copy();
    for _ in 0..3 {
        session.keys(&[DOWN]);
    }
    session.keys(&[RETURN]);
    let shown = session
        .daemon
        .until("Copy Path to report on the status line", || {
            (session.status_line() != blank).then(std::time::Instant::now)
        });
    session.daemon.until("the copy report to leave", || {
        (session.status_line() == blank).then_some(())
    });
    assert!(
        shown.elapsed() < PROMPT_FOR + OUTRO_PAST,
        "the copy report stayed {:?}, past {PROMPT_FOR:?}",
        shown.elapsed()
    );
}

#[test]
fn a_decode_failure_stays_on_the_status_line() {
    let case = "a_decode_failure_stays_on_the_status_line";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.png");
    image::RgbImage::from_pixel(SIZE.0, SIZE.1, image::Rgb(IMAGE))
        .save(&file)
        .unwrap();
    // The header the editor sizes its window from, and no pixels.
    let png = std::fs::read(&file).unwrap();
    std::fs::write(&file, &png[..64]).unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&[OsStr::new("--annotate"), file.as_os_str()]);
    let editor = daemon.until("the editor to open", || {
        windows_of(&conn, root, EDITOR).first().copied()
    });
    // The status line's text shows as more than one color.
    let shown = || {
        let (w, h, px) = client(&conn, root, editor);
        let px = &px;
        let left = status_left(w);
        let mut line = (h - STATUS_BOTTOM - STATUS_H..h - STATUS_BOTTOM)
            .flat_map(|y| (left..left + STATUS_W).map(move |x| px[y * w + x]));
        let first = line.next()?;
        line.any(|p| p != first).then_some(())
    };
    daemon.until("the decode failure to show on the status line", &shown);
    std::thread::sleep(PROMPT_FOR + OUTRO_PAST);
    assert!(
        shown().is_some(),
        "the decode failure left the status line of an editor with no image"
    );
}
