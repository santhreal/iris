//! A daemon grabs its hotkeys and binds its single-instance socket before
//! it opens a window, on X11 it sets up its X connection before its GPU
//! driver loads, and on a Wayland session it leaves the X display alone.
//!
//! WHY: the classes closed here are "the socket waits on the display",
//! "an input source starts after the warmup", "the daemon's X connection
//! races the X server's reset", and "a Wayland daemon uses XWayland". A
//! client that finds no daemon starts one, forwards its command once the
//! socket accepts, and fails after 8 s; every other client that finds no
//! socket meanwhile waits for the same bind. The daemon bound its socket
//! after the overlay warmup, whose renderer init on lavapipe took 0.2 s
//! with a warm shader cache and seconds with a cold one. It grabbed its
//! hotkeys after the warmup too, so a hotkey pressed during it reached no
//! client and was lost. It created its GPU context on a thread that
//! started before its X connection was set up, and Mesa's device
//! selection layer, loaded on that thread, sets up an X connection and
//! closes it as it enumerates devices. An X server started without
//! -noreset resets when its last client disconnects, and the reset closes
//! every connection still in setup: on a server with no other client, as
//! a fresh Xvfb is, the daemon's connection failed with "Unknown
//! connection error" in 19 of 80 debug-build starts on a loaded host. On
//! a Wayland session it grabbed the hotkeys on XWayland, which delivers a
//! key only while an X11 window has the focus. Its Vulkan instance also
//! enabled the XCB surface extension, with which the device selection
//! layer connects to `DISPLAY` while it enumerates devices whenever it
//! finds no device through Wayland, and the NVIDIA driver connects to
//! `DISPLAY` whenever the loader loads it. Any of these connections starts
//! an on-demand XWayland, about 100 MB resident, and the ones before the
//! bind held it until XWayland answered.
//!
//! The first case grabs the daemon's X server once the daemon prints its
//! start line. From then on every request the daemon sends the server
//! waits for the grab's release, and the socket must bind while the grab
//! holds. The second presses the capture hotkey the moment the socket
//! accepts, while the warmup runs, and waits for the overlay to take the
//! focus. A grab made on another thread after the bind fails it only when
//! the press lands first. The third starts a daemon on a fresh Xvfb
//! through a display that forwards each connection to the server. The
//! display records when it opens a connection, when the server accepts
//! a connection's setup, and when a client closes a connection, and it
//! passes the server's answer to the first setup on 2 s late, as a
//! loaded server answers. The daemon's loader also loads
//! tests/fixtures/display_driver.rs, which sets up a connection to
//! `DISPLAY` and closes it when it is loaded, as the device selection
//! layer does, and sets up another at each unload. Until the socket
//! binds, no connection but the first may open, and no set-up
//! connection may close, while no other connection is set up and open.
//! A GPU context thread that starts before the X connection is set up
//! loads the driver within those 2 s. The fourth starts a daemon on a
//! headless sway with `DISPLAY` naming a socket that counts
//! connections, and requires none once the daemon is idle; sway renders
//! with pixman, so the device selection layer finds no device through
//! Wayland. Its loader also loads the test driver, which records the
//! `DISPLAY` it saw at each load and at each device enumeration. The case
//! requires none at a load and the daemon's own at every enumeration: the
//! variable is back before the GPUs are enumerated, for the processes the
//! daemon starts and for its X11 clipboard. Not covered: startup work
//! before the start line (the X connection, the hotkey grabs, the GPU
//! device, the fonts), work that delays the bind without a request to the
//! server, the tray, whose menu a private Xvfb has no panel to show, an X
//! connection a Wayland daemon opens after its start, for a command, a
//! driver that connects to `DISPLAY` after the instance exists, an X
//! connection that closes after the bind, and a connection that passes a
//! file descriptor through the forwarding display, which forwards none.
//! The cases run only with `IRIS_X11_TEST_DISPLAY` set and need `Xvfb` or
//! `sway`; without them a case prints that it did not run.

#![cfg(target_os = "linux")]

// The exit cases use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
#[path = "support/driver.rs"]
mod driver;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;
#[path = "startup/xproxy.rs"]
mod xproxy;

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::time::Duration;

use daemon::{counted_x_display, enabled, runtime_dir, sway, xvfb, Daemon};
use driver::Driver;
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use xproxy::{Event, Proxy};

/// The daemon's first line, printed once its platform has connected to
/// the display and it has grabbed its hotkeys, before the rest of its
/// start runs.
const START: &str = "iris: daemon start";

/// The keysym of `Print`, the default `capture_hotkey`.
const PRINT: u32 = 0xff61;

/// The title of the capture overlay's window.
const OVERLAY: &str = "Capture - iris";

/// How late the forwarding display passes on the server's answer to the
/// daemon's first X connection's setup: longer than a daemon takes from
/// its start to load its GPU driver or to connect to the display.
const SETUP_HOLD: Duration = Duration::from_secs(2);

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
fn a_daemon_sets_up_its_x_connection_before_its_gpu_driver_connects() {
    if !enabled() {
        return;
    }
    let case = "a_daemon_sets_up_its_x_connection_before_its_gpu_driver_connects";
    let Some((_xvfb, server)) = xvfb(case) else {
        return;
    };
    let proxy = Proxy::new(&server, SETUP_HOLD);
    let dir = tempfile::tempdir().unwrap();
    let driver = Driver::build(dir.path());
    let mut daemon = Daemon::spawn(dir.path(), |cmd| {
        cmd.env("DISPLAY", &proxy.display)
            .env_remove("WAYLAND_DISPLAY");
        driver.add_to(cmd);
    });
    daemon.bound();
    let events = proxy.events();
    let log = std::fs::read_to_string(&driver.log).unwrap_or_default();
    assert!(
        log.lines()
            .any(|line| line == format!("load {}", proxy.display))
            && events.iter().any(|event| matches!(event, Event::Closed(_))),
        "the test driver closed no X connection before the bind; the driver's log:\n{log}\nthe \
         connections: {events:?}"
    );
    // The connections set up and not closed.
    let mut open = BTreeSet::new();
    for &event in &events {
        let alone = match event {
            Event::Opened(id) => id > 0 && open.is_empty(),
            Event::SetUp(id) => {
                open.insert(id);
                false
            }
            Event::Closed(id) => open.remove(&id) && open.is_empty(),
        };
        assert!(
            !alone,
            "the daemon's X connections reached {event:?} with no other connection set up and \
             open; the connections: {events:?}; daemon log:\n{}",
            daemon.log()
        );
    }
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
