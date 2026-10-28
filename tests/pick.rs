//! The X11 window pick of `--record-window`, driven on a private Xvfb.
//!
//! WHY: the class closed here is "the pick waits on a click with nothing
//! on screen stating it". The pick grabs the pointer with a crosshair and
//! nothing else, so a pick started from a hotkey or Home looked like a
//! hung cursor. A guide card now states the click and the cancel key for
//! the length of the pick, and the pick ignores a click on any
//! override-redirect window, the card included: before that, a click on
//! the card would have recorded the card. The cases click the card and
//! read the log for a segment that must not start, click the window and
//! read the card gone, and cancel a pick with Escape. The first case
//! also closes "a window iris maps takes the pick's click": the overlay
//! warmup mapped the capture shell over the whole screen while a pick
//! started at daemon start waited, and a click in that moment recorded
//! the invisible shell. The case watches every map on the root and
//! fails on a capture overlay mapped during the pick.
//!
//! Not covered: Wayland, where the portal shows its own picker; Windows
//! and macOS. The cases run only with `IRIS_X11_TEST_DISPLAY` set and
//! need Xvfb and ffmpeg; without them they print that they did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The chip and exit tests use the rest of the harness.
#[allow(dead_code)]
#[path = "support/recording.rs"]
mod recording;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use daemon::{enabled, xvfb, Daemon, Server};
use x11::{all_windows_of, click, tap, windows_of};

/// The guide card's window title.
const NOTICE: &str = "Notice - iris";
/// The capture overlay's window title.
const OVERLAY: &str = "Capture - iris";
/// The window to record on the 640x480 screen: x, y, width, height.
const WINDOW: (i16, i16, u16, u16) = (40, 40, 480, 360);
/// XK_Escape.
const ESCAPE: u32 = 0xff1b;
/// The log line of a recording's first segment.
const SEGMENT: &str = "iris: record: segment";

/// A daemon on a private Xvfb with one window mapped and a pick started,
/// its guide card open. Fields drop in order: the daemon, the server.
struct Pick {
    daemon: Daemon,
    _xvfb: Server,
    x: RustConnection,
    root: u32,
    guide: u32,
    _dir: tempfile::TempDir,
}

impl Pick {
    fn start(case: &str) -> Option<Pick> {
        if !enabled() {
            return None;
        }
        if !recording::on_path("ffmpeg") {
            eprintln!("{case} did not run: no ffmpeg on PATH");
            return None;
        }
        let (xvfb, display) = xvfb(case)?;
        let (x, screen) = x11rb::connect(Some(display.as_str())).unwrap();
        let root = x.setup().roots[screen].root;
        let window = x.generate_id().unwrap();
        let (left, top, width, height) = WINDOW;
        x.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            left,
            top,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().background_pixel(0x0033_66cc),
        )
        .unwrap();
        x.map_window(window).unwrap();
        // Every top-level map from here on queues a MapNotify.
        x.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
        )
        .unwrap();
        x.flush().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        daemon.send(&["--record-window"]);
        let guide = daemon.until("the pick's guide card to open", || {
            windows_of(&x, root, NOTICE).first().copied()
        });
        Some(Pick {
            daemon,
            _xvfb: xvfb,
            x,
            root,
            guide,
            _dir: dir,
        })
    }

    /// The guide window's centre, in root pixels.
    fn guide_centre(&self) -> (i16, i16) {
        let g = self.x.get_geometry(self.guide).unwrap().reply().unwrap();
        (g.x + g.width as i16 / 2, g.y + g.height as i16 / 2)
    }

    fn guide_open(&self) -> bool {
        windows_of(&self.x, self.root, NOTICE).contains(&self.guide)
    }

    fn recording(&self) -> bool {
        self.daemon.log().contains(SEGMENT)
    }

    /// Whether a capture overlay window mapped since the pick started,
    /// or since the last call.
    fn overlay_mapped(&self) -> bool {
        let overlays = all_windows_of(&self.x, self.root, OVERLAY);
        let mut mapped = false;
        while let Some(event) = self.x.poll_for_event().unwrap() {
            if let Event::MapNotify(ev) = event {
                mapped |= overlays.contains(&ev.window);
            }
        }
        mapped
    }
}

#[test]
fn a_click_on_the_guide_picks_nothing_and_a_click_on_a_window_ends_it() {
    let Some(pick) =
        Pick::start("a_click_on_the_guide_picks_nothing_and_a_click_on_a_window_ends_it")
    else {
        return;
    };
    // The pick grabs the pointer a moment after the card opens; clicks
    // over a second and a half land both before and under the grab.
    let centre = pick.guide_centre();
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        click(&pick.x, pick.root, centre);
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        !pick.recording(),
        "a click at {centre:?} on the guide card started a recording; daemon log:\n{}",
        pick.daemon.log()
    );
    assert!(
        pick.guide_open(),
        "the guide card left before the pick ended"
    );
    assert!(
        !pick.overlay_mapped(),
        "the capture overlay mapped over the screen while the pick waited on a click"
    );

    let (left, top, width, height) = WINDOW;
    let window = (left + width as i16 / 2, top + height as i16 / 2);
    pick.daemon
        .until("a click on the window to start the recording", || {
            click(&pick.x, pick.root, window);
            std::thread::sleep(Duration::from_millis(100));
            pick.recording().then_some(())
        });
    pick.daemon
        .until("the guide card to leave once the pick lands", || {
            (!pick.guide_open()).then_some(())
        });
}

#[test]
fn escape_cancels_the_pick_and_its_guide() {
    let Some(pick) = Pick::start("escape_cancels_the_pick_and_its_guide") else {
        return;
    };
    pick.daemon
        .until("Escape to cancel the pick and close its guide", || {
            tap(&pick.x, pick.root, ESCAPE);
            std::thread::sleep(Duration::from_millis(100));
            (!pick.guide_open()).then_some(())
        });
    assert!(
        !pick.recording(),
        "a cancelled pick started a recording; daemon log:\n{}",
        pick.daemon.log()
    );
}
