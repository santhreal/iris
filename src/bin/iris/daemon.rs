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

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};

use iris_lib::config::Config;
use iris_lib::record;
use gpui::*;

use crate::{chip, flash, library, overlay, pipeline, settings, stage};

/// The daemon-owned recording session. The record thread owns the
/// border strips; the chip window is a GPUI surface repositioned by
/// XID through `XcbChip`.
static RECORDING: std::sync::LazyLock<parking_lot::Mutex<record::RecordingManager>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(record::RecordingManager::default()));

/// The daemon's command channel, installed by `start`.
static COMMAND_TX: std::sync::OnceLock<Sender<Command>> = std::sync::OnceLock::new();

fn command_tx() -> Option<Sender<Command>> {
    COMMAND_TX.get().cloned()
}

/// ChipFollow that moves the GPUI chip window by XID (no app round-trip
/// per move) and asks the daemon to close it at the end. The XID is
/// resolved lazily: at window-open time the X window may not be in the
/// tree yet.
struct XcbChip {
    xid: std::sync::atomic::AtomicU32,
    conn: Option<x11rb::rust_connection::RustConnection>,
    done: Option<Sender<Command>>,
}

impl XcbChip {
    fn resolve_xid(&self) -> u32 {
        let cached = self.xid.load(std::sync::atomic::Ordering::SeqCst);
        if cached != 0 {
            return cached;
        }
        let Some(conn) = &self.conn else { return 0 };
        if let Some(xid) = crate::chip::find_chip_xid_on(conn) {
            self.xid.store(xid, std::sync::atomic::Ordering::SeqCst);
            return xid;
        }
        0
    }
}

impl record::x11::ChipFollow for XcbChip {
    fn place(&self, rect: record::x11::Rect) {
        let Some(conn) = &self.conn else { return };
        let xid = self.resolve_xid();
        if xid == 0 {
            return;
        }
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt};
        // Bleed-compensated: the pill sits 16px inside the window.
        let x = i32::from(rect.x) + i32::from(rect.w) - 148 - 16;
        let y = (i32::from(rect.y) - 44 - 16).max(0);
        let _ = conn.configure_window(xid, &ConfigureWindowAux::new().x(x).y(y));
        let _ = conn.flush();
    }
    fn hide(&self) {
        if let Some(tx) = &self.done {
            let _ = tx.send(Command::ChipHide);
        }
    }
}

/// One command line, forwarded to or invoked on the daemon.
#[derive(Debug)]
pub enum Command {
    Home,
    Capture,
    CaptureFullscreen,
    Library,
    Settings,
    Annotate(PathBuf),
    Toast(PathBuf),
    RecordToggle,
    ChipHide,
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
            "--library" => cmds.push(Command::Library),
            "--settings" => cmds.push(Command::Settings),
            "--quit" => cmds.push(Command::Quit),
            "--home" => cmds.push(Command::Home),
            "--record-window" => cmds.push(Command::RecordToggle),
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

/// Region capture: the overlay shells map at once, transparent over
/// the live desktop with crosshair and window-snap already live. The
/// grab and the per-monitor slices run behind them and each frozen
/// frame fades in as it lands, so the keypress reads as instant even
/// on a multi-4K setup where the grab takes half a second.
fn capture_region(cx: &mut App) -> Result<(), String> {
    let t0 = Instant::now();
    // The grab goes out FIRST: the X server reads the framebuffer
    // when it processes the request, before any overlay pixel can
    // map, so our own windows can never enter the frozen frame.
    let grab = cx
        .background_executor()
        .spawn(async move { pipeline::grab_frame() });
    // The layout query is cheap (xrandr + window list, single-digit
    // ms); the shell opens on it immediately.
    let layout = overlay::layout();
    // Reuse the pooled window when there is one: GPUI window init is
    // ~65% of keypress-to-overlay latency, and a parked window skips
    // all of it. A live session (visible overlay) swallows the press.
    let pooled = overlay::POOL.lock().unwrap().clone();
    let handle = if let Some((h, class)) = pooled {
        let session = h.update(cx, |o, _, cx| {
            if o.hidden {
                o.reset(&layout);
                cx.notify();
                true
            } else {
                false
            }
        });
        match session {
            Ok(true) => {
                let u = &layout.union;
                crate::xwin::unpark_span(class, u.x, u.y, u.width, u.height);
                h
            }
            // Busy mid-session: no stacking. Dead handle: fresh open.
            Ok(false) => return Ok(()),
            Err(e) => {
                eprintln!("iris: capture: pooled handle dead ({e:?}), reopening");
                overlay::open_shell(cx, &layout)?
            }
        }
    } else {
        overlay::open_shell(cx, &layout)?
    };
    eprintln!("iris: capture: shell in {:?}", t0.elapsed());
    cx.spawn(async move |cx| {
        let grabbed = cx
            .background_executor()
            .spawn(async move {
                let t_grab = Instant::now();
                let frame = Arc::new(grab.await?);
                let grab_ms = t_grab.elapsed();
                let t_slice = Instant::now();
                let img = overlay::slice_frame(&frame);
                eprintln!(
                    "iris: capture: grab {:?}, slice {:?}",
                    grab_ms,
                    t_slice.elapsed()
                );
                Ok::<(Arc<iris_lib::capture::Frame>, Arc<gpui::RenderImage>), String>((frame, img))
            })
            .await;
        let _ = cx.update(|cx| match grabbed {
            Ok((frame, img)) => {
                eprintln!("iris: capture: frame landed in {:?}", t0.elapsed());
                let _ = handle.update(cx, |overlay, window, cx| {
                    overlay.set_frame(frame, img, window, cx);
                    cx.notify();
                });
            }
            Err(e) => {
                eprintln!("iris: capture: {e}");
                let _ = handle.update(cx, |overlay, window, cx| {
                    overlay.cancel(window, cx);
                });
            }
        });
    })
    .detach();
    Ok(())
}

/// Full-screen capture: same flash discipline, no overlay. The
/// grab and the PNG save run off the main thread; a 4K frame
/// would otherwise freeze the app for seconds.
fn capture_fullscreen(cx: &mut App) -> Result<(), String> {
    fn grab_and_finish() -> Result<(PathBuf, iris_lib::library::CaptureEntry), String> {
        let frame = pipeline::grab_frame()?;
        // The region is the whole frame by construction; crop would
        // copy up to 200MB for nothing.
        let img = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)
            .ok_or("frame buffer size mismatch")?;
        let done = pipeline::finalize(&img)?;
        pipeline::play_shutter_sound();
        Ok(done)
    }
    let flash = Config::load().flash_on_capture;
    if flash {
        flash::show(cx)?;
    }
    cx.spawn(async move |cx| {
        if flash {
            cx.background_executor()
                .timer(Duration::from_millis(flash::duration_ms()))
                .await;
        }
        let done = cx
            .background_executor()
            .spawn(async move { grab_and_finish() })
            .await;
        let _ = cx.update(|cx| match done {
            Ok((path, entry)) => {
                if Config::load().show_toast_after_capture {
                    if let Err(e) = stage::show_toast(cx, &path, &entry.thumb, entry.width, entry.height) {
                        eprintln!("iris: capture: {e}");
                    }
                }
            }
            Err(e) => eprintln!("iris: capture: {e}"),
        });
    })
    .detach();
    Ok(())
}

/// Run one command against the live app.
pub fn dispatch(cx: &mut App, cmd: &Command) -> Result<(), String> {
    match cmd {
        Command::Home => crate::home::open(cx),
        Command::Capture => capture_region(cx),
        Command::CaptureFullscreen => capture_fullscreen(cx),
        Command::Library => library::open(cx),
        Command::Settings => settings::open(cx),
        Command::Annotate(path) => crate::editor::open(cx, path, None, None),
        Command::Toast(path) => stage::show_toast(cx, path, path, 0, 0),
        Command::RecordToggle => toggle_recording(cx),
        Command::ChipHide => {
            chip::close(cx);
            Ok(())
        }
        Command::Quit => {
            cx.quit();
            Ok(())
        }
    }
}

/// One action for the record hotkey/tray/CLI: stop when active, start a
/// window-picked recording when idle.
fn toggle_recording(cx: &mut App) -> Result<(), String> {
    let mut mgr = RECORDING.lock();
    if mgr.is_active() {
        match mgr.stop()? {
            Some(path) => eprintln!("iris: recording saved: {}", path.display()),
            None => {}
        }
        chip::close(cx); // defensive: any exit path that missed hide
        return Ok(());
    }
    let cfg = Config::load();
    let output = record::unique_recording_path(&cfg.recordings_dir);
    let mic = cfg.record_mic_default;
    let wayland_only = std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("DISPLAY").is_none();
    let active = if wayland_only {
        record::ActiveRecording::spawn(output, cfg.recording_fps, mic, move |spec| {
            record::wayland::record_window(spec)
        })
    } else {
        let xid = chip::open(cx, mic)?;
        let conn = x11rb::connect(None).ok().map(|(c, _)| c);
        let follower = std::sync::Arc::new(XcbChip {
            xid: std::sync::atomic::AtomicU32::new(xid),
            conn,
            done: command_tx(),
        });
        record::ActiveRecording::spawn(output, cfg.recording_fps, mic, move |spec| {
            record::x11::record_window_follow(spec, follower)
        })
    };
    mgr.active = Some(active);
    Ok(())
}

// ---- single instance -------------------------------------------------

fn socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|b| b.runtime_dir().map(|r| r.to_path_buf())).flatten())
        .unwrap_or_else(std::env::temp_dir);
    base.join("iris.sock")
}

/// If a daemon is already running, forward these args and exit the
/// process. Returns true when the caller must exit. A bare invocation
/// (no flags) surfaces the library in the running daemon.
pub fn forward_if_running(args: &[String]) -> bool {
    let path = socket_path();
    let Ok(mut stream) = UnixStream::connect(&path) else {
        return false;
    };
    let effective: &[String] = if args.is_empty() {
        &["--home".to_string()]
    } else {
        args
    };
    let payload = effective.join("\n");
    if stream.write_all(payload.as_bytes()).is_err() {
        return false;
    }
    true
}

fn bind_socket() -> Result<UnixListener, String> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("bind {}: {e}", path.display()))?;
    // The socket drives screen capture; only this user may connect.
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("nonblocking socket: {e}"))?;
    Ok(listener)
}

fn accept_args(listener: &UnixListener) -> Vec<String> {
    let mut args = Vec::new();
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut buf = String::new();
                if stream.read_to_string(&mut buf).is_ok() {
                    args.extend(buf.lines().map(|l| l.to_string()).filter(|l| !l.is_empty()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    args
}

// ---- tray ------------------------------------------------------------

#[cfg(target_os = "linux")]
struct IrisTray {
    tx: Sender<Command>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for IrisTray {
    fn id(&self) -> String {
        "iris".into()
    }
    fn title(&self) -> String {
        "iris".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // 22x22: white shutter glyph on transparent. Plain ARGB32.
        let n = 22;
        let mut data = vec![0u8; n * n * 4];
        for y in 4..18 {
            for x in 4..18 {
                let dx = x as i32 - 11;
                let dy = y as i32 - 11;
                let ring = (36..=49).contains(&(dx * dx + dy * dy));
                let dot = dx * dx + dy * dy <= 4;
                if ring || dot {
                    let i = (y * n + x) * 4;
                    data[i..i + 4].copy_from_slice(&[242, 242, 244, 255]);
                }
            }
        }
        vec![ksni::Icon {
            width: n as i32,
            height: n as i32,
            data,
        }]
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let item = |label: &str, cmd: Command| {
            let tx = self.tx.clone();
            StandardItem {
                label: label.to_string(),
                activate: Box::new(move |_| {
                    let _ = tx.send(cmd.clone_for_menu());
                }),
                ..Default::default()
            }
            .into()
        };
        vec![
            item("Capture", Command::Capture),
            item("Record window", Command::RecordToggle),
            item("Library", Command::Library),
            item("Settings", Command::Settings),
            MenuItem::Separator,
            item("Quit", Command::Quit),
        ]
    }
}

impl Command {
    /// Menu closures need 'static sends; Command is small, clone it.
    fn clone_for_menu(&self) -> Command {
        match self {
            Command::Home => Command::Home,
            Command::Capture => Command::Capture,
            Command::CaptureFullscreen => Command::CaptureFullscreen,
            Command::Library => Command::Library,
            Command::Settings => Command::Settings,
            Command::Annotate(p) => Command::Annotate(p.clone()),
            Command::Toast(p) => Command::Toast(p.clone()),
            Command::RecordToggle => Command::RecordToggle,
            Command::ChipHide => Command::ChipHide,
            Command::Quit => Command::Quit,
        }
    }
}

// ---- global hotkeys (X11) --------------------------------------------

#[cfg(target_os = "linux")]
mod hotkeys {
    use super::{Command, Config, Sender};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::*;

    /// Keysym values for the hotkey grammar (X11 keysym numbers).
    fn keysym_for(name: &str) -> Option<u32> {
        let lower = name.to_lowercase();
        Some(match lower.as_str() {
            "print" | "printscreen" | "prtscn" | "prtsc" | "prtscr" => 0xff61,
            "escape" | "esc" => 0xff1b,
            "enter" | "return" => 0xff0d,
            "space" | "spacebar" => 0x20,
            "tab" => 0xff09,
            "backspace" => 0xff08,
            "delete" | "del" => 0xffff,
            "home" => 0xff50,
            "end" => 0xff57,
            "pageup" | "pgup" => 0xff55,
            "pagedown" | "pgdn" => 0xff56,
            "up" | "arrowup" => 0xff52,
            "down" | "arrowdown" => 0xff54,
            "left" | "arrowleft" => 0xff51,
            "right" | "arrowright" => 0xff53,
            s if s.len() == 1 => {
                let c = s.chars().next()?;
                match c {
                    'a'..='z' => c as u32,
                    '0'..='9' => c as u32,
                    _ => return None,
                }
            }
            s if s.starts_with('f') => {
                let n: u32 = s[1..].parse().ok()?;
                if !(1..=24).contains(&n) {
                    return None;
                }
                0xffbd + n
            }
            _ => return None,
        })
    }

    /// "Ctrl+Shift+R" -> (modifier mask, keysym).
    fn parse_hotkey(s: &str) -> Option<(ModMask, u32)> {
        let mut mods = ModMask::from(0u16);
        let mut key = None;
        for tok in s.split('+').map(|t| t.trim()).filter(|t| !t.is_empty()) {
            match tok.to_lowercase().as_str() {
                "ctrl" | "control" => mods |= ModMask::CONTROL,
                "shift" => mods |= ModMask::SHIFT,
                "alt" | "option" => mods |= ModMask::M1,
                "super" | "cmd" | "command" | "meta" | "win" | "windows" => mods |= ModMask::M4,
                name => key = keysym_for(name),
            }
        }
        key.map(|k| (mods, k))
    }

    /// First keycode producing `keysym` at any shift level.
    fn keycode_for(conn: &impl Connection, keysym: u32) -> Option<u8> {
        let setup = conn.setup();
        let reply = conn
            .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)
            .ok()?
            .reply()
            .ok()?;
        let per = reply.keysyms_per_keycode as usize;
        for (i, chunk) in reply.keysyms.chunks(per).enumerate() {
            if chunk.contains(&keysym) {
                return Some(setup.min_keycode + i as u8);
            }
        }
        None
    }

    /// Signalled by settings save: ungrab everything, reload config,
    /// re-grab. The grab thread owns the connection.
    static REGRAB: std::sync::OnceLock<std::sync::mpsc::Sender<()>> =
        std::sync::OnceLock::new();

    pub fn request_regrab() {
        if let Some(tx) = REGRAB.get() {
            let _ = tx.send(());
        }
    }

    /// Grab the configured hotkeys on the root window and forward
    /// presses. Runs on its own thread; silently disabled on failure
    /// (Wayland, no DISPLAY).
    pub fn spawn(tx: Sender<Command>) {
        std::thread::spawn(move || {
            let (regrab_tx, regrab_rx) = std::sync::mpsc::channel::<()>();
            let _ = REGRAB.set(regrab_tx);
            let Ok((conn, screen_num)) = x11rb::connect(None) else {
                return;
            };
            let root = conn.setup().roots[screen_num].root;
            let locks = [
                ModMask::from(0u16),
                ModMask::M2,
                ModMask::LOCK,
                ModMask::M2 | ModMask::LOCK,
            ];
            let mut grabbed: Vec<(u8, Command)> = Vec::new();

            let grab_all = |conn: &x11rb::rust_connection::RustConnection,
                                grabbed: &mut Vec<(u8, Command)>| {
                let _ = conn.ungrab_key(0u8, root, ModMask::from(0x8000u16));
                grabbed.clear();
                let cfg = Config::load();
                for (hotkey, cmd) in [
                    (&cfg.capture_hotkey, Command::Capture),
                    (&cfg.record_hotkey, Command::RecordToggle),
                ] {
                    let Some((mods, keysym)) = parse_hotkey(hotkey) else {
                        eprintln!("iris: cannot parse hotkey {hotkey:?}");
                        continue;
                    };
                    let Some(keycode) = keycode_for(conn, keysym) else {
                        eprintln!("iris: no keycode for hotkey keysym {keysym:#x}");
                        continue;
                    };
                    let mut ok = true;
                    for extra in locks {
                        if conn
                            .grab_key(
                                false,
                                root,
                                mods | extra,
                                keycode,
                                GrabMode::ASYNC,
                                GrabMode::ASYNC,
                            )
                            .map_err(|e| e.to_string())
                            .and_then(|cookie| cookie.check().map_err(|e| e.to_string()))
                            .is_err()
                        {
                            eprintln!("iris: cannot grab hotkey keysym {keysym:#x}");
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        grabbed.push((keycode, cmd));
                    }
                }
                let _ = conn.flush();
            };
            grab_all(&conn, &mut grabbed);

            loop {
                if regrab_rx.try_recv().is_ok() {
                    grab_all(&conn, &mut grabbed);
                }
                // Poll (not wait_for_event): a regrab request must be
                // honored even when no X events arrive.
                loop {
                    match conn.poll_for_event() {
                        Ok(Some(x11rb::protocol::Event::KeyPress(ev))) => {
                            if let Some((_, cmd)) = grabbed.iter().find(|(kc, _)| *kc == ev.detail)
                            {
                                if tx.send(cmd.clone_for_menu()).is_err() {
                                    return;
                                }
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(_) => return,
                    }
                }
                // 8ms, not 100: a grabbed keypress must reach dispatch
                // inside a frame. The poll is a non-blocking drain, so
                // the tighter loop costs a wakeup, not work, and the
                // regrab check still runs between events.
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
        });
    }
}

// ---- daemon entry -----------------------------------------------------

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
    #[cfg(target_os = "linux")]
    hotkeys::request_regrab();
}

/// Start daemon services inside the GPUI app: the single-instance
/// socket, the tray icon, and the global hotkey grabs. Everything is
/// polled on one 100ms app timer; failures degrade to log lines.
pub fn start(cx: &mut App) {
    open_anchor(cx);
    // Warm the overlay pool: the first capture reuses a live window
    // instead of paying GPUI's ~130ms renderer init on the hotkey.
    overlay::warmup(cx);
    let (tx, rx) = channel::<Command>();
    let _ = COMMAND_TX.set(tx.clone());

    match bind_socket() {
        Ok(listener) => {
            cx.spawn(async move |cx| loop {
                // 16ms, not 100: a forwarded CLI command or tray click
                // should reach dispatch within a frame, not a tenth of
                // a second. try_iter is a non-blocking drain, so the
                // tighter poll costs a wakeup, not work.
                cx.background_executor().timer(Duration::from_millis(16)).await;
                let args = accept_args(&listener);
                if !args.is_empty() {
                    let cmds = parse_args(&args);
                    let _ = cx.update(|cx| {
                        for cmd in cmds {
                            if let Err(e) = dispatch(cx, &cmd) {
                                eprintln!("iris: dispatch: {e}");
                            }
                        }
                    });
                }
            })
            .detach();
        }
        Err(e) => eprintln!("iris: single-instance socket unavailable: {e}"),
    }

    #[cfg(target_os = "linux")]
    {
        hotkeys::spawn(tx.clone());
        let tray = IrisTray { tx: tx.clone() };
        std::thread::spawn(move || {
            use ksni::blocking::TrayMethods;
            match tray.spawn() {
                Err(e) => eprintln!("iris: tray unavailable: {e}"),
                Ok(handle) => {
                    // Keep the tray registered for the process lifetime.
                    let _keep = handle;
                    loop {
                        std::thread::park();
                    }
                }
            }
        });
    }

    // Command pump: tray + hotkey threads -> app dispatch. 16ms keeps
    // a hotkey press inside one frame of latency.
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(Duration::from_millis(16)).await;
        let pending: Vec<Command> = rx.try_iter().collect();
        if !pending.is_empty() {
            let _ = cx.update(|cx| {
                for cmd in pending {
                    if let Err(e) = dispatch(cx, &cmd) {
                        eprintln!("iris: dispatch: {e}");
                    }
                }
            });
        }
    })
    .detach();
}
