//! Drag-out into the platform file manager: a press on the toast's card,
//! a move onto an Explorer (Windows) or Finder (macOS) folder window, and
//! a release there copy the capture into that folder.
//!
//! WHY: the class closed here is "a drag out of iris that the file
//! manager never receives": a drag that never starts from the press, a
//! drag loop that loses the button, a payload the folder refuses, or an
//! operation the folder rejects. The tests beside each implementation
//! cover the payload with no pointer; this one moves the system pointer
//! with synthetic input, so the drag runs as a press and a move by hand
//! run it.
//!
//! Runs only with `IRIS_DESKTOP_TEST` set: the test takes the pointer of
//! the logged-in desktop and opens a file manager window, which a CI
//! runner's desktop allows and a desktop in use does not. Not covered:
//! Linux, where CI has no file manager to drop on; a drop onto a browser
//! or a chat app; the drag image; a library card drag, which starts the
//! same drag.
#![cfg(any(windows, target_os = "macos"))]

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// The toast window's title.
const TOAST: &str = "Screenshot - iris";

#[test]
fn a_toast_drag_onto_a_folder_window_copies_the_capture_there() {
    if std::env::var_os("IRIS_DESKTOP_TEST").is_none() {
        return;
    }
    desk::init();
    let home = Home::new();
    let shot = home.path().join("shots").join("drag-out.png");
    image::RgbaImage::from_fn(320, 200, |x, y| image::Rgba([x as u8, y as u8, 160, 255]))
        .save(&shot)
        .unwrap();
    let dest = home.path().join("iris-drop");
    std::fs::create_dir(&dest).unwrap();

    home.iris(&[OsStr::new("--toast"), shot.as_os_str()]);
    home.until("the toast to open", desk::toast);
    // The card slides in inside a window that stays put.
    std::thread::sleep(Duration::from_millis(1000));
    let toast = desk::toast().expect("the toast closed before the drag");
    let folder = desk::open_folder(&dest);
    println!("toast {toast:?}, folder window {:?}", folder.rect);

    // The window's center lies on the card for any toast size. The
    // folder window sits left of the toast: the first move is away from
    // the toast's screen edge, so it starts a drag, not a dismiss swipe.
    drag(toast.at(0.5, 0.5), folder.rect.at(0.65, 0.6));

    let copied = dest.join("drag-out.png");
    let len = std::fs::metadata(&shot).unwrap().len();
    home.until(
        "the file manager to copy the capture into the folder",
        || {
            let meta = std::fs::metadata(&copied).ok()?;
            (meta.len() == len).then_some(())
        },
    );
    assert_eq!(
        std::fs::read(&copied).unwrap(),
        std::fs::read(&shot).unwrap(),
        "the folder's copy differs from the capture"
    );
    assert!(
        shot.exists(),
        "the drag moved the capture instead of copying it"
    );
}

/// Press at `from`, move to `to` over half a second, dwell there, and
/// release. The button is released even when a move fails.
fn drag(from: (f64, f64), to: (f64, f64)) {
    desk::hover(from);
    std::thread::sleep(Duration::from_millis(200));
    desk::press(from);
    std::thread::sleep(Duration::from_millis(100));
    const STEPS: u32 = 40;
    for i in 1..=STEPS {
        let t = f64::from(i) / f64::from(STEPS);
        desk::drag_to((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t));
        std::thread::sleep(Duration::from_millis(16));
    }
    // A drop target settles its effect over a few moves above it.
    for i in 0..12 {
        desk::drag_to((to.0 + f64::from(i % 2) * 2.0, to.1));
        std::thread::sleep(Duration::from_millis(50));
    }
    desk::release(to);
}

/// A window rect in the coordinates the platform's input events use:
/// physical pixels on Windows, points on macOS, origin at the top left
/// of the primary display.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    /// The point at fractions `fx`, `fy` of the width and height.
    fn at(&self, fx: f64, fy: f64) -> (f64, f64) {
        (self.x + self.w * fx, self.y + self.h * fy)
    }
}

/// Every iris location in a fresh directory, with the toast held open
/// for the whole test. The daemon quits on drop.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Home {
        let dir = tempfile::Builder::new()
            .prefix("iris-dragout-")
            .tempdir()
            .unwrap();
        for sub in ["config", "shots", "vids"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        let quoted = |p: &Path| toml::Value::from(p.to_str().unwrap()).to_string();
        std::fs::write(
            dir.path().join("config").join("config.toml"),
            format!(
                "screenshots_dir = {}\nrecordings_dir = {}\n\
                 toast_duration_ms = 600000\ntoast_show_actions = false\n",
                quoted(&dir.path().join("shots")),
                quoted(&dir.path().join("vids")),
            ),
        )
        .unwrap();
        Home { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    /// `iris <args>`; a client with no daemon starts one.
    fn iris(&self, args: &[&OsStr]) {
        let out = Command::new(env!("CARGO_BIN_EXE_iris"))
            .args(args)
            .env("IRIS_HOME", self.path())
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "iris {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Poll `f` until it returns a value, for at most 15 s.
    fn until<T>(&self, what: &str, mut f: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(value) = f() {
                return value;
            }
            if Instant::now() > deadline {
                let log = std::fs::read_to_string(self.path().join("state").join("iris.log"))
                    .unwrap_or_default();
                panic!(
                    "timed out waiting for {what}\nwindows on screen:\n{}\ndaemon log:\n{log}",
                    desk::describe()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_iris"))
            .arg("--quit")
            .env("IRIS_HOME", self.path())
            .output();
    }
}

#[cfg(windows)]
#[path = "dragout/windows.rs"]
mod desk;

#[cfg(target_os = "macos")]
#[path = "dragout/macos.rs"]
mod desk;
