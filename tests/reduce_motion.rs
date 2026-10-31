//! The toast's entrance under the desktop's reduced-motion setting,
//! driven on a private Xvfb whose session bus serves a stand-in for the
//! XDG desktop portal's `org.freedesktop.portal.Settings`.
//!
//! WHY: the class closed here is "a surface that moves while the desktop
//! asks for reduced motion". The toast's card slides in from past the
//! screen edge; under reduced motion it appears on its resting rect and
//! fades in there. Each case reads two spots while the card enters,
//! slowed with `IRIS_SLOWMO`: one in the gap between the card's resting
//! rect and the screen edge, which a sliding card crosses and a fading
//! card never covers, and one inside the resting rect, which a fading
//! card darkens to a run of partial colours. The case with reduced motion
//! off proves the gap spot sees a sliding card.
//!
//! The case's connection stands in for the compositing manager (it owns
//! `_NET_WM_CM_S<n>`), so iris animates the toast; the X server blends
//! nothing, and shows each pixel as its colour times its alpha: the
//! toast's transparent pixels are black, and the card at opacity `a` is
//! its colour times `a`.
//!
//! Not covered: the notice, the capture flight, and the editor morph,
//! which read the same setting at their own call sites; a change of the
//! setting while iris runs; the GNOME `enable-animations` key; Wayland,
//! Windows, and macOS. The cases run only with `IRIS_X11_TEST_DISPLAY`
//! set and need Xvfb and dbus-daemon; without them a case prints that it
//! did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;
use zbus::zvariant::{OwnedValue, Value};

use daemon::{enabled, session_bus, xvfb_sized, Daemon};
use x11::{move_to, paint_root, pixel, rect, windows_of, Compositor};

/// The root window's colour, 0xRRGGBB, and so the colour of a capture of
/// the screen.
const DESKTOP: u32 = 0x5b7fa6;
const TOAST: &str = "Screenshot - iris";

/// The toast's card sits BLEED in from its window's top-left corner
/// (theme::CARD_BLEED) and MARGIN in from the screen's edges
/// (stage::MARGIN). A 1280x1024 capture's card is 175x140.
const BLEED: i16 = 44;
const MARGIN: i16 = 12;
const CARD: (i16, i16) = (175, 140);

/// Every animation runs this many times slower, so the entrance spans
/// many samples.
const SLOWMO: &str = "6";

/// The longest a case samples the entrance: past the slowed 280 ms.
const ENTRANCE: Duration = Duration::from_secs(5);

/// The stand-in portal's `org.freedesktop.portal.Settings`: it holds
/// `org.freedesktop.appearance` `reduced-motion` and no other key, and
/// records that the key was read.
struct Settings {
    reduced_motion: u32,
    read: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        self.value(namespace, key)
    }

    fn read_one(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        self.value(namespace, key)
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }
}

impl Settings {
    fn value(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        if (namespace, key) != ("org.freedesktop.appearance", "reduced-motion") {
            return Err(zbus::fdo::Error::Failed(format!(
                "no setting {namespace} {key}"
            )));
        }
        self.read.store(true, Ordering::Release);
        Ok(OwnedValue::try_from(Value::from(self.reduced_motion)).unwrap())
    }
}

/// The colours the screen showed at the gap and inside the resting rect
/// as the card entered, a run of one colour kept once, and the colour at
/// the resting rect once the card rests.
struct Entrance {
    gap: Vec<u32>,
    inside: Vec<u32>,
    rest: u32,
}

/// Capture the screen on a private Xvfb whose portal answers
/// `reduced-motion` with `reduced_motion`, and sample the toast's
/// entrance.
fn entrance(case: &str, reduced_motion: u32) -> Option<Entrance> {
    if !enabled() {
        return None;
    }
    let (_xvfb, display) = xvfb_sized(case, "1280x1024x24")?;
    let dir = tempfile::tempdir().unwrap();
    let (_bus, address) = session_bus(case, dir.path())?;
    let read = Arc::new(AtomicBool::new(false));
    let _portal = zbus::block_on(
        zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at(
                "/org/freedesktop/portal/desktop",
                Settings {
                    reduced_motion,
                    read: read.clone(),
                },
            )
            .unwrap()
            .build(),
    )
    .unwrap();

    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    paint_root(&conn, root, DESKTOP);
    let selection = conn
        .intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let _compositor = Compositor::start(&conn, root, selection);
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display)
            .env_remove("WAYLAND_DISPLAY")
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .env("IRIS_SLOWMO", SLOWMO);
    });
    daemon.until("iris to read reduced-motion from the portal", || {
        read.load(Ordering::Acquire).then_some(())
    });
    // The answer reaches the app's main thread after the read returns.
    daemon.settle();

    move_to(&conn, root, (10, 10));
    daemon.send(&["--capture-fullscreen"]);
    let window = daemon.until("the toast to open", || {
        windows_of(&conn, root, TOAST).first().copied()
    });
    let (x, y, w, h) = rect(&conn, root, window);
    let card = (x + BLEED, y + BLEED);
    let mid = card.1 + CARD.1 / 2;
    assert_eq!(
        (x + w as i16, y + h as i16),
        (1280, 1024),
        "the toast is not in the bottom-right corner"
    );
    let gap = (1280 - MARGIN / 2, mid);
    let inside = (card.0 + 20, mid);

    let start = Instant::now();
    let (mut gaps, mut insides) = (Vec::new(), Vec::new());
    while start.elapsed() < ENTRANCE {
        for (spot, seen) in [(gap, &mut gaps), (inside, &mut insides)] {
            let colour = pixel(&conn, root, spot);
            if seen.last() != Some(&colour) {
                seen.push(colour);
            }
        }
        if insides.last() == Some(&DESKTOP) && start.elapsed() > Duration::from_secs(3) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    daemon.settle();
    Some(Entrance {
        gap: gaps,
        inside: insides,
        rest: pixel(&conn, root, inside),
    })
}

/// Whether `colour` is the capture, the card at full opacity.
fn card(colour: u32) -> bool {
    let channels = |c: u32| [(c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff];
    channels(colour)
        .iter()
        .zip(channels(DESKTOP))
        .all(|(&a, b)| a.abs_diff(b) <= 0x10)
}

#[test]
fn under_reduced_motion_the_toast_fades_in_where_it_rests() {
    let Some(seen) = entrance("reduced motion: toast", 1) else {
        return;
    };
    assert!(card(seen.rest), "the card never rested: {:06x}", seen.rest);
    assert!(
        !seen.gap.iter().any(|&c| card(c)),
        "the card slid in past its resting rect: the gap showed {:06x?}",
        seen.gap
    );
    let partial: Vec<u32> = seen
        .inside
        .iter()
        .copied()
        .filter(|&c| c != 0 && !card(c))
        .collect();
    assert!(
        partial.len() >= 4,
        "the card did not fade in on its resting rect: {:06x?}",
        seen.inside
    );
}

#[test]
fn with_motion_the_toast_slides_in_from_past_the_screen_edge() {
    let Some(seen) = entrance("motion: toast", 0) else {
        return;
    };
    assert!(card(seen.rest), "the card never rested: {:06x}", seen.rest);
    assert!(
        seen.gap.iter().any(|&c| card(c)),
        "the card did not cross the gap to the screen edge: {:06x?}",
        seen.gap
    );
}
