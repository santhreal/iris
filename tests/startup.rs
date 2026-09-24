//! A daemon grabs its hotkeys and binds its single-instance socket before
//! it opens a window, and on a Wayland session it leaves the X display
//! alone.
//!
//! WHY: the classes closed here are "the socket waits on the display",
//! "an input source starts after the warmup", and "a Wayland daemon uses
//! XWayland". A client that finds no daemon starts one, forwards its
//! command once the socket accepts, and fails after 8 s; a second `iris`
//! that finds no socket starts a daemon of its own. The daemon bound its
//! socket after the overlay warmup, whose renderer init on lavapipe took
//! 0.2 s with a warm shader cache and seconds with a cold one. It grabbed
//! its hotkeys after the warmup too, so a hotkey pressed during it
//! reached no client and was lost. On a Wayland session it grabbed the
//! hotkeys on XWayland, which delivers a key only while an X11 window has
//! the focus. Its Vulkan instance also enabled the XCB surface extension,
//! with which Mesa's device selection layer connects to `DISPLAY` while it
//! enumerates devices whenever it finds no device through Wayland, and
//! the NVIDIA driver connects to `DISPLAY` whenever the loader loads it.
//! Any of these connections starts an on-demand XWayland, about 100 MB
//! resident, and the ones before the bind held it until XWayland answered.
//!
//! The first case grabs the daemon's X server once the daemon prints its
//! start line. From then on every request the daemon sends the server
//! waits for the grab's release, and the socket must bind while the grab
//! holds. The second presses the capture hotkey the moment the socket
//! accepts, while the warmup runs, and waits for the overlay to take the
//! focus. A grab made on another thread after the bind fails it only when
//! the press lands first. The third starts a daemon on a headless sway
//! with `DISPLAY` naming a socket that counts connections, and requires
//! none once the daemon is idle; sway renders with pixman, so the device
//! selection layer finds no device through Wayland. Its loader also loads
//! tests/fixtures/display_driver.rs, which connects to `DISPLAY` when it
//! is loaded, as the NVIDIA driver does, and records the `DISPLAY` it saw
//! at each load and at each device enumeration. The case requires none at
//! a load and the daemon's own at every enumeration: the variable is back
//! before the GPUs are enumerated, for the processes the daemon starts
//! and for its X11 clipboard. Not covered: startup work before the start
//! line (the X connection, the hotkey grabs, the GPU device, the fonts),
//! work that delays the bind without a request to the server, the tray,
//! whose menu a private Xvfb has no panel to show, an X connection a
//! Wayland daemon opens after its start, for a command, a driver that
//! connects to `DISPLAY` after the instance exists, and, on a host
//! without Mesa's Vulkan drivers, the device selection layer. The cases
//! run only with `IRIS_X11_TEST_DISPLAY` set and need `Xvfb` or `sway`;
//! without them a case prints that it did not run.

#![cfg(target_os = "linux")]

// The exit cases use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;

use daemon::{counted_x_display, enabled, runtime_dir, sway, xvfb, Daemon};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::ConnectionExt as _;

/// The daemon's first line, printed once its platform has connected to
/// the display and it has grabbed its hotkeys, before the rest of its
/// start runs.
const START: &str = "iris: daemon start";

/// The keysym of `Print`, the default `capture_hotkey`.
const PRINT: u32 = 0xff61;

/// The title of the capture overlay's window.
const OVERLAY: &str = "Capture - iris";

#[test]
fn a_daemon_binds_its_socket_before_it_opens_a_window() {
    if !enabled() {
        return;
    }
    let case = "a_daemon_binds_its_socket_before_it_opens_a_window";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, _) = x11rb::connect(Some(&display)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::spawn(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    daemon.until("the daemon's start line", || {
        daemon.log().contains(START).then_some(())
    });
    conn.grab_server().unwrap();
    // The reply comes after the grab: the server holds every other
    // client from here on.
    conn.get_input_focus().unwrap().reply().unwrap();
    daemon.bound();
    conn.ungrab_server().unwrap();
    conn.flush().unwrap();
}

#[test]
fn a_hotkey_pressed_once_the_socket_accepts_opens_the_overlay() {
    if !enabled() {
        return;
    }
    let case = "a_hotkey_pressed_once_the_socket_accepts_opens_the_overlay";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    x11::tap(&conn, root, PRINT);
    // The warmup's window opens unfocused and stays so; only a capture
    // gives the overlay the focus, once its frame lands.
    daemon.until("the overlay to take the focus", || {
        let focused = x11::focus(&conn);
        x11::windows_of(&conn, root, OVERLAY)
            .contains(&focused)
            .then_some(())
    });
}

#[test]
fn a_daemon_on_a_wayland_session_leaves_its_x_display_alone() {
    if !enabled() {
        return;
    }
    let case = "a_daemon_on_a_wayland_session_leaves_its_x_display_alone";
    let dir = tempfile::tempdir().unwrap();
    let Some((_sway, socket)) = sway(case, &runtime_dir(dir.path())) else {
        return;
    };
    let (display, connections) = counted_x_display();
    let driver = Driver::build(dir.path());
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("WAYLAND_DISPLAY", &socket).env("DISPLAY", &display);
        driver.add_to(cmd);
    });
    daemon.settle();
    let log = std::fs::read_to_string(&driver.log).unwrap_or_default();
    // The DISPLAY the driver saw at each `event`.
    let seen = |event: &str| -> Vec<&str> {
        log.lines()
            .filter_map(|line| line.strip_prefix(event)?.strip_prefix(' '))
            .collect()
    };
    let loads = seen("load");
    assert!(
        !loads.is_empty(),
        "the Vulkan loader never loaded the test driver; daemon log:\n{}",
        daemon.log()
    );
    assert!(
        loads.iter().all(|&at_load| at_load == "-"),
        "the Vulkan loader loaded a driver with DISPLAY set on a Wayland session; the driver's \
         log:\n{log}"
    );
    let enumerations = seen("devices");
    assert!(
        !enumerations.is_empty()
            && enumerations
                .iter()
                .all(|&at_enumeration| at_enumeration == display),
        "the daemon enumerated GPUs without DISPLAY={display} in its environment; the driver's \
         log:\n{log}"
    );
    assert_eq!(
        connections.load(Ordering::SeqCst),
        0,
        "the daemon connected to DISPLAY={display} on a Wayland session; daemon log:\n{}",
        daemon.log()
    );
}

/// tests/fixtures/display_driver.rs built into a shared library, the
/// manifest the Vulkan loader finds it through, and the file it records
/// each load and each device enumeration in.
struct Driver {
    manifest: PathBuf,
    log: PathBuf,
}

impl Driver {
    fn build(dir: &Path) -> Driver {
        let library = dir.join("libdisplay_driver.so");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/display_driver.rs");
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let built = Command::new(&rustc)
            .args(["--edition", "2021", "--crate-type", "cdylib"])
            .args([
                "--crate-name",
                "display_driver",
                "-C",
                "strip=symbols",
                "-o",
            ])
            .arg(&library)
            .arg(&source)
            .status()
            .unwrap_or_else(|e| panic!("run {rustc:?}: {e}"));
        assert!(
            built.success(),
            "{rustc:?} could not build {}",
            source.display()
        );
        // A driver without vkEnumerateInstanceVersion is a Vulkan 1.0
        // driver; the loader warns of one whose manifest states more.
        let manifest = dir.join("display_driver.json");
        let json = serde_json::json!({
            "file_format_version": "1.0.0",
            "ICD": { "library_path": library, "api_version": "1.0.0" },
        });
        std::fs::write(&manifest, json.to_string()).unwrap();
        Driver {
            manifest,
            log: dir.join("display_driver.log"),
        }
    }

    /// Add the driver to the ones the daemon's Vulkan loader loads.
    fn add_to(&self, cmd: &mut Command) {
        // The loader ignores VK_ADD_DRIVER_FILES once VK_ICD_FILENAMES is
        // set, as the harness sets it where lavapipe is installed.
        let listed = cmd
            .get_envs()
            .find(|(key, _)| *key == "VK_ICD_FILENAMES")
            .and_then(|(_, value)| value.map(OsString::from));
        match listed {
            Some(mut list) => {
                list.push(":");
                list.push(&self.manifest);
                cmd.env("VK_ICD_FILENAMES", list);
            }
            None => {
                cmd.env("VK_ADD_DRIVER_FILES", &self.manifest);
            }
        }
        cmd.env("IRIS_TEST_DRIVER_LOG", &self.log);
    }
}
