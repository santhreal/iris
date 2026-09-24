//! A copy on a Wayland session goes through the compositor's data-control
//! protocol: another client pastes it, and the copy connects to no X
//! server.
//!
//! WHY: the class closed here is "a Wayland copy needs XWayland". The
//! process's clipboard set text and images through the X server in
//! `DISPLAY`, so the first copy on a Wayland session started an on-demand
//! XWayland, about 100 MB resident, and a file copy failed there outright.
//! The case starts a headless sway with no XWayland and `DISPLAY` naming a
//! socket that counts connections, copies text, an image, and two files,
//! pastes each back with `wl-paste`, and requires no connection to
//! `DISPLAY`. src/clipboard.rs covers X11. Not covered: a compositor with
//! no data-control protocol, where a copy goes through XWayland, and
//! `ext-data-control-v1`, which sway 1.9 does not offer. The case runs
//! only with `IRIS_X11_TEST_DISPLAY` set, the switch for a host that runs
//! the daemon's windows, and needs `sway` and `wl-paste`; without them it
//! prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;

use std::ffi::OsStr;
use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use daemon::{counted_x_display, enabled, runtime_dir, sway, until};
use iris_lib::{clipboard, dragcopy};

#[test]
fn a_copy_on_wayland_pastes_back_and_connects_to_no_x_server() {
    if !enabled() {
        return;
    }
    let case = "a_copy_on_wayland_pastes_back_and_connects_to_no_x_server";
    if Command::new("wl-paste").arg("--version").output().is_err() {
        eprintln!("{case} did not run: no wl-paste on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let run = runtime_dir(dir.path());
    let Some((_sway, socket)) = sway(case, &run) else {
        return;
    };
    let (display, connections) = counted_x_display();
    // The copies run in this process. This is the file's one case, so no
    // other case reads the environment while it changes.
    std::env::set_var("XDG_RUNTIME_DIR", &run);
    std::env::set_var("WAYLAND_DISPLAY", &socket);
    std::env::set_var("DISPLAY", &display);
    let paste = |mime| wl_paste(&run, &socket, mime);

    // Each copy offers no type the one before it offered, so a paste
    // that runs before the compositor has the new selection reads
    // nothing and polls again.
    clipboard::set_text("#a1b2c3").unwrap();
    let text = until("wl-paste to read the text", || paste("text/plain"));
    assert_eq!(String::from_utf8_lossy(&text), "#a1b2c3");

    let img = image::RgbaImage::from_fn(3, 2, |x, y| {
        image::Rgba([x as u8 * 80, y as u8 * 90, 7, 255])
    });
    clipboard::set_image(&img).unwrap();
    let png = until("wl-paste to read the image", || paste("image/png"));
    assert_eq!(image::load_from_memory(&png).unwrap().to_rgba8(), img);

    // A space and a second file: the list names each file, in order.
    let files = [dir.path().join("a b.png"), dir.path().join("c.png")];
    for f in &files {
        std::fs::write(f, b"x").unwrap();
    }
    dragcopy::copy_file_paths(&files).unwrap();
    let list = until("wl-paste to read the files", || paste("text/uri-list"));
    let list = String::from_utf8(list).unwrap();
    let pasted: Vec<_> = list
        .lines()
        .map(|uri| uri.strip_prefix("file://").map(dragcopy::uri_decode_path))
        .collect();
    let want: Vec<_> = files.iter().map(|f| f.canonicalize().ok()).collect();
    assert_eq!(pasted, want, "uri-list {list:?}");

    assert_eq!(
        connections.load(Ordering::SeqCst),
        0,
        "a copy on a Wayland session connected to DISPLAY={display}"
    );
}

/// What `wl-paste` reads from the clipboard as `mime`, or None while the
/// clipboard offers no `mime`. Fails the case when `wl-paste` runs for
/// over 5 s, as it does on a source that never writes.
fn wl_paste(run: &Path, socket: &OsStr, mime: &str) -> Option<Vec<u8>> {
    let mut child = Command::new("wl-paste")
        .args(["--no-newline", "--type", mime])
        .env("XDG_RUNTIME_DIR", run)
        .env("WAYLAND_DISPLAY", socket)
        .env_remove("DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("start wl-paste: {e}"));
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("wl-paste --type {mime} ran for over 5 s");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let bytes = reader.join().unwrap().unwrap();
    status.success().then_some(bytes)
}
