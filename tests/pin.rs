//! WHY: a pin opened at one fixed corner whatever it showed, and a
//! second pin opened on the same pixel as the first and hid it. A pin
//! opens centered on the screen rect its capture came from, so a pin at
//! native scale covers those pixels exactly, and a pin that would open
//! on another pin's origin steps down and right. The first case pins
//! two region captures of one rect through the toast's context menu;
//! the second pins a library capture twice through the card's context
//! menu, which opens each pin centered on the library window. Not
//! covered: a pin scaled down to its 720x540 cap, a window capture's
//! pin, a pin with no rect (a toast forwarded with `--toast`), which
//! `src/bin/iris/pin/tests.rs` places; a window manager that places
//! windows itself; Wayland, where the compositor places every window.
//! The cases run only with `IRIS_X11_TEST_DISPLAY` set and need `Xvfb`;
//! without it a case prints that it did not run.

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
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, entries, store, xvfb_sized, Daemon};
use x11::{click, click_with, client, drag, extent, move_to, paint_root, rect, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;
const TOAST: &str = "Screenshot - iris";
const OVERLAY: &str = "Capture - iris";
const PIN: &str = "Pin - iris";
const LIBRARY: &str = "Library - iris";
const RETURN: u32 = 0xff0d;
const DOWN: u32 = 0xff54;
/// XK_Control_L, XK_c, XK_w, XK_0, XK_minus, and XK_equal.
const CONTROL: u32 = 0xffe3;
const C: u32 = 0x63;
const W: u32 = 0x77;
const ZERO: u32 = 0x30;
const MINUS: u32 = 0x2d;
const EQUAL: u32 = 0x3d;
/// The wheel's up and down buttons.
const WHEEL_UP: u8 = 4;
const WHEEL_DOWN: u8 = 5;

/// The selection both captures drag, corner to corner.
const FROM: (i16, i16) = (200, 200);
const TO: (i16, i16) = (520, 440);

/// The toast's card sits BLEED in from its window's top-left corner.
/// Its context menu is MENU_W wide, MENU_PAD above its first row, rows
/// MENU_ROW_H tall, ROWS of them, and stays 2 px inside the window
/// (theme::CARD_BLEED, widgets::menu). Pin to Screen is row PIN_ROW.
const BLEED: i16 = 44;
const MENU_W: i16 = 168;
const MENU_PAD: i16 = 5;
const MENU_ROW_H: i16 = 24;
const ROWS: i16 = 6;
const PIN_ROW: i16 = 3;

/// The pins' step down and right (pin::CASCADE).
const CASCADE: i16 = 24;

/// The library capture's one color, [r, g, b, a], and its size.
const CAPTURE: [u8; 4] = [40, 90, 200, 255];
const CAPTURE_SIZE: (u32, u32) = (64, 48);
/// The library card menu's Pin to Screen row: Open, Quick Look, Pin.
const LIBRARY_PIN_ROW: usize = 3;

#[test]
fn a_library_pin_opens_over_the_library_and_a_second_one_cascades() {
    let case = "a_library_pin_opens_over_the_library_and_a_second_one_cascades";
    if !enabled() {
        return;
    }
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
    let shot = dir.path().join("shots").join("one.png");
    std::fs::create_dir_all(shot.parent().unwrap()).unwrap();
    image::RgbaImage::from_pixel(CAPTURE_SIZE.0, CAPTURE_SIZE.1, image::Rgba(CAPTURE))
        .save(&shot)
        .unwrap();
    store(dir.path(), &[shot]);
    daemon.send(&["--library"]);
    let library = daemon.until("the library window to map", || {
        windows_of(&conn, root, LIBRARY).first().copied()
    });
    let [r, g, b, _] = CAPTURE;
    let (left, top, right, bottom) = daemon.until("the card's thumbnail to draw", || {
        extent(&client(&conn, root, library), [r, g, b])
    });
    daemon.settle();
    let (lx, ly, lw, lh) = rect(&conn, root, library);
    let at = conn
        .translate_coordinates(library, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let card = (
        at.dst_x + ((left + right) / 2) as i16,
        at.dst_y + ((top + bottom) / 2) as i16,
    );
    let centered = |offset: i16, extent: u16, size: u32| {
        offset + ((f32::from(extent) - size as f32) / 2.0).round() as i16
    };
    let first = (
        centered(lx, lw, CAPTURE_SIZE.0),
        centered(ly, lh, CAPTURE_SIZE.1),
    );
    let mut pins: Vec<u32> = Vec::new();
    for n in 0..2i16 {
        click_with(&conn, root, card, 3);
        daemon.settle();
        // The pointer leaves the menu, so only the keys light a row.
        move_to(&conn, root, (at.dst_x + 4, at.dst_y + 4));
        daemon.settle();
        for _ in 0..LIBRARY_PIN_ROW {
            tap(&conn, root, DOWN);
        }
        daemon.settle();
        tap(&conn, root, RETURN);
        let pin = daemon.until(&format!("library pin {n} to open"), || {
            windows_of(&conn, root, PIN)
                .into_iter()
                .find(|w| !pins.contains(w))
        });
        daemon.settle();
        let (x, y, w, h) = rect(&conn, root, pin);
        let want = (first.0 + n * CASCADE, first.1 + n * CASCADE);
        assert_eq!(
            ((x, y), (u32::from(w), u32::from(h))),
            (want, CAPTURE_SIZE),
            "library pin {n} opened at {:?}, {w}x{h}, not at {want:?} over the library at \
             {:?}",
            (x, y),
            (lx, ly, lw, lh)
        );
        pins.push(pin);
    }
}

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

/// WHY: a pin had no way to copy the image it shows: the capture's own
/// copy was gone from the clipboard once anything else was copied, and
/// the pin answered only Escape, not the close shortcut every other
/// iris window takes. The case takes the clipboard from the daemon,
/// presses Ctrl+C on a pin, reads the pinned capture back from the
/// clipboard and the pin's status pill on the screen, then closes the
/// pin with Ctrl+W. Not covered: a failed copy, whose error the same
/// pill states.
#[test]
fn ctrl_c_copies_a_pin_and_ctrl_w_closes_it() {
    let case = "ctrl_c_copies_a_pin_and_ctrl_w_closes_it";
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
    let size = capture(&conn, root, &daemon, dir.path(), 1);
    pin_from_toast(&conn, root, &daemon);
    let pin = daemon.until("the pin to open", || {
        windows_of(&conn, root, PIN).first().copied()
    });
    // With no window manager the keyboard follows the pointer.
    let (x, y, w, h) = rect(&conn, root, pin);
    move_to(&conn, root, (x + w as i16 / 2, y + h as i16 / 2));
    daemon.settle();
    let shown = client(&conn, root, pin);

    // The capture put itself on the clipboard; another client takes it.
    let clipboard = conn
        .intern_atom(false, b"CLIPBOARD")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let other = conn.generate_id().unwrap();
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        other,
        root,
        -1,
        -1,
        1,
        1,
        0,
        x11rb::protocol::xproto::WindowClass::INPUT_ONLY,
        x11rb::COPY_FROM_PARENT,
        &x11rb::protocol::xproto::CreateWindowAux::new(),
    )
    .unwrap();
    conn.set_selection_owner(other, clipboard, x11rb::CURRENT_TIME)
        .unwrap();
    conn.flush().unwrap();
    assert_eq!(x11::clipboard_owner(&conn), other);

    x11::chord(&conn, root, &[CONTROL, C]);
    daemon.until("the pin's copy to own the clipboard", || {
        let owner = x11::clipboard_owner(&conn);
        (owner != other && owner != x11rb::NONE).then_some(())
    });
    let png = daemon.until("the copied image", || {
        x11::clipboard(&conn, root, b"image/png")
    });
    let copied = image::load_from_memory(&png).unwrap().into_rgb8();
    assert_eq!(
        (copied.width() as u16, copied.height() as u16),
        size,
        "the clipboard holds another image"
    );
    let [_, r, g, b] = DESKTOP.to_be_bytes();
    assert!(
        copied.pixels().all(|p| p.0 == [r, g, b]),
        "the clipboard holds pixels the pin does not show"
    );
    daemon.until("the pin to state the copy", || {
        (pill_strip(&client(&conn, root, pin)) != pill_strip(&shown)).then_some(())
    });
    x11::chord(&conn, root, &[CONTROL, W]);
    daemon.until("Ctrl+W to close the pin", || {
        windows_of(&conn, root, PIN).is_empty().then_some(())
    });
}

/// WHY: a pin showed the pointing hand of a link over its whole surface,
/// where a press moves the window as a title bar does, so the surface
/// and its close control looked alike. The case hovers the pin's
/// surface, then its close control, and waits for the cursor that X
/// shows to change. Not covered: which cursor the surface shows, past
/// differing from the close control's; Wayland, where the compositor
/// draws the cursor.
#[test]
fn a_pins_surface_shows_another_cursor_than_its_close_control() {
    let case = "a_pins_surface_shows_another_cursor_than_its_close_control";
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
    capture(&conn, root, &daemon, dir.path(), 1);
    pin_from_toast(&conn, root, &daemon);
    let pin = daemon.until("the pin to open", || {
        windows_of(&conn, root, PIN).first().copied()
    });
    daemon.settle();
    let (x, y, w, h) = rect(&conn, root, pin);
    move_to(&conn, root, (x + w as i16 / 2, y + h as i16 / 2));
    daemon.settle();
    let surface = cursor(&conn);
    // The close control: 22 px, 8 px in from the top-left corner.
    move_to(&conn, root, (x + 8 + 11, y + 8 + 11));
    daemon.until("the close control to show another cursor", || {
        (cursor(&conn) != surface).then_some(())
    });
}

/// WHY: a pin kept the size it opened at, so a capture scaled down to
/// the 720x540 cap could not be read at native scale, and a pin could
/// not be made smaller to sit beside the work it is a reference for.
/// The case pins a 320x240 capture, wheels it larger, presses `0`, `-`,
/// and `=`, then wheels it down to its floor and once past it, reading
/// the window's rect after each: the top-left corner stays put and the
/// image's shape holds. Not covered: a pinch; the ceiling, which
/// `src/bin/iris/pin/tests.rs` sets; the percent the pill states.
#[test]
fn the_wheel_and_keys_zoom_a_pin_from_its_corner() {
    let case = "the_wheel_and_keys_zoom_a_pin_from_its_corner";
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
    let size = capture(&conn, root, &daemon, dir.path(), 1);
    assert_eq!(size, (320, 240), "the capture's size");
    pin_from_toast(&conn, root, &daemon);
    let pin = daemon.until("the pin to open", || {
        windows_of(&conn, root, PIN).first().copied()
    });
    daemon.settle();
    let (x, y, _, _) = rect(&conn, root, pin);
    // With no window manager the keyboard follows the pointer: the
    // pointer stays over the pin while it is at least 256x192.
    move_to(&conn, root, (x + 120, y + 90));
    daemon.settle();
    let sized = |what: &str, want: (u16, u16)| {
        daemon.until(what, || {
            let (px, py, w, h) = rect(&conn, root, pin);
            ((w, h) == want).then_some((px, py))
        })
    };
    click_with(&conn, root, (x + 120, y + 90), WHEEL_UP);
    let (gx, gy, gw, gh) = daemon.until("the wheel to zoom the pin in", || {
        let grown = rect(&conn, root, pin);
        (grown.2 > 320).then_some(grown)
    });
    tap(&conn, root, ZERO);
    let native = sized("0 to return the pin to native scale", (320, 240));
    tap(&conn, root, MINUS);
    let smaller = sized("- to zoom the pin out a step", (256, 192));
    tap(&conn, root, EQUAL);
    let larger = sized("= to zoom the pin in a step", (320, 240));
    // The floor: a shorter edge of 48 px. The wheel turns over the
    // bottom-right corner, clear of the close control.
    let corner = |(px, py, w, h): (i16, i16, u16, u16)| (px + w as i16 - 12, py + h as i16 - 12);
    let floor = daemon.until("the wheel to zoom the pin down to 64x48", || {
        let now = rect(&conn, root, pin);
        if (now.2, now.3) == (64, 48) {
            return Some((now.0, now.1));
        }
        click_with(&conn, root, corner(now), WHEEL_DOWN);
        None
    });
    click_with(&conn, root, corner(rect(&conn, root, pin)), WHEEL_DOWN);
    daemon.settle();
    let past = rect(&conn, root, pin);
    assert_eq!(
        (
            (gx, gy),
            (f32::from(gw) * 0.75 - f32::from(gh)).abs() <= 1.0,
            native,
            smaller,
            larger,
            floor,
            past,
        ),
        ((x, y), true, (x, y), (x, y), (x, y), (x, y), (x, y, 64, 48)),
        "the pin wheeled in to {gw}x{gh} at ({gx}, {gy}) from 320x240 at ({x}, {y})"
    );
}

/// The cursor X shows: its size, hotspot, and ARGB pixels.
fn cursor(conn: &RustConnection) -> (u16, u16, u16, u16, Vec<u32>) {
    use x11rb::protocol::xfixes::ConnectionExt as _;
    conn.xfixes_query_version(5, 0).unwrap().reply().unwrap();
    let image = conn.xfixes_get_cursor_image().unwrap().reply().unwrap();
    (
        image.width,
        image.height,
        image.xhot,
        image.yhot,
        image.cursor_image,
    )
}

/// The pixels of a pin's bottom edge, where its status pill shows,
/// centered: the full width, 40 px up from the bottom edge.
fn pill_strip((w, h, px): &(usize, usize, Vec<[u8; 3]>)) -> Vec<[u8; 3]> {
    px[h.saturating_sub(40) * w..h * w].to_vec()
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
