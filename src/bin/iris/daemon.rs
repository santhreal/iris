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

use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;
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
static COMMAND_TX: std::sync::OnceLock<UnboundedSender<Command>> = std::sync::OnceLock::new();

fn command_tx() -> Option<UnboundedSender<Command>> {
    COMMAND_TX.get().cloned()
}

/// ChipFollow that moves the GPUI chip window by XID (no app round-trip
/// per move) and asks the daemon to close it at the end. The XID is
/// resolved lazily: at window-open time the X window may not be in the
/// tree yet.
struct XcbChip {
    xid: std::sync::atomic::AtomicU32,
    conn: Option<&'static x11rb::rust_connection::RustConnection>,
    done: Option<UnboundedSender<Command>>,
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
            let _ = tx.unbounded_send(Command::ChipHide);
        }
    }
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
    RecordRegion { x: i32, y: i32, w: i32, h: i32 },
    RecordPause,
    RecordMic,
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
    let pooled = overlay::POOL.lock().clone();
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
                // Re-assert activation on GPUI's own connection: the
                // WM's map-time focus handling can land after the
                // helper thread's request.
                let _ = h.update(cx, |_, window, _| window.activate_window());
                h
            }
            // Busy mid-session: no stacking. Dead handle: fresh open.
            Ok(false) => return Ok(()),
            Err(e) => {
                iris_lib::ilog!("iris: capture: pooled handle dead ({e:?}), reopening");
                overlay::open_shell(cx, &layout)?
            }
        }
    } else {
        overlay::open_shell(cx, &layout)?
    };
    iris_lib::ilog!("iris: capture: shell in {:?}", t0.elapsed());
    cx.spawn(async move |cx| {
        let grabbed = cx
            .background_executor()
            .spawn(async move {
                let t_grab = Instant::now();
                let frame = grab.await?;
                let (width, height) = (frame.width, frame.height);
                let grab_ms = t_grab.elapsed();
                let t_slice = Instant::now();
                let img = overlay::slice_frame(frame);
                iris_lib::ilog!(
                    "iris: capture: grab {:?}, slice {:?}",
                    grab_ms,
                    t_slice.elapsed()
                );
                Ok::<(Arc<gpui::RenderImage>, u32, u32), String>((img, width, height))
            })
            .await;
        let _ = cx.update(|cx| match grabbed {
            Ok((img, width, height)) => {
                iris_lib::ilog!("iris: capture: frame landed in {:?}", t0.elapsed());
                let _ = handle.update(cx, |overlay, window, cx| {
                    overlay.set_frame(img, width, height, window, cx);
                    cx.notify();
                });
            }
            Err(e) => {
                iris_lib::ilog!("iris: capture: {e}");
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
                        iris_lib::ilog!("iris: capture: {e}");
                    }
                }
            }
            Err(e) => iris_lib::ilog!("iris: capture: {e}"),
        });
    })
    .detach();
    Ok(())
}

/// Capture the focused window's rect out of a full-screen grab. X11
/// only: the Wayland portal cannot name a window, so this reports an
/// honest error there.
fn capture_active_window(cx: &mut App) -> Result<(), String> {
    fn grab_and_finish() -> Result<(PathBuf, iris_lib::library::CaptureEntry), String> {
        let rect = iris_lib::capture::x11::active_window_rect()?;
        let frame = pipeline::grab_frame()?;
        // frame.rgba is RGBA; crop_bgra would swap R and B. The plain
        // copy crop keeps the channels as captured and bands across
        // threads on large windows.
        let rect = pipeline::Region {
            x: rect.x.max(0) as u32,
            y: rect.y.max(0) as u32,
            width: rect.width.min(frame.width.saturating_sub(rect.x.max(0) as u32)),
            height: rect.height.min(frame.height.saturating_sub(rect.y.max(0) as u32)),
        };
        if rect.width == 0 || rect.height == 0 {
            return Err("active window is outside the frame".to_string());
        }
        let crop = pipeline::crop_rgba(&frame.rgba, frame.width, frame.height, rect)?;
        let done = pipeline::finalize(&crop)?;
        pipeline::play_shutter_sound();
        Ok(done)
    }
    cx.spawn(async move |cx| {
        let done = cx
            .background_executor()
            .spawn(async move { grab_and_finish() })
            .await;
        let _ = cx.update(|cx| match done {
            Ok((path, entry)) => {
                if Config::load().show_toast_after_capture {
                    if let Err(e) = stage::show_toast(cx, &path, &entry.thumb, entry.width, entry.height) {
                        iris_lib::ilog!("iris: capture: {e}");
                    }
                }
            }
            Err(e) => iris_lib::ilog!("iris: capture: {e}"),
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
        Command::CaptureWindow => capture_active_window(cx),
        Command::Delayed(secs) => {
            let secs = *secs;
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(secs))
                    .await;
                let _ = cx.update(|cx| {
                    if let Err(e) = capture_fullscreen(cx) {
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
        Command::RecordToggle => toggle_recording(cx),
        Command::RecordRegionPick => record_region_pick(cx),
        Command::RecordRegion { x, y, w, h } => {
            record_region_start(cx, *x, *y, *w, *h)
        },
        Command::RecordPause => {
            let mgr = RECORDING.lock();
            if let Some(rec) = &mgr.active {
                chip::set_paused(!chip::paused());
                rec.send_control(if chip::paused() {
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
                chip::set_mic(rec.mic);
                rec.send_control(record::RecControl::ToggleMic);
            }
            Ok(())
        }
        Command::ChipHide => {
            chip::close(cx);
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

/// The recording parameters every source shares: output path under
/// the configured template, mic default, container and codec.
fn recording_params(
    cfg: &Config,
) -> (PathBuf, bool, iris_lib::config::RecordingFormat, iris_lib::config::RecordingEncoder) {
    let ext = match cfg.recording_format {
        iris_lib::config::RecordingFormat::Mp4 => "mp4",
        iris_lib::config::RecordingFormat::Gif => "gif",
        iris_lib::config::RecordingFormat::Webm => "webm",
    };
    (
        record::unique_recording_path(&cfg.recordings_dir, ext),
        cfg.record_mic_default,
        cfg.recording_format,
        cfg.recording_encoder,
    )
}

/// One action for the record hotkey/tray/CLI: stop when active, start a
/// window-picked recording when idle.
fn toggle_recording(cx: &mut App) -> Result<(), String> {
    if RECORDING.lock().is_active() {
        // The join (ffmpeg's trailer flush) can take seconds on a long
        // recording; it runs off the UI thread so hotkeys and socket
        // commands stay live while the file finishes.
        RECORDING.lock().stop_async(|result| {
            match result {
                Ok(Some(path)) => {
                    iris_lib::ilog!("iris: recording saved: {}", path.display());
                }
                Ok(None) => {}
                Err(e) => iris_lib::ilog!("iris: recording stop: {e}"),
            }
        });
        chip::close(cx); // defensive: any exit path that missed hide
        return Ok(());
    }
    let mut mgr = RECORDING.lock();
    let cfg = Config::load();
    let (output, mic, format, encoder) = recording_params(&cfg);
    let wayland_only = std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("DISPLAY").is_none();
    let active = if wayland_only {
        record::ActiveRecording::spawn(output, cfg.recording_fps, mic, format, encoder, move |spec| {
            record::wayland::record_window(spec)
        })
    } else {
        let xid = chip::open(cx, mic)?;
        let conn = iris_lib::capture::x11::shared_conn().ok().map(|(c, _)| c);
        let follower = std::sync::Arc::new(XcbChip {
            xid: std::sync::atomic::AtomicU32::new(xid),
            conn,
            done: command_tx(),
        });
        record::ActiveRecording::spawn(output, cfg.recording_fps, mic, format, encoder, move |spec| {
            record::x11::record_window_follow(spec, follower)
        })
    };
    mgr.active = Some(active);
    Ok(())
}

/// Region recording, step one: open the overlay in pick mode. The
/// frozen frame is not needed for picking, so the shell opens on the
/// live desktop and the grab still runs behind it for the loupe.
fn record_region_pick(cx: &mut App) -> Result<(), String> {
    // Same dead end as record_region_start: the picked rect feeds an
    // X11-only source, so on Wayland-only fail before the overlay opens.
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("DISPLAY").is_none()
    {
        return Err("region recording needs X11; on Wayland record a window".to_string());
    }
    let mgr = RECORDING.lock();
    if mgr.is_active() {
        return Err("a recording is already active".to_string());
    }
    let grab = cx
        .background_executor()
        .spawn(async move { pipeline::grab_frame() });
    let layout = overlay::layout();
    // Same pool path as capture_region: a parked window skips GPUI's
    // init, and reset() re-arms it before the mode flips to pick.
    let pooled = overlay::POOL.lock().clone();
    let handle = if let Some((h, class)) = pooled {
        let session = h.update(cx, |o, _, cx| {
            if o.hidden {
                o.reset(&layout);
                o.mode = overlay::OverlayMode::RecordPick;
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
                let _ = h.update(cx, |_, window, _| window.activate_window());
                h
            }
            Ok(false) => return Ok(()),
            Err(e) => {
                iris_lib::ilog!("iris: record-region: pooled handle dead ({e:?}), reopening");
                let h = overlay::open_shell(cx, &layout)?;
                h.update(cx, |o, _, cx| {
                    o.mode = overlay::OverlayMode::RecordPick;
                    cx.notify();
                }).map_err(|e| format!("set pick mode: {e}"))?;
                h
            }
        }
    } else {
        let h = overlay::open_shell(cx, &layout)?;
        h.update(cx, |o, _, cx| {
            o.mode = overlay::OverlayMode::RecordPick;
            cx.notify();
        }).map_err(|e| format!("set pick mode: {e}"))?;
        h
    };
    cx.spawn(async move |cx| {
        let grabbed = cx
            .background_executor()
            .spawn(async move {
                let frame = grab.await?;
                let (width, height) = (frame.width, frame.height);
                let img = overlay::slice_frame(frame);
                Ok::<(Arc<gpui::RenderImage>, u32, u32), String>((img, width, height))
            })
            .await;
        let _ = cx.update(|cx| match grabbed {
            Ok((img, width, height)) => {
                let _ = handle.update(cx, |overlay, window, cx| {
                    overlay.set_frame(img, width, height, window, cx);
                    cx.notify();
                });
            }
            Err(e) => {
                iris_lib::ilog!("iris: record-region: {e}");
                let _ = handle.update(cx, |overlay, window, cx| {
                    overlay.cancel(window, cx);
                });
            }
        });
    })
    .detach();
    Ok(())
}

/// Region recording, step two: the overlay committed a rect. Open the
/// chip at the rect's top-right and spawn the region source.
fn record_region_start(cx: &mut App, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    // Region recording reads the root window through X11 SHM; the
    // portal cannot name a rect, so on a Wayland-only session this
    // would open the chip and die on the first frame.
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var_os("DISPLAY").is_none()
    {
        return Err("region recording needs X11; on Wayland record a window".to_string());
    }
    let mut mgr = RECORDING.lock();
    if mgr.is_active() {
        return Err("a recording is already active".to_string());
    }
    let cfg = Config::load();
    let (output, mic, format, encoder) = recording_params(&cfg);
    let xid = chip::open(cx, mic)?;
    let conn = iris_lib::capture::x11::shared_conn().ok().map(|(c, _)| c);
    let follower = std::sync::Arc::new(XcbChip {
        xid: std::sync::atomic::AtomicU32::new(xid),
        conn,
        done: command_tx(),
    });
    let rect = record::x11::Rect { x: x as i16, y: y as i16, w: w as u16, h: h as u16 };
    let active = record::ActiveRecording::spawn(output, cfg.recording_fps, mic, format, encoder, move |spec| {
        record::x11::record_region(spec, follower, rect)
    });
    mgr.active = Some(active);
    Ok(())
}

// ---- single instance -------------------------------------------------

fn socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().and_then(|b| b.runtime_dir().map(|r| r.to_path_buf())))
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
                // The accepted socket does not inherit O_NONBLOCK, and a
                // client that connects but never writes or closes would
                // stall every later command on a blocking read. Bound
                // the read: 2s of poll, then give up on the peer.
                let mut buf = String::new();
                {
                    use std::io::Read;
                    use std::os::unix::io::AsRawFd;
                    let fd = stream.as_raw_fd();
                    let mut chunk = [0u8; 4096];
                    let started = std::time::Instant::now();
                    // drops its stream after the write) or 2s passes
                    // with no data. Payloads are argv lines; 1MiB caps
                    // a hostile flood.
                    loop {
                        let mut pfd = libc::pollfd {
                            fd,
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        if unsafe { libc::poll(&mut pfd, 1, 2000) } <= 0 {
                            break;
                        }
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
                                // 1MiB caps a hostile flood; the 5s
                                // total deadline caps a slow drip that
                                // keeps poll() fed forever.
                                if buf.len() > (1 << 20)
                                    || started.elapsed() > Duration::from_secs(5)
                                {
                                    break;
                                }
                            }
                        }
                    }
                }
                args.extend(buf.lines().map(|l| l.to_string()).filter(|l| !l.is_empty()));
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
    tx: UnboundedSender<Command>,
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
        // Static: ksni re-queries the pixmap on every tray update, so
        // the raster is built once and cloned.
        static ICON: std::sync::LazyLock<ksni::Icon> = std::sync::LazyLock::new(|| {
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
            ksni::Icon {
                width: n as i32,
                height: n as i32,
                data,
            }
        });
        vec![ICON.clone()]
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let item = |label: &str, cmd: Command| {
            let tx = self.tx.clone();
            StandardItem {
                label: label.to_string(),
                activate: Box::new(move |_| {
                    let _ = tx.unbounded_send(cmd.clone());
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


// ---- global hotkeys (X11) --------------------------------------------

#[cfg(target_os = "linux")]
mod hotkeys {
    use super::{Command, Config, UnboundedSender};
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

    /// The full keyboard mapping, fetched once per regrab: resolving
    /// each hotkey's keycode against a fresh reply was a round trip
    /// per key.
    struct Keymap {
        lo: u8,
        per: usize,
        keysyms: Vec<u32>,
    }
    impl Keymap {
        fn fetch(conn: &impl Connection) -> Option<Self> {
            let setup = conn.setup();
            let reply = conn
                .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)
                .ok()?
                .reply()
                .ok()?;
            Some(Self {
                lo: setup.min_keycode,
                per: reply.keysyms_per_keycode as usize,
                keysyms: reply.keysyms,
            })
        }
        /// First keycode producing `keysym` at any shift level.
        fn keycode_for(&self, keysym: u32) -> Option<u8> {
            for (i, chunk) in self.keysyms.chunks(self.per).enumerate() {
                if chunk.contains(&keysym) {
                    return Some(self.lo + i as u8);
                }
            }
            None
        }
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
    pub fn spawn(tx: UnboundedSender<Command>) {
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
            // (keycode, modifier mask, command): the mask matters when
            // two hotkeys share a key, e.g. "R" and "Ctrl+R".
            let mut grabbed: Vec<(u8, ModMask, Command)> = Vec::new();

            let grab_all = |conn: &x11rb::rust_connection::RustConnection,
                            grabbed: &mut Vec<(u8, ModMask, Command)>| {
                let keymap = Keymap::fetch(conn);
                let _ = conn.ungrab_key(0u8, root, ModMask::from(0x8000u16));
                grabbed.clear();
                let cfg = Config::load();
                for (hotkey, cmd) in [
                    (&cfg.capture_hotkey, Command::Capture),
                    (&cfg.record_hotkey, Command::RecordToggle),
                ] {
                    let Some((mods, keysym)) = parse_hotkey(hotkey) else {
                        iris_lib::ilog!("iris: cannot parse hotkey {hotkey:?}");
                        continue;
                    };
                    let Some(keycode) = keymap.as_ref().and_then(|k| k.keycode_for(keysym)) else {
                        iris_lib::ilog!("iris: no keycode for hotkey keysym {keysym:#x}");
                        continue;
                    };
                    // Issue all four lock-variant grabs before checking
                    // any: a serial check() per grab is a round trip
                    // each, four per hotkey per regrab.
                    let cookies: Vec<_> = locks
                        .iter()
                        .map(|extra| {
                            conn.grab_key(
                                false,
                                root,
                                mods | *extra,
                                keycode,
                                GrabMode::ASYNC,
                                GrabMode::ASYNC,
                            )
                        })
                        .collect();
                    let ok = cookies.into_iter().all(|c| {
                        c.map_err(|e| e.to_string())
                            .and_then(|cookie| cookie.check().map_err(|e| e.to_string()))
                            .is_ok()
                    });
                    if !ok {
                        iris_lib::ilog!("iris: cannot grab hotkey keysym {keysym:#x}");
                    }
                    if ok {
                        grabbed.push((keycode, mods, cmd));
                    }
                }
                let _ = conn.flush();
            };
            grab_all(&conn, &mut grabbed);

            // Event-driven, not polled: poll() the connection's fd so a
            // grabbed keypress wakes the thread the instant the X server
            // delivers it, instead of up to a sleep interval late. The
            // timeout still lets a regrab request land when no events
            // arrive.
            use std::os::unix::io::AsRawFd;
            let x_fd = conn.stream().as_raw_fd();
            loop {
                if regrab_rx.try_recv().is_ok() {
                    grab_all(&conn, &mut grabbed);
                }
                // Drain every queued event before sleeping again.
                loop {
                    match conn.poll_for_event() {
                        Ok(Some(x11rb::protocol::Event::KeyPress(ev))) => {
                            // Match keycode AND modifier mask: two
                            // hotkeys can share a key ("R" vs "Ctrl+R")
                            // and the grab fires per (key, mods) pair.
                            // Lock bits (NumLock M2, CapsLock LOCK) are
                            // masked out: they are grabbed as variants
                            // but are not part of the configured mask.
                            let ev_state: u16 = ev.state.into();
                            let ev_mods = ModMask::from(
                                ev_state & !(u16::from(ModMask::M2) | u16::from(ModMask::LOCK)),
                            );
                            if let Some((_, _, cmd)) = grabbed
                                .iter()
                                .find(|(kc, m, _)| *kc == ev.detail && *m == ev_mods)
                            {
                                if tx.unbounded_send(cmd.clone()).is_err() {
                                    return;
                                }
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) => break,
                        Err(_) => return,
                    }
                }
                // Sleep until the next X event or the regrab deadline.
                // 50ms is the regrab latency bound, not the hotkey one:
                // a keypress wakes poll() immediately.
                let mut pfd = libc::pollfd {
                    fd: x_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                unsafe {
                    libc::poll(&mut pfd, 1, 50);
                }
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
    let (tx, rx) = unbounded::<Command>();
    let _ = COMMAND_TX.set(tx.clone());

    match bind_socket() {
        Ok(listener) => {
            let listener = Arc::new(listener);
            cx.spawn(async move |cx| {
                // Event-driven: poll() the listener fd so a forwarded
                // CLI command dispatches the instant it connects, not
                // up to a timer interval late. accept_args drains every
                // pending connection before the next sleep.
                use std::os::unix::io::AsRawFd;
                let fd = listener.as_raw_fd();
                loop {
                    let listener = Arc::clone(&listener);
                    let args = cx
                        .background_executor()
                        .spawn(async move {
                            let mut pfd = libc::pollfd {
                                fd,
                                events: libc::POLLIN,
                                revents: 0,
                            };
                            unsafe {
                                libc::poll(&mut pfd, 1, -1);
                            }
                            accept_args(&listener)
                        })
                        .await;
                    if !args.is_empty() {
                        let cmds = parse_args(&args);
                        let _ = cx.update(|cx| {
                            for cmd in cmds {
                                if let Err(e) = dispatch(cx, &cmd) {
                                    iris_lib::ilog!("iris: dispatch: {e}");
                                }
                            }
                        });
                    }
                }
            })
            .detach();
        }
        Err(e) => iris_lib::ilog!("iris: single-instance socket unavailable: {e}"),
    }

    #[cfg(target_os = "linux")]
    {
        hotkeys::spawn(tx.clone());
        let tray = IrisTray { tx: tx.clone() };
        std::thread::spawn(move || {
            use ksni::blocking::TrayMethods;
            match tray.spawn() {
                Err(e) => iris_lib::ilog!("iris: tray unavailable: {e}"),
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
