//! WHY: a pop-up (the toast, a notice) is a transparent window with a
//! card in it and room around the card for its shadow. On an X11 screen
//! with no compositing manager nothing blends a transparent pixel, and
//! the X server shows it black: the room around the card showed as a
//! black box. There the window takes the card as its bounding shape, and
//! the screen beneath shows through the rest; under a compositing
//! manager the window stays whole, shadow and all. A toast card that
//! slid in from past its window's edge, or out past it, left the window
//! no pixel to show; the X server then reported the window fully
//! obscured, and GPUI draws no frame for such a window: the card never
//! arrived, and the window never closed. With no compositing manager a
//! card, the toast's or a notice's, arrives and leaves at once. The
//! capture flash, a fill over every monitor, has no card to cut to, and
//! opens only under a compositing manager. The classes closed here:
//!
//! - a pop-up with no compositing manager that shows anything past its
//!   card, or cuts the card's corners short of their radius;
//! - a pop-up with no compositing manager whose card moves into its rest
//!   or out of it, or never reaches it, or whose window does not close
//!   as the card leaves;
//! - a pop-up under a compositing manager cut to a shape;
//! - a pop-up whose shadow and the room around its card take a click
//!   meant for the window beneath, or whose card does not take one;
//! - a flash that darkens every monitor with no compositing manager, or
//!   one that no longer opens under one.
//!
//! Each case runs on a private Xvfb with no window manager, whose root
//! shows a solid colour; the case's connection stands in for the
//! compositing manager (it owns the screen's `_NET_WM_CM_S<n>`
//! selection, EWMH). Not covered: the chip's shape, which `tests/chip.rs`
//! clicks and reads through with no compositing manager but does not
//! measure; the toast's menu and dismiss swipe; a
//! compositing manager that starts or exits while a pop-up is open;
//! Wayland, Windows, and macOS, which always blend. The cases run only
//! with `IRIS_X11_TEST_DISPLAY` set and need `Xvfb`; without it a case
//! prints that it did not run.

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
use x11rb::protocol::shape::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask, PropMode,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{drag, move_to, rect, tap, windows_of, Compositor};

/// The screen, and its width and height in pixels.
const SCREEN: &str = "1280x1024x24";
const SIZE: (u16, u16) = (1280, 1024);

/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;

/// A pop-up window's room for the card's shadow on the card's inner
/// sides, and its gap to the screen edge on the outer ones, in pixels at
/// scale 1 (theme::CARD_BLEED, stage::MARGIN, notice::MARGIN). A pop-up
/// in the default bottom-right corner has its card at (BLEED, BLEED).
const BLEED: i16 = 44;
const MARGIN: i16 = 12;

/// The titles of the toast's and a notice's windows.
const TOAST: &str = "Screenshot - iris";
const NOTICE: &str = "Notice - iris";

#[test]
fn a_popup_shows_only_its_card_where_no_compositor_blends_it() {
    if !enabled() {
        return;
    }
    let mut wrong = Vec::new();
    for compositor in [false, true] {
        let case = format!("pop-ups, compositor {compositor}");
        let Some((_xvfb, display)) = xvfb_sized(&case, SCREEN) else {
            return;
        };
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        // A solid desktop, and every window the daemon creates reported
        // to this connection, the flash's included.
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new()
                .background_pixel(DESKTOP)
                .event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
        )
        .unwrap();
        conn.clear_area(false, root, 0, 0, 0, 0).unwrap();
        let selection = conn
            .intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let _compositor = compositor.then(|| Compositor::start(&conn, root, selection));
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });

        // A toast that cannot open reports why in a notice, which leaves
        // after the eight seconds a failure stays.
        let broken = dir.path().join("broken.png");
        std::fs::write(&broken, "not an image").unwrap();
        daemon.send(&[std::ffi::OsStr::new("--toast"), broken.as_os_str()]);
        let (window, rest) = card(&conn, root, &daemon, NOTICE, compositor, &mut wrong);
        input(&conn, root, window, NOTICE, compositor, false, &mut wrong);
        leaves(&conn, root, &daemon, NOTICE, window, rest, &mut wrong);

        // A capture flashes, then its toast lands, and leaves after its
        // default five seconds.
        daemon.send(&["--capture-fullscreen"]);
        let (window, rest) = card(&conn, root, &daemon, TOAST, compositor, &mut wrong);
        input(&conn, root, window, TOAST, compositor, true, &mut wrong);
        let mut flashes = 0;
        while let Some(event) = conn.poll_for_event().unwrap() {
            if let Event::CreateNotify(created) = event {
                flashes += usize::from(
                    created.override_redirect && (created.width, created.height) == SIZE,
                );
            }
        }
        if flashes != usize::from(compositor) {
            wrong.push(format!(
                "compositor {compositor}: a capture opened {flashes} flash windows"
            ));
        }
        leaves(&conn, root, &daemon, TOAST, window, rest, &mut wrong);
    }
    assert!(wrong.is_empty(), "pop-ups:\n{}", wrong.join("\n"));
}

/// The work area panels leave on the screen in the second case: a 30
/// pixel top bar and a 40 pixel right dock, x, y, width, height.
const WORK_AREA: [u32; 4] = [0, 30, 1240, 934];

/// The Return key's keysym.
const RETURN: u32 = 0xff0d;

// WHY: a pop-up placed from the whole monitor rests under a panel, a
// dock, or a taskbar along the toast's corner, and the card is hidden or
// covers the panel. The class closed here: a toast, from a command line
// or from a region capture's flight, or a notice, whose window does not
// end at the corner of the work area the window manager publishes
// (`_NET_WORKAREA` of `_NET_CURRENT_DESKTOP`, EWMH), or does not end at
// the screen's corner when none is published. Not covered: the other
// corners, which share the placement with the default one; a work area
// over several monitors; Wayland, Windows, and macOS, which read theirs
// from the platform.
#[test]
fn a_popup_rests_in_the_corner_of_the_work_area() {
    if !enabled() {
        return;
    }
    let mut wrong = Vec::new();
    for published in [false, true] {
        let case = format!("work area, published {published}");
        let Some((_xvfb, display)) = xvfb_sized(&case, SCREEN) else {
            return;
        };
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let corner = if published {
            // Desktop 1 is current; desktop 0's area, the whole screen,
            // is the wrong one to read.
            let atom = |name: &str| {
                conn.intern_atom(false, name.as_bytes())
                    .unwrap()
                    .reply()
                    .unwrap()
                    .atom
            };
            let (w, h) = (u32::from(SIZE.0), u32::from(SIZE.1));
            let areas = [[0, 0, w, h], WORK_AREA].concat();
            conn.change_property32(
                PropMode::REPLACE,
                root,
                atom("_NET_WORKAREA"),
                AtomEnum::CARDINAL,
                &areas,
            )
            .unwrap();
            conn.change_property32(
                PropMode::REPLACE,
                root,
                atom("_NET_CURRENT_DESKTOP"),
                AtomEnum::CARDINAL,
                &[1],
            )
            .unwrap();
            conn.flush().unwrap();
            (
                (WORK_AREA[0] + WORK_AREA[2]) as i32,
                (WORK_AREA[1] + WORK_AREA[3]) as i32,
            )
        } else {
            (i32::from(SIZE.0), i32::from(SIZE.1))
        };
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        let mut check = |what: &str, window: u32| {
            let (x, y, w, h) = rect(&conn, root, window);
            let end = (i32::from(x) + i32::from(w), i32::from(y) + i32::from(h));
            if end != corner {
                wrong.push(format!(
                    "published {published}: the {what} window ends at {end:?}, not {corner:?}"
                ));
            }
        };

        // A notice, before a toast opens: a toast moves it aside.
        let broken = dir.path().join("broken.png");
        std::fs::write(&broken, "not an image").unwrap();
        daemon.send(&[std::ffi::OsStr::new("--toast"), broken.as_os_str()]);
        let notice = daemon.until("the notice to open", || {
            windows_of(&conn, root, NOTICE).first().copied()
        });
        daemon.settle();
        check("notice", notice);

        // A toast with no region to land beside.
        daemon.send(&["--capture-fullscreen"]);
        let toast = daemon.until("the toast to open", || {
            windows_of(&conn, root, TOAST).first().copied()
        });
        daemon.settle();
        check("full-screen toast", toast);
        daemon.until("the toast to leave", || {
            windows_of(&conn, root, TOAST).is_empty().then_some(())
        });

        // A toast landing from a region capture's flight.
        let landed = daemon.log().matches("frame landed").count();
        daemon.send(&["--capture"]);
        daemon.until("the frozen frame to land", || {
            (daemon.log().matches("frame landed").count() > landed).then_some(())
        });
        daemon.settle();
        drag(&conn, root, (200, 200), (400, 350), 40);
        tap(&conn, root, RETURN);
        let toast = daemon.until("the region's toast to open", || {
            windows_of(&conn, root, TOAST).first().copied()
        });
        daemon.settle();
        check("region toast", toast);
    }
    assert!(wrong.is_empty(), "work area:\n{}", wrong.join("\n"));
}

/// Check that a click on the room around the resting card of the pop-up
/// `window` reaches the window beneath, the root here, and, with `card`,
/// that a click on the card reaches `window`. Leaves the pointer in the
/// screen's top-left corner, off the pop-up. Pushes to `wrong` what is
/// wrong.
fn input(
    conn: &RustConnection,
    root: u32,
    window: u32,
    title: &str,
    compositor: bool,
    card: bool,
    wrong: &mut Vec<String>,
) {
    let at = conn
        .translate_coordinates(window, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let size = conn.get_geometry(window).unwrap().reply().unwrap();
    let mut spots = vec![("the room around the card", (BLEED / 2, BLEED / 2), false)];
    if card {
        let centre = (
            BLEED + (size.width as i16 - BLEED - MARGIN) / 2,
            BLEED + (size.height as i16 - BLEED - MARGIN) / 4,
        );
        spots.push(("the card", centre, true));
    }
    for (what, (x, y), takes) in spots {
        move_to(conn, root, (at.dst_x + x, at.dst_y + y));
        let under = conn.query_pointer(root).unwrap().reply().unwrap().child;
        if (under == window) != takes {
            wrong.push(format!(
                "{title:?}, compositor {compositor}: a click on {what} at ({x}, {y}) \
                 reaches window {under:#x}, the pop-up is {window:#x}"
            ));
        }
    }
    move_to(conn, root, (0, 0));
}

/// A window's bounding shape when it has one: the extents x, y, width,
/// and height.
type Shape = (i16, i16, u16, u16);

/// The bounding shape of `window`, None when it has none or is gone.
fn shape(conn: &RustConnection, window: u32) -> Option<Shape> {
    let extents = conn.shape_query_extents(window).ok()?.reply().ok()?;
    extents.bounding_shaped.then_some((
        extents.bounding_shape_extents_x,
        extents.bounding_shape_extents_y,
        extents.bounding_shape_extents_width,
        extents.bounding_shape_extents_height,
    ))
}

/// Wait up to the harness's ten seconds for the pop-up `window` titled
/// `title` to close as its card leaves. A window cut to `rest` keeps
/// that shape until it closes: with no compositor the card leaves at
/// once. Pushes to `wrong` what is wrong.
fn leaves(
    conn: &RustConnection,
    root: u32,
    daemon: &Daemon,
    title: &str,
    window: u32,
    rest: Option<Shape>,
    wrong: &mut Vec<String>,
) {
    let mut moved = None;
    daemon.until(&format!("the {title:?} window to close"), || {
        if let (Some(rest), Some(cut)) = (rest, shape(conn, window)) {
            if cut != rest {
                moved.get_or_insert(cut);
            }
        }
        windows_of(conn, root, title).is_empty().then_some(())
    });
    if let Some(cut) = moved {
        wrong.push(format!(
            "{title:?} with no compositor: took shape {cut:?} as its card left"
        ));
    }
}

/// Check the pop-up window titled `title` once it opens and its card is
/// at rest: cut to the card with no compositor, where the card takes its
/// rest as its first shape, and not cut under one. Returns the window
/// and, with no compositor, the shape it rests in; pushes to `wrong`
/// what is wrong.
fn card(
    conn: &RustConnection,
    root: u32,
    daemon: &Daemon,
    title: &str,
    compositor: bool,
    wrong: &mut Vec<String>,
) -> (u32, Option<Shape>) {
    let window = daemon.until(&format!("the {title:?} window to open"), || {
        windows_of(conn, root, title).first().copied()
    });
    let size = conn.get_geometry(window).unwrap().reply().unwrap();
    let (w, h) = (size.width as i16, size.height as i16);
    let rest = (
        BLEED,
        BLEED,
        (w - BLEED - MARGIN) as u16,
        (h - BLEED - MARGIN) as u16,
    );
    if compositor {
        // The card slides in and settles; the daemon then goes idle.
        daemon.settle();
        if let Some(cut) = shape(conn, window) {
            wrong.push(format!("{title:?} under a compositor: cut to {cut:?}"));
        }
        return (window, None);
    }
    let mut moved = None;
    daemon.until(
        &format!("the {title:?} window to take its resting card {rest:?} as its shape"),
        || match shape(conn, window) {
            Some(cut) if cut == rest => Some(()),
            Some(cut) => {
                moved.get_or_insert(cut);
                None
            }
            None => None,
        },
    );
    if let Some(cut) = moved {
        wrong.push(format!(
            "{title:?} with no compositor: took shape {cut:?} before its rest {rest:?}"
        ));
    }
    daemon.settle();
    // The screen beneath shows in the shadow's room and past the card's
    // rounded top-left corner, both outside the shape.
    let at = conn
        .translate_coordinates(window, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let pixel = |x: i16, y: i16| x11::pixel(conn, root, (at.dst_x + x, at.dst_y + y));
    let shown: Vec<((i16, i16), u32)> = [(BLEED / 2, BLEED / 2), (BLEED, BLEED)]
        .into_iter()
        .map(|(x, y)| ((x, y), pixel(x, y)))
        .filter(|&(_, colour)| colour != DESKTOP)
        .collect();
    if !shown.is_empty() {
        wrong.push(format!(
            "{title:?} with no compositor: the desktop does not show at {shown:x?}"
        ));
    }
    (window, Some(rest))
}
