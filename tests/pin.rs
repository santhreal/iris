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
use x11rb::protocol::shape::{ConnectionExt as _, SK};
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, ImageFormat, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

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

/// The 187x140 card of the FROM..TO capture's toast. Its context menu
/// is MENU_W wide, MENU_PAD above its first row, rows MENU_ROW_H tall,
/// ROWS of them with one SEPARATOR_H separator above the last, and stays
/// MENU_EDGE inside the window's right and bottom edges, on the screen's
/// edges, and MENU_ROOM inside its left and top ones, for its shadow
/// (widgets::menu, stage::actions). Pin to Screen is row PIN_ROW, above
/// the separator.
const CARD: (i16, i16) = (187, 140);
const MENU_W: i16 = 192;
const MENU_PAD: i16 = 5;
const MENU_ROW_H: i16 = 24;
const SEPARATOR_H: i16 = 9;
const ROWS: i16 = 6;
const PIN_ROW: i16 = 4;
const MENU_EDGE: i16 = 2;
const MENU_ROOM: i16 = 58;

/// The pins' step down and right (pin::CASCADE).
const CASCADE: i16 = 24;

/// The library capture's one color, [r, g, b, a], and its size.
const CAPTURE: [u8; 4] = [40, 90, 200, 255];
const CAPTURE_SIZE: (u32, u32) = (64, 48);
/// The library card menu's Pin to Screen row: Open, Quick Look, Pin.
const LIBRARY_PIN_ROW: usize = 3;
/// The room an app window keeps around its frame for its shadow under a
/// compositing manager and a window manager that lists
/// `_GTK_FRAME_EXTENTS` (theme::WINDOW_ROOM; tests/frame.rs), in pixels.
const ROOM: i16 = 40;

/// WHY, beyond the module's: a pin opened at its image's size with no
/// room around it, so where an app window floats on its shadow, under a
/// compositing manager and a window manager that lists
/// `_GTK_FRAME_EXTENTS`, a pin had none and looked flat beside it. The
/// case runs with no compositor, where a window keeps no room, and under
/// a stand-in compositor and window manager (tests/frame.rs), where it
/// keeps ROOM. In each it reads every pin's window: the frame centered on
/// the library's and the second CASCADE down and right of the first;
/// ROOM around the frame, stated in `_GTK_FRAME_EXTENTS`, with pointer
/// input only in the frame; the shadow under the frame and gone at the
/// room's outer edge; and `=` zooming the frame from its corner with the
/// room kept. Not covered: how a real window manager places a pin by its
/// frame extents; the shadow's shape beyond those two points.
#[test]
fn a_library_pin_opens_over_the_library_and_a_second_one_cascades() {
    if !enabled() {
        return;
    }
    for room in [0, ROOM] {
        library_pins(room);
    }
}

fn library_pins(room: i16) {
    // A screen the library window fits, so its pixels read back.
    let Some((_xvfb, display)) = xvfb_sized(&format!("library pins, room {room}"), "1600x1200x24")
    else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let atom = |name: &str| {
        conn.intern_atom(false, name.as_bytes())
            .unwrap()
            .reply()
            .unwrap()
            .atom
    };
    let extents_atom = atom("_GTK_FRAME_EXTENTS");
    let _compositor = (room > 0).then(|| {
        let supported: Vec<u32> = [
            "_NET_SUPPORTED",
            "_NET_WM_NAME",
            "_NET_WM_STATE",
            "_GTK_FRAME_EXTENTS",
        ]
        .into_iter()
        .map(atom)
        .collect();
        conn.change_property32(
            PropMode::REPLACE,
            root,
            atom("_NET_SUPPORTED"),
            AtomEnum::ATOM,
            &supported,
        )
        .unwrap();
        x11::wm_check(&conn, root);
        x11::Compositor::start(&conn, root, atom(&format!("_NET_WM_CM_S{screen}")))
    });
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
    let card = (
        lx + ((left + right) / 2) as i16,
        ly + ((top + bottom) / 2) as i16,
    );
    // The pin's frame centers on the library's, and the library keeps
    // the same room around its frame on every side.
    let centered = |offset: i16, extent: u16, size: u32| {
        offset + ((f32::from(extent) - size as f32) / 2.0).round() as i16
    };
    let first = (
        centered(lx, lw, CAPTURE_SIZE.0) - room,
        centered(ly, lh, CAPTURE_SIZE.1) - room,
    );
    let grown = |(w, h): (u32, u32)| (w as u16 + 2 * room as u16, h as u16 + 2 * room as u16);
    let mut pins: Vec<u32> = Vec::new();
    for n in 0..2i16 {
        click_with(&conn, root, card, 3);
        daemon.settle();
        // The pointer leaves the menu for the library's frame, so only
        // the keys light a row, and with no window manager the keys go
        // to the window under the pointer.
        move_to(&conn, root, (lx + room + 4, ly + room + 4));
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
            ((x, y), (w, h)),
            (want, grown(CAPTURE_SIZE)),
            "library pin {n} with room {room} opened at {:?}, {w}x{h}, not at {want:?} over \
             the library at {:?}",
            (x, y),
            (lx, ly, lw, lh)
        );
        let extents = conn
            .get_property(false, pin, extents_atom, AtomEnum::CARDINAL, 0, 4)
            .unwrap()
            .reply()
            .unwrap()
            .value32()
            .map(|v| v.collect::<Vec<_>>());
        let input: Vec<_> = conn
            .shape_get_rectangles(pin, SK::INPUT)
            .unwrap()
            .reply()
            .unwrap()
            .rectangles
            .iter()
            .map(|r| (r.x, r.y, r.width, r.height))
            .collect();
        let (cw, ch) = (CAPTURE_SIZE.0 as u16, CAPTURE_SIZE.1 as u16);
        assert_eq!(
            (extents, input),
            (
                (room > 0).then(|| vec![room as u32; 4]),
                vec![(room, room, cw, ch)]
            ),
            "library pin {n} with room {room}: _GTK_FRAME_EXTENTS and input shape"
        );
        if room > 0 {
            // The window's own alpha: the shadow 4 px under the frame's
            // bottom center, and none on the room's outer edge there.
            let image = conn
                .get_image(ImageFormat::Z_PIXMAP, pin, 0, 0, w, h, !0)
                .unwrap()
                .reply()
                .unwrap();
            let alpha = |(ax, ay): (i16, i16)| {
                image.data[(ay as usize * usize::from(w) + ax as usize) * 4 + 3]
            };
            let under = alpha((room + cw as i16 / 2, room + ch as i16 + 4));
            let outside = alpha((room + cw as i16 / 2, h as i16 - 1));
            assert!(
                image.depth == 32 && under >= 24 && outside <= 2,
                "library pin {n}'s shadow: alpha {under} under the frame, {outside} at the \
                 room's edge, depth {}",
                image.depth
            );
        }
        pins.push(pin);
    }
    // `=` on the pin under the pointer: the frame grows a key step from
    // its top-left corner and the room stays around it.
    let pin = pins[1];
    let (x, y, _, _) = rect(&conn, root, pin);
    move_to(&conn, root, (x + room + 32, y + room + 24));
    daemon.settle();
    tap(&conn, root, EQUAL);
    let zoomed = daemon.until("= to zoom the library pin a step", || {
        let now = rect(&conn, root, pin);
        (now.2 != grown(CAPTURE_SIZE).0).then_some(now)
    });
    assert_eq!(
        zoomed,
        (x, y, 80 + 2 * room as u16, 60 + 2 * room as u16),
        "= zoomed the 64x48 pin with room {room} from ({x}, {y})"
    );
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
/// where a press moves the window as a title bar does, and then over its
/// close control, where a native window's controls show the arrow; and
/// its close control, shown while the pointer is on the pin, went
/// transparent once the pointer reached it. The case hovers the pin's
/// surface, then its close control, and reads the arrow and the close
/// control's fill over both. Not covered: Wayland, where the compositor
/// draws the cursor.
#[test]
fn a_pin_shows_its_close_control_and_the_arrow_under_the_pointer() {
    let case = "a_pin_shows_its_close_control_and_the_arrow_under_the_pointer";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let arrow = x11::arrow(&conn, screen, (0, 0));
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
    // The surface, then the close control: 22 px, 8 px in from the
    // top-left corner. Its fill shows 7 px above its centre, clear of
    // the glyph: dark where the pin shows the desktop's blue.
    let fill = (x + 8 + 11, y + 8 + 4);
    for (part, at) in [
        ("surface", (x + w as i16 / 2, y + h as i16 / 2)),
        ("close control", (x + 8 + 11, y + 8 + 11)),
    ] {
        move_to(&conn, root, at);
        daemon.settle();
        assert!(
            x11::cursor(&conn) == arrow,
            "the pin's {part} at {at:?} shows another cursor than the arrow"
        );
        let shown = x11::pixel(&conn, root, fill);
        assert!(
            x11::channels(shown).iter().all(|&c| c < 80),
            "with the pointer on the pin's {part}, its close control shows {shown:06x}, \
             not its dark fill"
        );
    }
}

/// WHY: a pin drew no edge, so a capture pinned over a desktop of its own
/// colour ran into it with no line between them. The case pins a capture
/// of the desktop under a stand-in compositing manager, so the pin draws
/// its own shape as on a desktop with one, and reads the pin's four
/// edges, which differ from the desktop, and a point inside them, which
/// matches it. Not covered: the edge over a dark capture; a window
/// manager's frame, which an X11 pin opened with no compositor takes; and
/// Windows 11, where the platform draws it.
#[test]
fn a_pin_keeps_an_edge_over_a_desktop_of_its_own_color() {
    let case = "a_pin_keeps_an_edge_over_a_desktop_of_its_own_color";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let selection = conn
        .intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let _compositor = x11::Compositor::start(&conn, root, selection);
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    capture(&conn, root, &daemon, dir.path(), 1);
    pin_from_toast(&conn, root, &daemon);
    let pin = daemon.until("the pin to open", || {
        windows_of(&conn, root, PIN).first().copied()
    });
    move_to(&conn, root, (0, 0));
    daemon.settle();
    let (x, y, w, h) = rect(&conn, root, pin);
    let (w, h) = (w as i16, h as i16);
    let inside = x11::pixel(&conn, root, (x + w / 2, y + h / 2 + 8));
    assert_eq!(inside, DESKTOP, "the pin does not show the desktop it pins");
    for (side, at) in [
        ("left", (x, y + h / 2)),
        ("right", (x + w - 1, y + h / 2)),
        ("top", (x + w / 2, y)),
        ("bottom", (x + w / 2, y + h - 1)),
    ] {
        let edge = x11::pixel(&conn, root, at);
        let apart = x11::channels(edge)
            .iter()
            .zip(x11::channels(DESKTOP))
            .map(|(&a, b)| a.abs_diff(b))
            .max()
            .unwrap();
        assert!(
            apart >= 5,
            "the pin's {side} edge at {at:?} shows {edge:06x}, the desktop's {DESKTOP:06x}"
        );
    }
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
    let card = x11::toast_card(conn, root, toast, CARD);
    let press = (card.0 - x + 20, card.1 - y + 20);
    click_with(conn, root, (x + press.0, y + press.1), 3);
    daemon.settle();
    let menu_h = ROWS * MENU_ROW_H + SEPARATOR_H + 2 * MENU_PAD;
    let menu = (
        press.0.min(w as i16 - MENU_W - MENU_EDGE).max(MENU_ROOM),
        press.1.min(h as i16 - menu_h - MENU_EDGE).max(MENU_ROOM),
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
