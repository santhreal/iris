//! The pixel density a saved capture states, read from the files the
//! daemon writes on a private Xvfb.
//!
//! WHY: the class closed here is "a capture states no density, or the
//! wrong one, so a document places a HiDPI capture at twice its size on
//! screen". A full-screen capture and a region capture, at scale 1 and
//! at scale 2, each state the display's scale in their `pHYs` chunk:
//! 96 logical pixels per inch, times the scale. A third case closes "an
//! edit drops the density": the editor's save re-encodes the image, and
//! the file it writes keeps the density the original stated.
//!
//! Not covered: a window capture, which needs a window manager's active
//! window and goes through the full-screen capture's density; Wayland,
//! whose portal frames state none; Windows and macOS, where a logical
//! pixel is a DIP and a point. `src/density.rs` covers the chunk itself.
//! The cases run only with `IRIS_X11_TEST_DISPLAY` set and need Xvfb;
//! without them a case prints that it did not run.

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

use iris_lib::density::Density;
use x11rb::connection::Connection;

use daemon::{enabled, entries, xvfb_sized, Daemon};
use x11::{paint_root, pixel, rect, tap, windows_of};

const SCREEN: &str = "1280x1024x24";
const DESKTOP: u32 = 0x5b7fa6;
const OVERLAY: &str = "Capture - iris";
const EDITOR: &str = "plain.png - iris";
const RETURN: u32 = 0xff0d;
const R: u32 = 0x72;
/// The editor fixture's colour, and a density no display in these
/// cases has: 120 per inch.
const IMAGE: [u8; 3] = [60, 140, 200];
const FIXTURE: (u32, u32) = (4724, 4724);

/// The density the file at `path` states, read from disk.
fn density(path: &Path) -> Option<Density> {
    Density::read(&mut std::fs::File::open(path).ok()?)
}

/// The newest capture once the store lists more than `before` and the
/// file decodes whole.
fn landed(daemon: &Daemon, dir: &Path, before: usize) -> PathBuf {
    let shot = daemon.until("the capture to land", || {
        let listed = entries(dir).unwrap_or_default();
        (listed.len() > before).then(|| listed[0].path.clone())
    });
    daemon.until("the capture to read whole", || {
        image::open(&shot).ok().map(drop)
    });
    shot
}

#[test]
fn a_capture_states_the_scale_of_its_display() {
    let case = "a_capture_states_the_scale_of_its_display";
    if !enabled() {
        return;
    }
    for scale in [1u32, 2] {
        let Some((_xvfb, display)) = xvfb_sized(case, SCREEN) else {
            return;
        };
        let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
        let root = conn.setup().roots[screen].root;
        paint_root(&conn, root, DESKTOP);
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display)
                .env_remove("WAYLAND_DISPLAY")
                .env("GPUI_X11_SCALE_FACTOR", scale.to_string());
        });
        let want = Density::of_scale(scale as f32);

        daemon.send(&["--capture-fullscreen"]);
        let full = landed(&daemon, dir.path(), 0);
        assert_eq!(
            density(&full),
            want,
            "a full-screen capture at scale {scale}"
        );

        daemon.send(&["--capture"]);
        daemon.until("the overlay to take the focus", || {
            windows_of(&conn, root, OVERLAY)
                .contains(&x11::focus(&conn))
                .then_some(())
        });
        daemon.until("the frozen frame to land", || {
            daemon.log().contains("frame landed").then_some(())
        });
        daemon.settle();
        x11::drag(&conn, root, (200, 200), (520, 440), 20);
        daemon.settle();
        tap(&conn, root, RETURN);
        let region = landed(&daemon, dir.path(), 1);
        assert_ne!(region, full);
        assert_eq!(density(&region), want, "a region capture at scale {scale}");
    }
}

#[test]
fn an_edit_keeps_the_density_its_capture_states() {
    let case = "an_edit_keeps_the_density_its_capture_states";
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
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(600, 400, image::Rgb(IMAGE))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let mut png = png.into_inner();
    let stated = Density::per_metre(FIXTURE.0, FIXTURE.1);
    stated.insert(&mut png).unwrap();
    std::fs::write(&file, &png).unwrap();
    let written = std::fs::metadata(&file).unwrap().modified().unwrap();

    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.send(&[OsStr::new("--annotate"), file.as_os_str()]);
    let editor = daemon.until("the editor to open", || {
        windows_of(&conn, root, EDITOR).first().copied()
    });
    let (x, y, w, h) = rect(&conn, root, editor);
    let centre = (x + w as i16 / 2, y + h as i16 / 2);
    let rgb = u32::from_be_bytes([0, IMAGE[0], IMAGE[1], IMAGE[2]]);
    daemon.until("the editor to draw the image", || {
        (pixel(&conn, root, centre) == rgb).then_some(())
    });
    daemon.settle();
    // A rectangle across the middle of the image, then Done.
    tap(&conn, root, R);
    daemon.settle();
    x11::drag(
        &conn,
        root,
        (centre.0 - 80, centre.1 - 50),
        (centre.0 + 80, centre.1 + 50),
        6,
    );
    daemon.settle();
    tap(&conn, root, RETURN);
    let saved = daemon.until("the editor to save and close", || {
        if !windows_of(&conn, root, EDITOR).is_empty() {
            return None;
        }
        let modified = std::fs::metadata(&file).ok()?.modified().ok()?;
        if modified == written {
            return None;
        }
        Some(image::open(&file).ok()?.to_rgb8())
    });
    assert!(
        saved.pixels().any(|p| p.0 != IMAGE),
        "the save holds no mark: the rectangle was not drawn"
    );
    assert_eq!(density(&file), Some(stated), "the edited file's density");
}
