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
//! runs through `record::x11::record_window_follow` with the chip window
//! repositioned by XID through `XcbChip`.

use std::path::PathBuf;
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
use gpui::*;
use iris_lib::record;

#[cfg(target_os = "linux")]
use crate::chip;
use crate::{library, overlay, settings, stage};

mod capture;
mod recording;

/// The daemon-owned recording session. The record thread owns the
/// border strips; the chip window is a GPUI surface repositioned by
/// XID through `XcbChip`.
pub(crate) static RECORDING: std::sync::LazyLock<parking_lot::Mutex<record::RecordingManager>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(record::RecordingManager::default()));

/// The daemon's command channel, installed by `start`.
pub(crate) static COMMAND_TX: std::sync::OnceLock<UnboundedSender<Command>> =
    std::sync::OnceLock::new();

/// The command channel sender for the X11 chip's hide callback; only
/// Linux drives a chip that needs it.
#[cfg(target_os = "linux")]
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
    /// The X11 chip window asked to close; only Linux has a chip.
    #[cfg(target_os = "linux")]
    ChipHide,
    /// Print the running version and exit (handled before dispatch).
    Version,
    /// Report whether a newer release exists.
    CheckUpdate,
    /// Download and apply the latest release, then restart.
    Update,
    Quit,
}

/// Parse CLI-style args into commands. Unknown flags are ignored so an
/// old client never wedges a new daemon.
pub fn parse_args(args: &[String]) -> Vec<Command> {
    let mut cmds = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--capture" => cmds.push(Command::Capture),
            "--capture-fullscreen" => cmds.push(Command::CaptureFullscreen),
            "--capture-window" => cmds.push(Command::CaptureWindow),
            "--delay" => {
                if let Some(secs) = args.get(i + 1).and_then(|s| s.parse::<u64>().ok()) {
                    cmds.push(Command::Delayed(secs));
                    i += 1;
                }
            }
            "--settings" => cmds.push(Command::Settings),
            "--library" => cmds.push(Command::Library),
            "--quit" => cmds.push(Command::Quit),
            "--version" => cmds.push(Command::Version),
            "--check-update" => cmds.push(Command::CheckUpdate),
            "--update" => cmds.push(Command::Update),
            "--home" => cmds.push(Command::Home),
            "--record-window" => cmds.push(Command::RecordToggle),
            "--record-region" => cmds.push(Command::RecordRegionPick),
            "--record-pause" => cmds.push(Command::RecordPause),
            "--record-mic" => cmds.push(Command::RecordMic),
            "--annotate" => {
                if let Some(path) = args.get(i + 1) {
                    cmds.push(Command::Annotate(PathBuf::from(path)));
                    i += 1;
                }
            }
            "--toast" => {
                if let Some(path) = args.get(i + 1) {
                    cmds.push(Command::Toast(PathBuf::from(path)));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    cmds
}

/// Run one command against the live app.
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
                let _ = cx.update(|cx| {
                    if let Err(e) = capture::capture_fullscreen(cx) {
                        iris_lib::ilog!("iris: capture: {e}");
                    }
                });
            })
            .detach();
            Ok(())
        }
        Command::Library => library::open(cx),
        Command::Settings => settings::open(cx),
        Command::Annotate(path) => crate::editor::open(cx, path, None, None),
        Command::Toast(path) => stage::show_toast(cx, path, path, 0, 0),
        Command::RecordToggle => recording::toggle_recording(cx),
        Command::RecordRegionPick => recording::record_region_pick(cx),
        Command::RecordRegion { x, y, w, h } => recording::record_region_start(cx, *x, *y, *w, *h),
        Command::RecordPause => {
            let mgr = RECORDING.lock();
            if let Some(rec) = &mgr.active {
                #[cfg(target_os = "linux")]
                chip::set_paused(!chip::paused());
                #[cfg(target_os = "linux")]
                let paused = chip::paused();
                #[cfg(not(target_os = "linux"))]
                let paused = false;
                rec.send_control(if paused {
                    record::RecControl::Pause
                } else {
                    record::RecControl::Resume
                });
            }
            Ok(())
        }
        Command::RecordMic => {
            let mut mgr = RECORDING.lock();
            if let Some(rec) = &mut mgr.active {
                rec.mic = !rec.mic;
                #[cfg(target_os = "linux")]
                chip::set_mic(rec.mic);
                rec.send_control(record::RecControl::ToggleMic);
            }
            Ok(())
        }
        #[cfg(target_os = "linux")]
        Command::ChipHide => {
            chip::close(cx);
            Ok(())
        }
        Command::Version => {
            println!("iris {}", env!("CARGO_PKG_VERSION"));
            cx.quit();
            Ok(())
        }
        Command::CheckUpdate => {
            cx.spawn(async move |cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { crate::update::check() })
                    .await;
                match result {
                    Ok(Some(info)) => {
                        iris_lib::ilog!("iris: update available: {}", info.version)
                    }
                    Ok(None) => iris_lib::ilog!("iris: up to date"),
                    Err(e) => iris_lib::ilog!("iris: {e}"),
                }
            })
            .detach();
            Ok(())
        }
        Command::Update => {
            cx.spawn(async move |cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        match crate::update::check()? {
                            Some(info) => crate::update::apply(&info),
                            None => {
                                iris_lib::ilog!("iris: up to date");
                                Ok(())
                            }
                        }
                    })
                    .await;
                if let Err(e) = result {
                    iris_lib::ilog!("iris: {e}");
                }
            })
            .detach();
            Ok(())
        }
        Command::Quit => {
            // Flush an in-flight recording before the process exits:
            // quitting with the encoder live orphans ffmpeg mid-write
            // and leaves a truncated file.
            if let Ok(Some(path)) = RECORDING.lock().stop() {
                iris_lib::ilog!("iris: recording saved: {}", path.display());
            }
            cx.quit();
            Ok(())
        }
    }
}

/// The daemon must never die with its last surface: GPUI's X11 client
/// stops the event loop when the window list empties. This 1x1
/// transparent notification window stays mapped for the process
/// lifetime; notification windows are skipped by taskbars and alt-tab.
struct Anchor;

impl Render for Anchor {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

fn open_anchor(cx: &mut App) {
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: size(px(1.), px(1.)),
            })),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: None,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some("dev.iris.daemon".to_string()),
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
        },
        |_, cx| cx.new(|_| Anchor),
    );
}

/// Settings saved: reload the hotkey grabs.
pub fn notify_hotkeys_changed() {
    crate::sys::hotkeys::request_regrab();
}

/// Start daemon services inside the GPUI app: the single-instance
/// socket, the tray icon, and the global hotkey grabs. The socket and
/// command pump are channel-driven; failures degrade to log lines.
pub fn start(cx: &mut App) {
    open_anchor(cx);
    iris_lib::ilog!("iris: daemon start");
    // Warm the overlay pool: the first capture reuses a live window
    // instead of paying GPUI's ~130ms renderer init on the hotkey.
    overlay::warmup(cx);
    let (tx, rx) = unbounded::<Command>();
    let _ = COMMAND_TX.set(tx.clone());

    match crate::sys::ipc::spawn_listener() {
        // The accept thread pushes each connection's argv here; the
        // pump parses and dispatches it. Channel-driven, so a forwarded
        // command lands the instant it connects on every platform.
        Ok(mut ipc_rx) => {
            cx.spawn(async move |cx| {
                while let Some(args) = ipc_rx.next().await {
                    let cmds = parse_args(&args);
                    let _ = cx.update(|cx| {
                        for cmd in cmds {
                            if let Err(e) = dispatch(cx, &cmd) {
                                iris_lib::ilog!("iris: dispatch: {e}");
                            }
                        }
                    });
                }
            })
            .detach();
        }
        Err(e) => iris_lib::ilog!("iris: single-instance socket unavailable: {e}"),
    }

    crate::sys::hotkeys::spawn(tx.clone());
    crate::sys::tray::spawn(tx.clone());
    // Command pump: tray + hotkey threads -> app dispatch. The
    // receiver is a stream, so a press dispatches the instant it
    // arrives instead of up to a poll interval late.
    cx.spawn(async move |cx| {
        let mut rx = rx;
        while let Some(cmd) = rx.next().await {
            let _ = cx.update(|cx| {
                if let Err(e) = dispatch(cx, &cmd) {
                    iris_lib::ilog!("iris: dispatch: {e}");
                }
            });
        }
    })
    .detach();
}
