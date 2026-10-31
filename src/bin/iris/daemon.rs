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

use crate::{chip, library, overlay, settings, stage};

mod capture;
mod recording;

/// The daemon's live recording, if any: one at a time. The record
/// thread owns the border strips; the chip window is a GPUI surface.
/// The tray menu's record row follows it.
pub(crate) static RECORDING: recording::Live =
    recording::Live::new(crate::sys::tray::set_recording);

/// The daemon's command channel, installed by `start`.
pub(crate) static COMMAND_TX: std::sync::OnceLock<UnboundedSender<Command>> =
    std::sync::OnceLock::new();

/// The command channel sender for a recording source's end notice,
/// which runs on the source thread.
pub(crate) fn command_tx() -> Option<UnboundedSender<Command>> {
    COMMAND_TX.get().cloned()
}

/// How long after the daemon starts the overlay warmup runs: past the
/// first frame of a window the start opens, the library on a launch,
/// and of one a client that waited for the bind forwards.
const WARMUP_AFTER: Duration = Duration::from_millis(400);

/// One command line, forwarded to or invoked on the daemon.
#[derive(Debug, Clone)]
pub enum Command {
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
    /// Stop the live recording: the chip's stop button. Unlike
    /// RecordToggle it never starts one.
    RecordStop,
    /// A recording source returned: the daemon collects the recording
    /// when it ended on its own, not by a stop.
    RecordingEnded,
    /// The capture store changed in this process: an open library
    /// window shows the change now.
    LibraryChanged,
    /// The global hotkeys were registered again: open windows redraw
    /// the shortcut state they show.
    HotkeysBound,
    /// A thread with no window of its own failed at `what`: the daemon
    /// shows `err` in a notice.
    Failed {
        what: &'static str,
        err: String,
    },
    /// The background update check found `info`, or no update for
    /// `None`: the tray shows or hides its install item, and
    /// `announce` shows the update notice.
    UpdateOffer {
        info: Option<crate::update::UpdateInfo>,
        announce: bool,
    },
    /// The install item of the tray or of the update notice: download,
    /// verify, and install the offered update.
    InstallUpdate,
    Quit,
}

impl Command {
    /// The notice title when this command fails. The match is
    /// exhaustive so a new command fails to compile until it has one.
    pub(crate) fn failure_title(&self) -> &'static str {
        match self {
            Command::Capture
            | Command::CaptureFullscreen
            | Command::CaptureWindow
            | Command::Delayed(_) => "Capture failed",
            Command::RecordToggle
            | Command::RecordRegionPick
            | Command::RecordRegion { .. }
            | Command::RecordStop
            | Command::RecordingEnded => "Recording failed",
            Command::LibraryChanged => "Library did not refresh",
            Command::HotkeysBound => "Shortcuts did not refresh",
            Command::RecordPause => "Pause failed",
            Command::RecordMic => "Microphone toggle failed",
            Command::Library => "Library did not open",
            Command::Settings => "Settings did not open",
            Command::Annotate(_) => "Editor did not open",
            Command::Toast(_) => "Toast did not open",
            Command::Failed { what, .. } => what,
            Command::UpdateOffer { .. } => "Update notice did not open",
            Command::InstallUpdate => "Update failed",
            Command::Quit => "Quit failed",
        }
    }

    /// How this command uses the screen. The match is exhaustive so a
    /// new command fails to compile until it is classified.
    fn screen(&self) -> Screen {
        match self {
            Command::Capture
            | Command::CaptureFullscreen
            | Command::CaptureWindow
            | Command::RecordRegionPick => Screen::Starts,
            // Starts a recording when none is live, else stops it.
            Command::RecordToggle if !RECORDING.active() => Screen::Starts,
            Command::RecordRegion { .. } => Screen::Commits,
            // Its card counts down first; the capture it sends reads.
            Command::Delayed(_)
            // Stops the live recording.
            | Command::RecordToggle
            | Command::Library
            | Command::Settings
            | Command::Annotate(_)
            | Command::Toast(_)
            | Command::RecordPause
            | Command::RecordMic
            | Command::RecordStop
            | Command::RecordingEnded
            | Command::LibraryChanged
            | Command::HotkeysBound
            | Command::Failed { .. }
            | Command::UpdateOffer { .. }
            | Command::InstallUpdate
            | Command::Quit => Screen::Untouched,
        }
    }
}

/// How a command uses the screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Untouched,
    /// It starts reading the screen: a capture, the region pick over the
    /// live desktop, or a recording's start.
    Starts,
    /// The overlay hands over the region it picked, and the recording
    /// starts reading it while that overlay is still up.
    Commits,
}

/// Send `err` from a thread with no window to the daemon's notice.
/// Outside the daemon it is only logged.
pub(crate) fn report_failure(what: &'static str, err: String) {
    iris_lib::ilog!("iris: {what}: {err}");
    if let Some(tx) = command_tx() {
        let _ = tx.unbounded_send(Command::Failed { what, err });
    }
}

/// True when every command acts on a running daemon's live state:
/// quit, pause, the mic toggle, and stop. A fresh daemon holds none of
/// that state, so the client delivers these only to a daemon that is
/// already up and never spawns one for them. The match is exhaustive
/// so a new command fails to compile until it is classified.
pub fn live_daemon_only(cmds: &[Command]) -> bool {
    !cmds.is_empty()
        && cmds.iter().all(|c| match c {
            Command::Quit | Command::RecordPause | Command::RecordMic | Command::RecordStop => true,
            Command::Capture
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
            | Command::HotkeysBound
            | Command::Failed { .. }
            | Command::UpdateOffer { .. }
            | Command::InstallUpdate => false,
        })
}

/// Run `cmd` and show why it failed in a notice: a hotkey, the tray,
/// a toolbar button, or a forwarded command line has no other place
/// to show it. A command that starts or ends the recording redraws the
/// open windows: the library's Record button shows whether one runs.
pub fn run(cx: &mut App, cmd: &Command) {
    let recording = RECORDING.active();
    if let Err(e) = dispatch(cx, cmd) {
        iris_lib::ilog!("iris: {cmd:?}: {e}");
        crate::notice::failed(cx, cmd.failure_title(), &e);
    }
    if RECORDING.active() != recording {
        cx.refresh_windows();
    }
}

/// Take the resting toast and the live notice off the screen before
/// `cmd` reads it, and run `cmd` again once they are gone. Returns
/// whether `cmd` waits on them.
fn clear_popups_for(cx: &mut App, cmd: &Command) -> bool {
    if cmd.screen() == Screen::Untouched {
        return false;
    }
    let popups: Vec<AnyWindowHandle> = [
        stage::leave_for_capture(cx),
        crate::notice::leave_for_capture(cx),
    ]
    .into_iter()
    .flatten()
    .collect();
    if popups.is_empty() {
        return false;
    }
    crate::widgets::clear_then_run(cx, popups, cmd.clone());
    true
}

/// Run one command against the live app. A caller with a surface of its
/// own for the error (the library's status line) takes it here; every
/// other caller goes through `run`.
pub fn dispatch(cx: &mut App, cmd: &Command) -> Result<(), String> {
    // A second session would read the first one's overlay, and a window
    // pick takes the next click, which would land on a new overlay.
    if cmd.screen() == Screen::Starts
        && (overlay::session_live(cx) || crate::sys::record::picking())
    {
        iris_lib::ilog!(
            "iris: {cmd:?} ignored: a capture overlay or a window pick holds the screen"
        );
        return Ok(());
    }
    if clear_popups_for(cx, cmd) {
        return Ok(());
    }
    match cmd {
        Command::Capture => capture::capture_region(cx),
        Command::CaptureFullscreen => capture::capture_fullscreen(cx),
        Command::CaptureWindow => capture::capture_active_window(cx),
        Command::Delayed(secs) => {
            crate::notice::countdown(cx, *secs);
            Ok(())
        }
        Command::Library => library::open(cx),
        Command::Settings => settings::open(cx),
        Command::Annotate(path) => crate::editor::open(cx, path, None, None),
        Command::Toast(path) => {
            stage::show_toast(cx, path, None);
            Ok(())
        }
        Command::RecordToggle => recording::toggle_recording(cx),
        Command::RecordRegionPick => recording::record_region_pick(cx),
        Command::RecordRegion { x, y, w, h } => recording::record_region_start(cx, *x, *y, *w, *h),
        Command::RecordPause => {
            let paused = RECORDING.with(|rec| {
                rec.map(|rec| rec.toggle_pause())
                    .ok_or("no recording is active")
            })?;
            chip::set_paused(cx, paused);
            Ok(())
        }
        Command::RecordMic => {
            let on = RECORDING.with(|rec| {
                rec.ok_or("no recording is active".to_string())
                    .and_then(|rec| rec.toggle_mic())
            })?;
            chip::set_mic(cx, on);
            Ok(())
        }
        Command::RecordStop => recording::stop(cx),
        Command::RecordingEnded => {
            recording::collect_ended(cx);
            Ok(())
        }
        Command::LibraryChanged => {
            library::store_changed(cx);
            Ok(())
        }
        Command::HotkeysBound => {
            cx.refresh_windows();
            Ok(())
        }
        Command::Failed { what, err } => {
            crate::notice::failed(cx, what, err);
            Ok(())
        }
        Command::UpdateOffer { info, announce } => {
            crate::sys::tray::set_offer(info.as_ref().map(|i| &i.version));
            if let (Some(info), true) = (info, *announce) {
                crate::notice::update_available(cx, &info.version);
            }
            Ok(())
        }
        Command::InstallUpdate => crate::update::install_offered(),
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
/// holds the process about 5 s. `_exit` also skips the destructors and
/// exit handlers of the loaded libraries, which `std::process::exit`
/// runs: the destructor of NVIDIA's Vulkan driver makes a round trip to
/// the X server, and a stopped server never answers it.
fn quit(_: &mut App) -> std::future::Ready<()> {
    recording::save_before_exit();
    // Rust's buffered stdout is the one flush of `std::process::exit`
    // that iris needs.
    let _ = std::io::Write::flush(&mut std::io::stdout());
    // SAFETY: _exit takes no pointers and does not return.
    unsafe { libc::_exit(0) }
}

/// Start daemon services inside the GPUI app: the global hotkey grabs,
/// the single-instance socket, and the tray icon. The socket and command
/// pump are channel-driven; failures degrade to log lines. `claimed`
/// proves this process is the one daemon, so the socket is its to bind.
pub fn start(cx: &mut App, claimed: crate::sys::ipc::Claimed) {
    // The daemon outlives every surface: GPUI otherwise quits on Linux
    // and Windows when the last window closes, and a parked stand-in
    // window would keep a renderer and its frame timer live.
    cx.set_quit_mode(QuitMode::Explicit);
    cx.on_app_quit(quit).detach();
    let (tx, rx) = unbounded::<Command>();
    let _ = COMMAND_TX.set(tx.clone());
    // The hotkeys are grabbed before the socket binds, so a caller that
    // waits for the socket can press one at once. A press during the
    // overlay warmup waits on the channel and runs once `start` returns.
    crate::sys::hotkeys::spawn(tx.clone());
    iris_lib::ilog!("iris: starting");
    // The socket binds before the overlay warmup's renderer init: every
    // client that finds this daemon starting waits for the bind. A
    // command that lands during the warmup runs once `start` returns.
    match crate::sys::ipc::spawn_listener(claimed) {
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
                    cx.update(|cx| {
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
    // The background update check reports through the command pump.
    crate::update::spawn_background_check(tx.clone());
    crate::sys::tray::spawn(tx);
    // A store write runs on whichever thread saved or deleted a capture;
    // the command pump brings it to the open library window.
    iris_lib::library::on_write(|| {
        if let Some(tx) = command_tx() {
            let _ = tx.unbounded_send(Command::LibraryChanged);
        }
    });
    // Warm the overlay pool once the start's own window is up: the
    // warmup builds a renderer (~130ms on the main thread), and run
    // inline it held back the library a launch opens, and a forwarded
    // `--library` behind it. A capture before the warmup opens
    // a fresh shell. The install tidy runs after it: the `iris
    // --update` that started this daemon exits right after the start,
    // and Windows deletes no file a process still runs. The first-start
    // integration of an AppImage or an iris.app runs off the main
    // thread after both.
    cx.spawn(async move |cx| {
        cx.background_executor().timer(WARMUP_AFTER).await;
        cx.update(|cx| {
            // The shell maps over the whole screen before it parks, and
            // a window pick waits on a click there: the first capture
            // after the recording opens and pools the shell instead.
            if !RECORDING.active() {
                overlay::warmup(cx);
            }
            crate::flash::warmup(cx);
            crate::sys::install::tidy();
            cx.background_executor()
                .spawn(async { crate::sys::install::integrate() })
                .detach();
        });
    })
    .detach();
    // Command pump: tray + hotkey threads -> app dispatch. The
    // receiver is a stream, so a press dispatches the instant it
    // arrives instead of up to a poll interval late.
    cx.spawn(async move |cx| {
        let mut rx = rx;
        while let Some(cmd) = rx.next().await {
            cx.update(|cx| run(cx, &cmd));
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
            vec![RecordStop],
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
            vec![RecordToggle],
            vec![RecordRegionPick],
            vec![Annotate("shot.png".into())],
            vec![Toast("shot.png".into())],
            vec![UpdateOffer {
                info: None,
                announce: false,
            }],
            vec![InstallUpdate],
            vec![Quit, Capture],
            vec![RecordPause, RecordToggle],
        ] {
            assert!(!live_daemon_only(&cmds), "{cmds:?}");
        }
    }
}
