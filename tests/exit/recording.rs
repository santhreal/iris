//! A daemon on a private Xvfb, recording a window there.

use std::path::PathBuf;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use crate::daemon::{enabled, xvfb, Daemon, Server};
use crate::x11::click;

/// The recorded window on the 640x480 Xvfb screen: x, y, width, height.
const WINDOW: (i16, i16, u16, u16) = (40, 40, 480, 360);

/// A recording in progress. Fields drop in order: the daemon, the
/// server, the connection, then the directory.
pub struct Recording {
    pub daemon: Daemon,
    pub xvfb: Server,
    /// Holds the recorded window: closing the connection destroys it.
    _x: RustConnection,
    dir: tempfile::TempDir,
}

impl Recording {
    /// Map a window on a new Xvfb, start a daemon there, and record the
    /// window through `--record-window` and a click on it. `None`,
    /// printed, when a tool the case needs is missing.
    pub fn start(case: &str) -> Option<Recording> {
        if !enabled() {
            return None;
        }
        if !on_path("ffmpeg") {
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
        x.flush().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        daemon.send(&["--record-window"]);
        // The pick takes the window under a click once it grabs the
        // pointer; a click before the grab reaches the window, which
        // selects no input. Click the window's center, away from the
        // chip at its top-right, until a segment starts.
        let center = (left + width as i16 / 2, top + height as i16 / 2);
        daemon.until("the recording to start", || {
            click(&x, root, center);
            std::thread::sleep(Duration::from_millis(100));
            daemon.log().contains("iris: record: segment").then_some(())
        });
        // Repaint the window so the segment holds frames.
        for pixel in [0x00cc_3366, 0x0066_cc33, 0x0033_66cc] {
            let repaint = ChangeWindowAttributesAux::new().background_pixel(pixel);
            x.change_window_attributes(window, &repaint).unwrap();
            x.clear_area(false, window, 0, 0, 0, 0).unwrap();
            x.flush().unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        Some(Recording {
            daemon,
            xvfb,
            _x: x,
            dir,
        })
    }

    /// Assert the recording is on disk: the recordings directory holds
    /// one file, an MP4 with the index a finished file has.
    pub fn saved(&self) {
        let vids = self.dir.path().join("vids");
        let files: Vec<PathBuf> = std::fs::read_dir(&vids)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let log = self.daemon.log();
        let [mp4] = files.as_slice() else {
            panic!(
                "{} holds {files:?}, not one recording:\n{log}",
                vids.display()
            );
        };
        assert_eq!(
            mp4.extension().and_then(|e| e.to_str()),
            Some("mp4"),
            "{files:?}:\n{log}"
        );
        let boxes = top_level_boxes(&std::fs::read(mp4).unwrap());
        assert!(
            ["ftyp", "mdat", "moov"]
                .iter()
                .all(|want| boxes.iter().any(|b| b == want)),
            "{} has the top-level boxes {boxes:?}, not ftyp, mdat, and moov: it was cut off\n{log}",
            mp4.display()
        );
    }
}

/// The types of an MP4 file's top-level boxes, in order, up to the first
/// box that runs past the end of the file.
fn top_level_boxes(mut rest: &[u8]) -> Vec<String> {
    let mut types = Vec::new();
    while rest.len() >= 8 {
        types.push(String::from_utf8_lossy(&rest[4..8]).into_owned());
        let size = match u32::from_be_bytes(rest[..4].try_into().unwrap()) {
            // The box runs to the end of the file.
            0 => rest.len() as u64,
            // A 64-bit size follows the type.
            1 if rest.len() >= 16 => u64::from_be_bytes(rest[8..16].try_into().unwrap()),
            n => u64::from(n),
        };
        if size < 8 || size > rest.len() as u64 {
            break;
        }
        rest = &rest[size as usize..];
    }
    types
}

/// Whether `tool` is a file in a directory on PATH.
fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}
