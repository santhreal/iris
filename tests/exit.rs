//! A daemon exits when its display server goes away, and every exit
//! saves the recording first and then ends the process.
//!
//! WHY: the class closed here is "the daemon ends at the wrong time": it
//! outlives its display server, it exits before a recording is on disk,
//! or its exit waits on the display. When its X server died, the
//! daemon's X11 event source read a connection error on every poll,
//! logged it, and was ready again at once: the daemon spun a core, and
//! held its GPU device, until something killed it. A daemon that exits
//! must not lose a recording on the way out: frames go to Matroska
//! segments that are joined into the output file last. An exit on the
//! display's loss joined nothing, and a `--quit` behind a stop left the
//! stop's flush to a thread the exit never waited for. After the save,
//! the exit closed the windows and dropped the GPU device: a window's
//! checked UnmapWindow waits on the X server, and with the server killed
//! mid-present, lavapipe's swapchain teardown held the process about 5 s.
//!
//! The exit cases start the daemon on a display server of its own, kill
//! the server, and bound the time the daemon takes to exit, for the two
//! session kinds `iris_lib::session` distinguishes: X11 and Wayland. A
//! third stops the X server, which then answers nothing, and bounds the
//! exit on `--quit`: an exit that waits on the display never ends. The
//! save cases record a window on Xvfb and end the daemon three ways:
//! `--quit` while it records, `--quit` while a stopped recording still
//! flushes, and the X server's death while it records. Each bounds the
//! exit and asserts one finished MP4.
//!
//! The cases run only with `IRIS_X11_TEST_DISPLAY` set, the switch for a
//! host that runs the daemon's windows. The X11 cases need `Xvfb`, the
//! Wayland case `sway`, and the save cases `ffmpeg`; a case whose tool is
//! missing prints that it did not run. Not covered: a display lost
//! during a flush, a Wayland recording, whose portal needs a session
//! bus, Windows, and macOS. A server that stops answering without
//! closing its socket ends the daemon only through `--quit`: nothing
//! else reaches the daemon.

#![cfg(target_os = "linux")]

#[path = "support/daemon.rs"]
mod daemon;
#[path = "exit/recording.rs"]
mod recording;

use std::process::{Command, Stdio};
use std::time::Duration;

use daemon::{enabled, runtime_dir, until, xvfb, Daemon, Server};
use recording::Recording;

/// How long the daemon has to exit once its display server is gone.
const EXIT_BOUND: Duration = Duration::from_secs(5);

/// How long a daemon saving a recording has to exit: the encoder kills a
/// wedged ffmpeg 10 s into a flush, and the join follows.
const SAVE_BOUND: Duration = Duration::from_secs(15);

#[test]
fn a_daemon_exits_when_its_x_server_dies() {
    if !enabled() {
        return;
    }
    let Some((mut xvfb, display)) = xvfb("a_daemon_exits_when_its_x_server_dies") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", display).env_remove("WAYLAND_DISPLAY");
    });
    xvfb.kill();
    daemon.exits(EXIT_BOUND, "its X server died");
}

#[test]
fn a_quit_ends_a_daemon_whose_x_server_stopped_answering() {
    if !enabled() {
        return;
    }
    let case = "a_quit_ends_a_daemon_whose_x_server_stopped_answering";
    let Some((xvfb, display)) = xvfb(case) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", display).env_remove("WAYLAND_DISPLAY");
    });
    // The daemon holds a window from its start: the parked overlay.
    daemon.settle();
    xvfb.stop();
    daemon.send(&["--quit"]);
    daemon.exits(EXIT_BOUND, "--quit, its X server stopped");
}

#[test]
fn a_daemon_exits_when_its_wayland_compositor_dies() {
    if !enabled() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let run = runtime_dir(dir.path());
    let config = dir.path().join("sway.conf");
    std::fs::write(&config, "xwayland disable\n").unwrap();
    // The headless backend with pixman: no GPU, no input devices, no
    // output scanned out. sway refuses to start while the NVIDIA module is
    // loaded unless told otherwise, even headless.
    let Some(mut sway) = Server::spawn(
        Command::new("sway")
            .arg("--unsupported-gpu")
            .arg("-c")
            .arg(config)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env("XDG_RUNTIME_DIR", &run)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_RENDERER", "pixman")
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    ) else {
        eprintln!("a_daemon_exits_when_its_wayland_compositor_dies did not run: no sway on PATH");
        return;
    };
    let socket = until("sway to listen", || {
        std::fs::read_dir(&run)
            .ok()?
            .filter_map(|entry| Some(entry.ok()?.file_name()))
            .find(|name| {
                name.to_str()
                    .is_some_and(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))
            })
    });

    let mut daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("WAYLAND_DISPLAY", socket).env_remove("DISPLAY");
    });
    sway.kill();
    daemon.exits(EXIT_BOUND, "its Wayland compositor died");
}

#[test]
fn a_quit_saves_the_recording_in_progress() {
    let Some(mut rec) = Recording::start("a_quit_saves_the_recording_in_progress") else {
        return;
    };
    rec.daemon.send(&["--quit"]);
    rec.daemon.exits(SAVE_BOUND, "--quit");
    rec.saved();
}

#[test]
fn a_quit_saves_a_stopped_recording_still_flushing() {
    let Some(mut rec) = Recording::start("a_quit_saves_a_stopped_recording_still_flushing") else {
        return;
    };
    // One command line: the stop hands the flush to a thread, and the
    // quit runs before that thread ends.
    rec.daemon.send(&["--record-window", "--quit"]);
    rec.daemon.exits(SAVE_BOUND, "--record-window --quit");
    rec.saved();
}

#[test]
fn an_x_server_death_saves_the_recording_in_progress() {
    let Some(mut rec) = Recording::start("an_x_server_death_saves_the_recording_in_progress")
    else {
        return;
    };
    rec.xvfb.kill();
    rec.daemon.exits(SAVE_BOUND, "its X server died");
    rec.saved();
}
