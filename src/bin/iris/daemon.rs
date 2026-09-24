//! Daemon mode: single instance, tray icon, global hotkeys.
//!
//! The first process binds a Unix socket and becomes the daemon: it
//! owns the tray icon (StatusNotifierItem via ksni), the X11 global
//! hotkey grabs, and every window. Later invocations forward their
//! arguments over the socket and exit, so `iris --capture` on a
//! hotkey always lands in the one persistent process.
//!
//! Wayland denies global key grabs; there the compositor-level binding
//! runs `iris --capture`, which forwards here the same way. Recording
//! sources are started through `sys::record`.

use std::path::PathBuf;
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui::*;
use iris_lib::record;

use crate::{chip, library, overlay, settings, stage};

mod capture;
mod recording;

/// The daemon's live recording, if any: one at a time. The record
/// thread owns the border strips; the chip window is a GPUI surface.
pub(crate) static RECORDING: parking_lot::Mutex<Option<record::ActiveRecording>> =
    parking_lot::Mutex::new(None);

/// The daemon's command channel, installed by `start`.
pub(crate) static COMMAND_TX: std::sync::OnceLock<UnboundedSender<Command>> =
    std::sync::OnceLock::new();

/// The command channel sender for a recording source's end notice,
/// which runs on the source thread.
pub(crate) fn command_tx() -> Option<UnboundedSender<Command>> {
    COMMAND_TX.get().cloned()
}

/// One command line, forwarded to or invoked on the daemon.
#[derive(Debug, Clone)]
pub enum Command {
    Home,
    Capture,
    CaptureFullscreen,
    CaptureWindow,
    /// Fullscreen capture after N seconds: menus and tooltips that
    /// close on any click survive the wait.
    Delayed(u64),
    Library,
    Settings,
    Annotate(PathBuf),
    Toast(PathBuf),
    RecordToggle,
    /// Open the overlay in region-pick mode for a region recording.
    RecordRegionPick,
    /// The overlay committed a rect; start recording it.
    RecordRegion {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    RecordPause,
    RecordMic,
    /// A recording source returned: the daemon collects the recording
    /// when it ended on its own, not by a stop.
    RecordingEnded,
    /// The capture store changed in this process: an open library
    /// window shows the change now instead of on its next poll.
    LibraryChanged,
    /// A thread with no window of its own failed at `what`: the daemon
    /// shows `err` in a notice.
    Failed {
        what: &'static str,
        err: String,
    },
    Quit,
}

impl Command {
    /// The notice title when this command fails. The match is
    /// exhaustive so a new command fails to compile until it has one.
    fn failure_title(&self) -> &'static str {
        match self {
            Command::Capture
            | Command::CaptureFullscreen
            | Command::CaptureWindow
            | Command::Delayed(_) => "Capture failed",
            Command::RecordToggle
            | Command::RecordRegionPick
            | Command::RecordRegion { .. }
            | Command::RecordingEnded => "Recording failed",
            Command::LibraryChanged => "Library did not refresh",
            Command::RecordPause => "Pause failed",
            Command::RecordMic => "Microphone toggle failed",
            Command::Home => "Home did not open",
            Command::Library => "Library did not open",
            Command::Settings => "Settings did not open",
            Command::Annotate(_) => "Editor did not open",
            Command::Toast(_) => "Toast did not open",
            Command::Failed { what, .. } => what,
            Command::Quit => "Quit failed",
        }
    }
}

/// Send `err` from a thread with no window to the daemon's notice.
/// Before the daemon's command loop starts it is only logged.
pub(crate) fn report_failure(what: &'static str, err: String) {
    iris_lib::ilog!("iris: {what}: {err}");
    if let Some(tx) = command_tx() {
        let _ = tx.unbounded_send(Command::Failed { what, err });
    }
}

/// True when every command acts on a running daemon's live state:
/// quit, pause, and the mic toggle. A fresh daemon holds none of that
/// state, so the client delivers these only to a daemon that is
/// already up and never spawns one for them. The match is exhaustive
/// so a new command fails to compile until it is classified.
pub fn live_daemon_only(cmds: &[Command]) -> bool {
    !cmds.is_empty()
        && cmds.iter().all(|c| match c {
            Command::Quit | Command::RecordPause | Command::RecordMic => true,
            Command::Home
            | Command::Capture
            | Command::CaptureFullscreen
            | Command::CaptureWindow
            | Command::Delayed(_)
            | Command::Library
            | Command::Settings
            | Command::Annotate(_)
            | Command::Toast(_)
            | Command::RecordToggle
            | Command::RecordRegionPick
            | Command::RecordRegion { .. }
            | Command::RecordingEnded
            | Command::LibraryChanged
            | Command::Failed { .. } => false,
        })
}

/// Run `cmd` and show why it failed in a notice: a hotkey, the tray,
/// a home tile, or a forwarded command line has no other place to show
/// it.
pub fn run(cx: &mut App, cmd: &Command) {
    if let Err(e) = dispatch(cx, cmd) {
        iris_lib::ilog!("iris: {cmd:?}: {e}");
        crate::notice::failed(cx, cmd.failure_title(), &e);
    }
}

/// Run one command against the live app. A caller with a surface of its
/// own for the error (the library's status line) takes it here; every
/// other caller goes through `run`.
pub fn dispatch(cx: &mut App, cmd: &Command) -> Result<(), String> {
    match cmd {
        Command::Home => crate::home::open(cx),
        Command::Capture => capture::capture_region(cx),
        Command::CaptureFullscreen => capture::capture_fullscreen(cx),
        Command::CaptureWindow => capture::capture_active_window(cx),
        Command::Delayed(secs) => {
            let secs = *secs;
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(secs))
                    .await;
                let _ = cx.update(|cx| run(cx, &Command::CaptureFullscreen));
            })
            .detach();
            Ok(())
        }
        Command::Library => library::open(cx),
        Command::Settings => settings::open(cx),
        Command::Annotate(path) => crate::editor::open(cx, path, None, None),
        Command::Toast(path) => {
            stage::show_toast(cx, path);
            Ok(())
        }
        Command::RecordToggle => recording::toggle_recording(cx),
        Command::RecordRegionPick => recording::record_region_pick(cx),
        Command::RecordRegion { x, y, w, h } => recording::record_region_start(cx, *x, *y, *w, *h),
        Command::RecordPause => {
            let paused = RECORDING
                .lock()
                .as_mut()
                .ok_or("no recording is active")?
                .toggle_pause();
            chip::set_paused(cx, paused);
            Ok(())
        }
        Command::RecordMic => {
            let on = RECORDING
                .lock()
                .as_mut()
                .ok_or("no recording is active")?
                .toggle_mic()?;
            chip::set_mic(cx, on);
            Ok(())
        }
        Command::RecordingEnded => {
            recording::collect_ended(cx);
            Ok(())
        }
        Command::LibraryChanged => {
            library::store_changed(cx);
            Ok(())
        }
        Command::Failed { what, err } => {
            crate::notice::failed(cx, what, err);
            Ok(())
        }
        // The quit observer `start` registers saves a live or flushing
        // recording before the process exits.
        Command::Quit => {
            cx.quit();
            Ok(())
        }
    }
}

/// Settings saved: reload the hotkey grabs.
pub fn notify_hotkeys_changed() {
    crate::sys::hotkeys::request_regrab();
}

/// The daemon's quit, on `--quit` and when the display server goes
/// away. It saves the recording first: a process that exits mid-flush
/// leaves no output file. Then it ends the process, before GPUI closes
/// the windows and drops the GPU device: the OS frees both, and on a
/// display that is gone a driver's swapchain teardown can block. With
/// its X server killed mid-present, lavapipe's X11 swapchain teardown
/// holds the process about 5 s.
fn quit(_: &mut App) -> std::future::Ready<()> {
    recording::save_before_exit();
    std::process::exit(0)
}

/// Start daemon services inside the GPUI app: the single-instance
/// socket, the tray icon, and the global hotkey grabs. The socket and
/// command pump are channel-driven; failures degrade to log lines.
pub fn start(cx: &mut App) {
    // The daemon outlives every surface: GPUI's Linux and Windows run
    // loops otherwise stop when the last window closes, and a parked
    // stand-in window would keep a renderer and its frame timer live.
    cx.set_quit_on_last_window_closed(false);
    cx.on_app_quit(quit).detach();
    iris_lib::ilog!("iris: daemon start");
    // The socket binds before the overlay warmup's renderer init: a
    // client that spawned this daemon waits for the bind, and a second
    // `iris` started meanwhile forwards here instead of starting another
    // daemon. A command that lands during the warmup runs once `start`
    // returns.
    match crate::sys::ipc::spawn_listener() {
        // The accept thread pushes each connection's argv here; the
        // pump parses and dispatches it. Channel-driven, so a forwarded
        // command lands the instant it connects on every platform.
        Ok(mut ipc_rx) => {
            cx.spawn(async move |cx| {
                while let Some(args) = ipc_rx.next().await {
                    let parsed = crate::cli::parse(&args);
                    // Only a client of another version sends an option
                    // this daemon does not parse; the rest still runs.
                    for e in &parsed.errors {
                        iris_lib::ilog!("iris: forwarded command line: {e}");
                    }
                    let _ = cx.update(|cx| {
                        for cmd in parsed.cmds {
                            run(cx, &cmd);
                        }
                    });
                }
            })
            .detach();
        }
        Err(e) => iris_lib::ilog!("iris: single-instance socket unavailable: {e}"),
    }
    // Warm the overlay pool: the first capture reuses a live window
    // instead of paying GPUI's ~130ms renderer init on the hotkey.
    overlay::warmup(cx);
    let (tx, rx) = unbounded::<Command>();
    let _ = COMMAND_TX.set(tx.clone());
    // A store write runs on whichever thread saved or deleted a capture;
    // the command pump brings it to the open library window.
    iris_lib::library::on_write(|| {
        if let Some(tx) = command_tx() {
            let _ = tx.unbounded_send(Command::LibraryChanged);
        }
    });

    crate::sys::hotkeys::spawn(tx.clone());
    crate::sys::tray::spawn(tx.clone());
    // Command pump: tray + hotkey threads -> app dispatch. The
    // receiver is a stream, so a press dispatches the instant it
    // arrives instead of up to a poll interval late.
    cx.spawn(async move |cx| {
        let mut rx = rx;
        while let Some(cmd) = rx.next().await {
            let _ = cx.update(|cx| run(cx, &cmd));
        }
    })
    .detach();
}

// WHY: the class closed here is "a client spawns a daemon only to hand
// it a command that needs an existing one": `iris --quit` with nothing
// running started a full daemon (GPUI init, overlay warmup) just to
// stop it, which held iris.exe open while the installer overwrote it.
// Not covered: the socket probe itself, which needs a live listener.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    #[test]
    fn live_state_commands_never_spawn_a_daemon() {
        use Command::*;
        for cmds in [
            vec![Quit],
            vec![RecordPause],
            vec![RecordMic],
            vec![RecordPause, RecordMic],
            vec![Quit, RecordPause],
        ] {
            assert!(live_daemon_only(&cmds), "{cmds:?}");
        }
    }

    #[test]
    fn commands_that_start_work_may_spawn_one() {
        use Command::*;
        for cmds in [
            vec![],
            vec![Capture],
            vec![CaptureFullscreen],
            vec![CaptureWindow],
            vec![Delayed(3)],
            vec![Library],
            vec![Settings],
            vec![Home],
            vec![RecordToggle],
            vec![RecordRegionPick],
            vec![Annotate("shot.png".into())],
            vec![Toast("shot.png".into())],
            vec![Quit, Capture],
            vec![RecordPause, RecordToggle],
        ] {
            assert!(!live_daemon_only(&cmds), "{cmds:?}");
        }
    }
}
