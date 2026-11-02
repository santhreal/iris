//! The cursor iris's controls and drags show, read through XFixes on a
//! private Xvfb with no window manager.
//!
//! WHY: the first class closed here is "a control shows another cursor
//! than the arrow". Every button, toggle, menu row, and card showed the
//! hand a web page shows over a link, where a native app's controls show
//! the arrow; and a window that never asked for another cursor set none,
//! so X showed the root's. The cases read the arrow, the cursor theme's
//! `left_ptr`, over a window of their own, then hover controls each built
//! by another constructor and read the arrow over each: the capture
//! bar's mode buttons and its run button, dimmed and runnable, whose
//! overlay shows the crosshair everywhere else; the toast's corner disc
//! and its Copy pill; and the library's toolbar buttons. `tests/pin.rs`
//! reads the pin's surface and close control.
//!
//! The second class is "a drag shows another cursor than the one it
//! runs". A committed selection, which a press inside moves, showed the
//! crosshair, which promises a fresh selection; so did its move, the
//! editor's pan, Space held to pan, a drawn crop, and its move. Space
//! let go while another window had the focus never reached the editor,
//! which kept its pan armed. The cases read the theme's open hand,
//! closed hand, and crosshair as GPUI loads them, then read each over
//! the overlay's selection and the editor's stage, at rest, armed, and
//! mid-drag.
//!
//! The third class is "a key that types into no field hides the
//! pointer". GPUI hid the cursor at every key that types a character,
//! so Space held to pan and a tool's letter hid the hand or the I-beam
//! they set until the pointer moved. The cases read the cursor after
//! Space, after the Text tool's key, and after a letter that an open
//! label or the Settings File name field takes, which hides it until
//! the pointer moves.
//!
//! Not covered: the menus, the other settings controls, the editor's
//! options bar, the notice's poster, a crop or selection handle's
//! arrow, which `widgets/selection/tests.rs` states; Wayland, where the
//! compositor draws the cursor; Windows and macOS. The cases run only
//! with `IRIS_X11_TEST_DISPLAY` set and need Xvfb; without it a case
//! prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the layout.
#[allow(dead_code)]
#[path = "support/app_window.rs"]
mod app_window;
// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::ffi::OsStr;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AutoRepeatMode, ChangeKeyboardControlAux, ConnectionExt as _, CreateWindowAux, InputFocus,
    WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use app_window::{FIELD_W, INSET, TEMPLATE_Y};
use daemon::{enabled, xvfb_sized, Daemon};
use x11::{
    click, client, cursor, drag, extent, field_left_of, hold, let_go, move_to, paint_root,
    press_and_move, rect, release, tap, windows_of, CursorImage,
};

const DESKTOP: u32 = 0x5b7fa6;
const OVERLAY: &str = "Capture - iris";
const TOAST: &str = "Screenshot - iris";
const LIBRARY: &str = "Library - iris";
const EDITOR: &str = "plain.png - iris";
const SETTINGS: &str = "Settings - iris";

/// The names GPUI loads the open hand, the closed hand, and the
/// crosshair by, the first the theme has (gpui_linux's
/// `cursor_style_to_icon_names`).
const OPEN_HAND: &[&str] = &["openhand", "grab", "hand1"];
const CLOSED_HAND: &[&str] = &["closedhand", "grabbing", "hand2"];
const CROSSHAIR: &[&str] = &["crosshair", "cross"];
/// The names GPUI loads the I-beam by.
const IBEAM: &[&str] = &["text", "xterm"];
/// The keysyms of Space, which arms the editor's pan, `c`, which picks
/// its Crop tool, `t`, which picks its Text tool, and `a`, which a
/// label types.
const SPACE: u32 = 0x20;
const CROP: u32 = 0x63;
const TEXT: u32 = 0x74;
const LETTER: u32 = 0x61;
/// The editor's image: one colour, which the stage shows it by.
const IMAGE: [u8; 3] = [60, 140, 200];

/// The capture bar on a 1280x1024 monitor (`tests/capture_bar.rs`): its
/// left edge and its buttons' row, and the centers of cancel, entire
/// screen, record selection, and run.
const BAR_LEFT: i16 = 482;
const ROW: i16 = 1024 - 64 - 50 + 25;
const BAR: [(&str, i16); 4] = [
    ("cancel", 25),
    ("entire screen", 74),
    ("record selection", 199),
    ("run", 270),
];
const RUN: (i16, i16) = (BAR_LEFT + 270, ROW);

/// The toast's card sits BLEED in from its window's top-left corner; on
/// it, the centers of the Close disc and the Copy pill
/// (`tests/toast_actions.rs`).
const BLEED: i16 = 44;
const CLOSE: (i16, i16) = (17, 17);
const COPY: (i16, i16) = (51, 70);

/// The library's toolbar buttons' centres, 28 px apart by 8, from the
/// window's top-right corner at scale 1. With no compositing manager the
/// window takes the platform's frame and draws no window controls, so
/// New Screenshot is the trailing button, 12 px in.
const TOOLBAR: [(&str, i16); 3] = [
    ("New Screenshot", 12 + 14),
    ("Record", 12 + 28 + 8 + 14),
    ("Keyboard Shortcuts", 12 + 2 * (28 + 8) + 14),
];
const TOOLBAR_ROW: i16 = 26;

/// A daemon on its own 1280x1024 Xvfb with a one-colour desktop, and the
/// arrow its cursor theme draws.
struct Session {
    daemon: Daemon,
    conn: RustConnection,
    screen: usize,
    root: u32,
    arrow: CursorImage,
    dir: tempfile::TempDir,
    _xvfb: daemon::Server,
}

fn session(case: &str) -> Option<Session> {
    if !enabled() {
        return None;
    }
    let (xvfb, display) = xvfb_sized(case, "1280x1024x24")?;
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let arrow = x11::arrow(&conn, screen, (0, 0));
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    Some(Session {
        daemon,
        conn,
        screen,
        root,
        arrow,
        dir,
        _xvfb: xvfb,
    })
}

impl Session {
    /// Rest the pointer on `at` and assert `want`, `name`, shows there,
    /// over `what`.
    fn shows_at(&self, at: (i16, i16), want: &CursorImage, name: &str, what: &str) {
        move_to(&self.conn, self.root, at);
        self.shows(want, name, &format!("{what} at {at:?}"));
    }

    /// Assert the arrow shows over `what` at `at`.
    fn arrow_at(&self, what: &str, at: (i16, i16)) {
        self.shows_at(at, &self.arrow, "arrow", what);
    }

    /// The open hand, the closed hand, and the crosshair the theme
    /// draws, read before an iris window covers the screen's corner.
    /// The reads below tell them apart.
    fn drag_cursors(&self) -> (CursorImage, CursorImage, CursorImage) {
        let read = |names| x11::themed(&self.conn, self.screen, names, (0, 0));
        let (open, closed, cross) = (read(OPEN_HAND), read(CLOSED_HAND), read(CROSSHAIR));
        assert!(
            open != closed && open != cross && closed != cross && open != self.arrow,
            "the cursor theme draws two of the hands, the crosshair, and the arrow alike"
        );
        (open, closed, cross)
    }

    /// Open the capture overlay and wait for its frozen frame.
    fn open_overlay(&self) {
        self.daemon.send(&["--capture"]);
        self.daemon.until("the overlay to take the focus", || {
            windows_of(&self.conn, self.root, OVERLAY)
                .contains(&x11::focus(&self.conn))
                .then_some(())
        });
        self.daemon.until("the frozen frame to land", || {
            self.daemon.log().contains("frame landed").then_some(())
        });
        self.daemon.settle();
    }

    /// Open the editor on a one-colour image and wait until it draws the
    /// image: the editor's window, its origin on the screen, and the
    /// image's left, top, right, and bottom in the window.
    fn open_editor(&self) -> (u32, (i16, i16), (usize, usize, usize, usize)) {
        let file = self.dir.path().join("plain.png");
        image::RgbImage::from_pixel(600, 400, image::Rgb(IMAGE))
            .save(&file)
            .unwrap();
        self.daemon
            .send(&[OsStr::new("--annotate"), file.as_os_str()]);
        let editor = self.daemon.until("the editor to open", || {
            windows_of(&self.conn, self.root, EDITOR).first().copied()
        });
        let image = self.daemon.until("the editor to draw the image", || {
            extent(&client(&self.conn, self.root, editor), IMAGE)
                .filter(|&(left, top, right, bottom)| right - left >= 200 && bottom - top >= 100)
        });
        self.daemon.settle();
        let at = self
            .conn
            .translate_coordinates(editor, self.root, 0, 0)
            .unwrap()
            .reply()
            .unwrap();
        (editor, (at.dst_x, at.dst_y), image)
    }

    /// Assert `want`, `name`, shows now, over `what`: it shows once the
    /// daemon goes idle, or within `until`'s bound on a host whose load
    /// holds the daemon's redraw past an idle read.
    fn shows(&self, want: &CursorImage, name: &str, what: &str) {
        self.daemon.settle();
        self.sees(want, name, what);
    }

    /// `shows` without the wait for idle, for a window that redraws on
    /// a timer: an open label's caret blinks.
    fn sees(&self, want: &CursorImage, name: &str, what: &str) {
        self.daemon
            .until(&format!("{what} to show the {name}"), || {
                (cursor(&self.conn) == *want).then_some(())
            });
    }

    /// Assert the pointer shows no pixel over `what`: a field took a
    /// key, and the cursor is hidden until the pointer moves. An open
    /// label's caret blinks, so this does not wait for idle.
    fn hides(&self, what: &str) {
        self.daemon
            .until(&format!("{what} to hide the pointer"), || {
                cursor(&self.conn)
                    .4
                    .iter()
                    .all(|argb| argb >> 24 == 0)
                    .then_some(())
            });
    }
}

#[test]
fn the_capture_bar_shows_the_arrow_over_the_crosshair() {
    let Some(s) = session("the_capture_bar_shows_the_arrow_over_the_crosshair") else {
        return;
    };
    let (_, _, cross) = s.drag_cursors();
    s.open_overlay();
    // The screen outside the bar selects: the crosshair, not the arrow,
    // so the reads below tell the two apart.
    s.shows_at((300, 300), &cross, "crosshair", "the overlay's screen");
    for (button, x) in BAR {
        s.arrow_at(&format!("the bar's {button} button"), (BAR_LEFT + x, ROW));
    }
    // A selection makes the run button runnable, drawn and hovered as
    // another control than the dimmed one.
    drag(&s.conn, s.root, (200, 200), (600, 500), 20);
    s.daemon.settle();
    s.arrow_at("the bar's runnable run button", RUN);
}

#[test]
fn the_toast_shows_the_arrow_over_its_buttons() {
    let Some(s) = session("the_toast_shows_the_arrow_over_its_buttons") else {
        return;
    };
    s.daemon.send(&["--capture-fullscreen"]);
    let toast = s.daemon.until("the toast to open", || {
        windows_of(&s.conn, s.root, TOAST).first().copied()
    });
    s.daemon.settle();
    let (x, y, _, _) = rect(&s.conn, s.root, toast);
    let card = (x + BLEED, y + BLEED);
    for (button, spot) in [("Close disc", CLOSE), ("Copy pill", COPY)] {
        s.arrow_at(
            &format!("the toast's {button}"),
            (card.0 + spot.0, card.1 + spot.1),
        );
    }
}

#[test]
fn the_library_shows_the_arrow_over_its_toolbar_buttons() {
    let Some(s) = session("the_library_shows_the_arrow_over_its_toolbar_buttons") else {
        return;
    };
    s.daemon.send(&["--library"]);
    let library = s.daemon.until("the library window to map", || {
        windows_of(&s.conn, s.root, LIBRARY).first().copied()
    });
    s.daemon.settle();
    let (x, y, w, _) = rect(&s.conn, s.root, library);
    for (button, from_right) in TOOLBAR {
        s.arrow_at(
            &format!("the library's {button} button"),
            (x + w as i16 - from_right, y + TOOLBAR_ROW),
        );
    }
}

#[test]
fn a_committed_selection_shows_the_open_hand_and_its_move_the_closed_hand() {
    let case = "a_committed_selection_shows_the_open_hand_and_its_move_the_closed_hand";
    let Some(s) = session(case) else {
        return;
    };
    let (open, closed, cross) = s.drag_cursors();
    s.open_overlay();
    drag(&s.conn, s.root, (200, 200), (600, 500), 20);
    move_to(&s.conn, s.root, (400, 350));
    s.shows(&open, "open hand", "the inside of a committed selection");
    move_to(&s.conn, s.root, (900, 300));
    s.shows(&cross, "crosshair", "the screen outside the selection");
    press_and_move(&s.conn, s.root, (400, 350), (440, 380), 4);
    s.shows(&closed, "closed hand", "the selection's move");
    release(&s.conn, s.root);
    s.shows(&open, "open hand", "the moved selection");
}

#[test]
fn the_editor_shows_the_hands_for_a_pan_and_a_crop_move() {
    let case = "the_editor_shows_the_hands_for_a_pan_and_a_crop_move";
    let Some(s) = session(case) else {
        return;
    };
    let (open, closed, cross) = s.drag_cursors();
    // Space stays one press and one release however long it is held.
    s.conn
        .change_keyboard_control(
            &ChangeKeyboardControlAux::new().auto_repeat_mode(AutoRepeatMode::OFF),
        )
        .unwrap();
    let (editor, origin, (left, top, right, bottom)) = s.open_editor();
    let on = |x: usize, y: usize| (origin.0 + x as i16, origin.1 + y as i16);
    let center = on((left + right) / 2, (top + bottom) / 2);

    // The drawing tool's stage, Space held, its pan, and its end.
    move_to(&s.conn, s.root, center);
    s.shows(&cross, "crosshair", "the drawing tool's stage");
    hold(&s.conn, s.root, SPACE);
    s.shows(&open, "open hand", "the stage with Space held");
    press_and_move(&s.conn, s.root, center, (center.0 + 30, center.1 + 20), 5);
    s.shows(&closed, "closed hand", "the stage's pan");
    release(&s.conn, s.root);
    let_go(&s.conn, s.root, SPACE);
    s.shows(&cross, "crosshair", "the stage after the pan");

    // Space let go while a window of the case's own has the focus.
    hold(&s.conn, s.root, SPACE);
    s.shows(&open, "open hand", "the stage with Space held again");
    let other = s.conn.generate_id().unwrap();
    s.conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            other,
            s.root,
            0,
            0,
            40,
            40,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().override_redirect(1),
        )
        .unwrap();
    s.conn.map_window(other).unwrap();
    s.conn.sync().unwrap();
    s.conn
        .set_input_focus(InputFocus::NONE, other, x11rb::CURRENT_TIME)
        .unwrap();
    s.conn.sync().unwrap();
    s.daemon.settle();
    let_go(&s.conn, s.root, SPACE);
    s.conn
        .set_input_focus(InputFocus::NONE, editor, x11rb::CURRENT_TIME)
        .unwrap();
    s.conn.destroy_window(other).unwrap();
    s.conn.sync().unwrap();
    s.shows(
        &cross,
        "crosshair",
        "the stage after Space was let go in another window",
    );

    // A drawn crop: the open hand inside it, the closed hand while it
    // moves, and the crosshair on the image outside it. The pan moved
    // the image.
    let (left, top, right, bottom) =
        extent(&client(&s.conn, s.root, editor), IMAGE).expect("the image stays on the stage");
    tap(&s.conn, s.root, CROP);
    let (from, to) = (on(left + 60, top + 60), on(left + 360, top + 260));
    drag(&s.conn, s.root, from, to, 20);
    let inside = (from.0 + 150, from.1 + 100);
    move_to(&s.conn, s.root, inside);
    s.shows(&open, "open hand", "the inside of a drawn crop");
    press_and_move(&s.conn, s.root, inside, (inside.0 + 40, inside.1 + 30), 5);
    s.shows(&closed, "closed hand", "the crop's move");
    release(&s.conn, s.root);
    s.shows(&open, "open hand", "the moved crop");
    move_to(&s.conn, s.root, on(right - 20, bottom - 20));
    s.shows(&cross, "crosshair", "the image outside the crop");
}

#[test]
fn a_tool_key_keeps_the_pointer_and_a_letter_a_label_takes_hides_it() {
    let case = "a_tool_key_keeps_the_pointer_and_a_letter_a_label_takes_hides_it";
    let Some(s) = session(case) else {
        return;
    };
    let ibeam = x11::themed(&s.conn, s.screen, IBEAM, (0, 0));
    assert!(
        ibeam != s.arrow,
        "the cursor theme draws the I-beam and the arrow alike"
    );
    let (_, origin, (left, top, right, bottom)) = s.open_editor();
    let center = (
        origin.0 + ((left + right) / 2) as i16,
        origin.1 + ((top + bottom) / 2) as i16,
    );
    // The Text tool's key runs a command at the pointer, which stays.
    move_to(&s.conn, s.root, center);
    s.daemon.settle();
    tap(&s.conn, s.root, TEXT);
    s.shows(&ibeam, "I-beam", "the stage after the Text tool's key");
    // A press opens a label, which types the next letter: the pointer
    // hides until it moves.
    click(&s.conn, s.root, center);
    tap(&s.conn, s.root, LETTER);
    s.hides("a label that typed a letter");
    move_to(&s.conn, s.root, (center.0 + 4, center.1));
    s.sees(&ibeam, "I-beam", "the label after the pointer moved");
}

#[test]
fn a_letter_a_settings_field_takes_hides_the_pointer() {
    let case = "a_letter_a_settings_field_takes_hides_the_pointer";
    let Some(s) = session(case) else {
        return;
    };
    let ibeam = x11::themed(&s.conn, s.screen, IBEAM, (0, 0));
    s.daemon.send(&["--settings"]);
    let settings = s.daemon.until("Settings to open", || {
        windows_of(&s.conn, s.root, SETTINGS).first().copied()
    });
    s.daemon.settle();
    // The File name field, left of the card's right padding on its
    // row's center line (`settings_paste.rs`).
    let (w, _, px) = client(&s.conn, s.root, settings);
    let scale = app_window::scale(w);
    let y = (TEMPLATE_Y * scale) as usize;
    let row = &px[y * w..(y + 1) * w];
    let card = row[w - ((INSET + 6.0) * scale) as usize];
    let before = w - (INSET * scale) as usize - 1;
    let (left, right) = field_left_of(row, card, before, (FIELD_W * scale) as usize)
        .unwrap_or_else(|| panic!("the File name row has no field {FIELD_W} px wide"));
    let (x, top, _, _) = rect(&s.conn, s.root, settings);
    let field = (x + ((left + right) / 2) as i16, top + y as i16);
    s.shows_at(field, &ibeam, "I-beam", "the File name field");
    // A click opens the field, which types the next letter: the pointer
    // hides until it moves.
    click(&s.conn, s.root, field);
    tap(&s.conn, s.root, LETTER);
    s.hides("the File name field that typed a letter");
    move_to(&s.conn, s.root, (field.0 + 4, field.1));
    s.sees(&ibeam, "I-beam", "the field after the pointer moved");
}
