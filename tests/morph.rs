//! WHY: the editor opens from the toast in a morph, the image flying from
//! the card to its place while the editor's background and chrome fade
//! in, and it closes in a fade. Both show the window's transparent
//! pixels. On an X11 screen with no compositing manager nothing blends
//! them, and the X server shows each as its colour times its alpha: the
//! editor opened as a black window that lightened to its background, and
//! closed by darkening to black. There the editor opens and closes at
//! once. The classes closed here:
//!
//! - an editor with no compositing manager that shows any colour but its
//!   background where its background shows, as it opens or closes;
//! - an editor under a compositing manager that no longer fades in or out.
//!
//! Each case runs on a private Xvfb with no window manager, whose root
//! shows a solid colour; the case's connection stands in for the
//! compositing manager (it owns the screen's `_NET_WM_CM_S<n>`
//! selection, EWMH). The X server blends nothing in either case, so a
//! fade shows under the stand-in as it does with no compositing manager.
//! Not covered: the image's flight and the chrome's fade, which a sample
//! of the background does not see; `--annotate`, which opens the editor
//! with no morph; Wayland, Windows, and macOS, which always blend. The
//! cases run only with `IRIS_X11_TEST_DISPLAY` set and need `Xvfb`;
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

use std::path::Path;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt as _, ImageFormat};
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, xvfb_sized, Daemon};
use x11::{click, tap, windows_of, Compositor};

/// The screen: large enough for the editor's window and the toast.
const SCREEN: &str = "1280x1024x24";

/// The root window's colour, 0xRRGGBB.
const DESKTOP: u32 = 0x5b7fa6;

/// The toast window's room for the card's shadow on the card's inner
/// sides, and its gap to the screen edge on the outer ones, in pixels at
/// scale 1 (theme::CARD_BLEED, stage::MARGIN). The toast in the default
/// bottom-right corner has its card at (BLEED, BLEED).
const BLEED: i16 = 44;
const MARGIN: i16 = 12;

/// The title of the toast's window.
const TOAST: &str = "Screenshot - iris";

/// The keysym of `Escape`, which discards the edit and closes the editor.
const ESCAPE: u32 = 0xff1b;

/// How long a sample of the editor's opening runs: past the 380 ms morph.
const OPENING: Duration = Duration::from_millis(800);

/// The longest a sample of the editor's closing runs: the harness's ten
/// seconds, past the 160 ms outro.
const CLOSING: Duration = Duration::from_secs(10);

#[test]
fn an_editor_opens_and_closes_at_once_where_no_compositor_blends_it() {
    if !enabled() {
        return;
    }
    let mut wrong = Vec::new();
    for compositor in [false, true] {
        let case = format!("editor, compositor {compositor}");
        let Some((_xvfb, display)) = xvfb_sized(&case, SCREEN) else {
            return;
        };
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().background_pixel(DESKTOP),
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

        // A capture's toast lands; a click on its card's image opens the
        // editor on the capture.
        daemon.send(&["--capture-fullscreen"]);
        let toast = daemon.until("the toast to open", || {
            windows_of(&conn, root, TOAST).first().copied()
        });
        daemon.settle();
        let (at, w, h) = placed(&conn, root, toast);
        let card = (w - BLEED - MARGIN, h - BLEED - MARGIN);
        click(
            &conn,
            root,
            (at.0 + BLEED + card.0 / 2, at.1 + BLEED + card.1 / 4),
        );
        let title = format!("{} - iris", shot(&daemon, &dir.path().join("shots")));
        let editor = daemon.until(&format!("the {title:?} window to open"), || {
            windows_of(&conn, root, &title).first().copied()
        });

        // Clear of the sidebar, of the image and its shadow wherever the
        // image flies, and of the toast: the editor's background.
        let (at, _, h) = placed(&conn, root, editor);
        let spot = (at.0 + 30, at.1 + h - 5);
        let opening = colours(&conn, root, spot, OPENING, || false);
        daemon.settle();
        let rest = pixel(&conn, root, spot);
        // With no window manager nothing gives the editor the focus; the
        // server's focus follows the pointer, which a click on the
        // background, which takes no action, leaves over the editor.
        click(&conn, root, spot);
        tap(&conn, root, ESCAPE);
        let closing = colours(&conn, root, spot, CLOSING, || {
            windows_of(&conn, root, &title).is_empty()
        });
        daemon.until(&format!("the {title:?} window to close"), || {
            windows_of(&conn, root, &title).is_empty().then_some(())
        });

        for (what, seen) in [("opening", opening), ("closing", closing)] {
            // Before the first frame and after the last, the window shows
            // nothing of its own: the screen beneath, or the black of a
            // transparent background under a compositing manager.
            let faded: Vec<u32> = seen
                .iter()
                .copied()
                .filter(|&c| c != rest && c != DESKTOP && c != 0)
                .collect();
            if compositor && faded.is_empty() {
                wrong.push(format!(
                    "compositor: the editor showed no fade {what}: {seen:06x?}"
                ));
            }
            if !compositor && seen.iter().any(|&c| c != rest && c != DESKTOP) {
                wrong.push(format!(
                    "no compositor: the editor's background {what} was {seen:06x?}, at rest {rest:06x}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "editor:\n{}", wrong.join("\n"));
}

/// The root position and the size of `window`.
fn placed(conn: &RustConnection, root: u32, window: u32) -> ((i16, i16), i16, i16) {
    let size = conn.get_geometry(window).unwrap().reply().unwrap();
    let at = conn
        .translate_coordinates(window, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    ((at.dst_x, at.dst_y), size.width as i16, size.height as i16)
}

/// The file name of the one capture in `shots`, once it lands.
fn shot(daemon: &Daemon, shots: &Path) -> String {
    daemon.until("the capture to land in its folder", || {
        std::fs::read_dir(shots)
            .ok()?
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .find(|name| name.ends_with(".png"))
    })
}

/// The colours the screen shows at `spot`, sampled until `done` or for
/// `limit`, with a run of one colour kept once.
fn colours(
    conn: &RustConnection,
    root: u32,
    spot: (i16, i16),
    limit: Duration,
    mut done: impl FnMut() -> bool,
) -> Vec<u32> {
    let start = Instant::now();
    let mut seen: Vec<u32> = Vec::new();
    loop {
        let colour = pixel(conn, root, spot);
        if seen.last() != Some(&colour) {
            seen.push(colour);
        }
        if done() || start.elapsed() >= limit {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The colour the screen shows at `spot`, 0xRRGGBB.
fn pixel(conn: &RustConnection, root: u32, spot: (i16, i16)) -> u32 {
    let image = conn
        .get_image(ImageFormat::Z_PIXMAP, root, spot.0, spot.1, 1, 1, !0)
        .unwrap()
        .reply()
        .unwrap();
    // Xvfb stores depth 24 in 32 bits a pixel, blue first.
    u32::from_le_bytes(image.data[..4].try_into().unwrap()) & 0xff_ffff
}
