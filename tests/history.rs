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
//! Not covered: the two flips, which go through the rotation's path;
//! redo after a rotation or Clear, which goes through the crop's swap;
//! the Library's context menu, whose keys `widgets::menu_key` runs as
//! these menus'; Windows and macOS. The cases run only with
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
use std::path::PathBuf;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, entries, store, stored, xvfb_sized, Daemon, Server};
use x11::{
    chord, click, client, clipboard_owner, clipboard_text, drag, extent, tap, windows_of, wm_close,
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
/// The rectangle's corners and the crop's, in image pixels.
const RECT: [(f32, f32); 2] = [(100.0, 100.0), (200.0, 180.0)];
const CROP: [(f32, f32); 2] = [(50.0, 50.0), (300.0, 250.0)];
/// The More menu's geometry, from `theme` and `widgets::menu`: the
/// menu drops 4 px under the 28 px button centered in the 52 px toolbar,
/// is snapped 8 px inside the window's right edge, and is 168 px wide
/// with 5 px of padding, 24 px rows, and 9 px separators.
const MENU_TOP: i16 = (52 - 28) / 2 + 28 + 4;
const MENU_W: i16 = 168;
const MENU_CENTER_FROM_RIGHT: i16 = 8 + MENU_W / 2;
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
        let w = self.canvas().0 as i16;
        let left = w - MENU_CENTER_FROM_RIGHT - MENU_W / 2;
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
        let w = self.canvas().0 as i16;
        let y = MENU_TOP
            + MENU_PAD
            + item.rows * MENU_ROW
            + item.separators * MENU_SEPARATOR
            + MENU_ROW / 2;
        let x = w - MENU_CENTER_FROM_RIGHT;
        click(&self.conn, self.root, (self.at.0 + x, self.at.1 + y));
        self.daemon.settle();
    }

    /// Open the Copy menu: the Copy button's chevron is the first light
    /// pixels left of the 28 px More button.
    fn open_copy(&self) {
        let more = self.more_button();
        let (w, _, px) = self.canvas();
        let mid = &px[TOOLBAR_MID * w..(TOOLBAR_MID + 1) * w];
        let more_left = (more.0 - self.at.0 - 14) as usize;
        let chevron = (0..more_left)
            .rev()
            .find(|&x| mid[x].iter().all(|&c| c > 150))
            .expect("the Copy button is left of the More button");
        click(
            &self.conn,
            self.root,
            (
                self.at.0 + chevron as i16 - 2,
                self.at.1 + TOOLBAR_MID as i16,
            ),
        );
        self.daemon.settle();
    }

    /// Whether the image stands on its side: taller than wide.
    fn on_its_side(&self) -> Option<bool> {
        let (left, top, right, bottom) = extent(&self.canvas(), IMAGE)?;
        Some(bottom - top > right - left)
    }

    /// Save with Return and return the saved file.
    fn save(self) -> image::RgbImage {
        tap(&self.conn, self.root, RETURN);
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
    let saved = session.save();
    let want = (
        (CROP[1].0 - CROP[0].0) as u32,
        (CROP[1].1 - CROP[0].1) as u32,
    );
    let (w, h) = saved.dimensions();
    assert!(
        w.abs_diff(want.0) <= 2 && h.abs_diff(want.1) <= 2,
        "the file saved after redoing the crop is {w}x{h}, not about {}x{}",
        want.0,
        want.1
    );
    let off = |p: (f32, f32)| ((p.0 - CROP[0].0) as u32, (p.1 - CROP[0].1) as u32);
    let (a, b) = (off(RECT[0]), off(RECT[1]));
    assert_rect(&saved, (a.0, b.0), (a.1, b.1), "after redoing the crop");
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
