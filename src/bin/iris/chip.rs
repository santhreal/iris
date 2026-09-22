//! Recording chip: the frosted badge pinned above the recorded area.
//!
//! A small transparent popup with the pulsing record dot, a tabular
//! elapsed timer, and the mic badge. On X11 the border thread in
//! record::x11 moves it by XID (x11rb configure_window), so it follows
//! the target window without going through the app event loop, and the
//! chip's input shape is emptied, so clicks pass straight through. On
//! Windows and macOS it opens above the recorded region and is excluded
//! from screen capture, so it never appears in the recording.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use gpui::*;

use crate::theme;

const CHIP_W: f32 = 208.0;
const CHIP_H: f32 = 36.0;
/// Transparent margin around the pill so its shadow is not clipped
/// by the window bounds.
const CHIP_BLEED: f32 = 16.0;

/// The chip window's X11 id, read by the record thread's chip follower.
pub static CHIP_XID: AtomicU32 = AtomicU32::new(0);

/// The open chip window, so `close` can remove it without downcasting.
static CHIP_HANDLE: parking_lot::Mutex<Option<AnyWindowHandle>> = parking_lot::Mutex::new(None);

/// Live recording state the daemon flips; the chip re-renders every
/// frame so these read through without a notify round-trip.
static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static MIC_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Wall time spent paused, subtracted from the elapsed timer so a
/// pause does not inflate the recording's clock.
static PAUSED_MS: AtomicU32 = AtomicU32::new(0);
static PAUSE_STARTED: parking_lot::Mutex<Option<Instant>> = parking_lot::Mutex::new(None);

pub fn paused() -> bool {
    PAUSED.load(Ordering::Relaxed)
}
pub fn set_paused(v: bool) {
    let was = PAUSED.swap(v, Ordering::Relaxed);
    let mut started = PAUSE_STARTED.lock();
    match (was, v) {
        (false, true) => *started = Some(Instant::now()),
        (true, false) => {
            if let Some(t) = started.take() {
                PAUSED_MS.fetch_add(t.elapsed().as_millis() as u32, Ordering::Relaxed);
            }
        }
        _ => {}
    }
}
pub fn set_mic(v: bool) {
    MIC_ON.store(v, Ordering::Relaxed);
}

pub struct Chip {
    started: Instant,
    /// The 30Hz repaint timer is armed on first render.
    timer_started: bool,
    /// The last rendered timer text and its second: formatting
    /// "MM:SS" per repaint allocates a String 30x a second for a
    /// value that changes once a second.
    timer_at: u64,
    timer_text: SharedString,
}

/// Open the chip. `anchor` is the recorded rect in root pixels (`None`:
/// the main display); X11 ignores it, the chip follower places the
/// chip there. Returns the X11 window id on X11, 0 elsewhere.
pub fn open(
    cx: &mut App,
    mic: bool,
    anchor: Option<iris_lib::capture::WinRect>,
) -> Result<u32, String> {
    let origin = anchor_origin(cx, anchor);
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(origin.0), px(origin.1)),
                    size: size(px(CHIP_W + 2.0 * CHIP_BLEED), px(CHIP_H + 2.0 * CHIP_BLEED)),
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
                // Distinct WM_CLASS so the XID can be looked up on the
                // root tree (GPUI's X11 HasWindowHandle is unimplemented).
                app_id: Some("dev.iris.chip".to_string()),
                window_min_size: None,
                window_decorations: Some(WindowDecorations::Client),
                tabbing_identifier: None,
            },
            |_, cx| {
                cx.new(|_| Chip {
                    started: Instant::now(),
                    timer_started: false,
                    timer_at: u64::MAX,
                    timer_text: SharedString::from("00:00"),
                })
            },
        )
        .map_err(|e| format!("open chip window: {e}"))?;

    // X11: the chip is positioned by XID and undecorated via a motif
    // hint; other platforms honor the bounds GPUI requests at creation.
    #[cfg(target_os = "linux")]
    let xid = {
        let xid = find_chip_xid().unwrap_or(0);
        if let Ok((conn, _)) = iris_lib::capture::x11::shared_conn() {
            if xid != 0 {
                crate::sys::window::suppress_decorations_on(conn, xid);
            }
        }
        // The XID lookup above can beat the map; strip decorations with
        // the persistent fixup: openbox decorates at map time and only
        // re-reads the hint when forced to re-frame the window.
        crate::sys::window::place_after_map("dev.iris.chip".to_string(), 0.0, 0.0);
        xid
    };
    #[cfg(not(target_os = "linux"))]
    let xid = {
        let _ = handle.update(cx, |_, window, _| {
            crate::sys::window::exclude_from_capture(window)
        });
        0u32
    };
    CHIP_XID.store(xid, Ordering::SeqCst);
    MIC_ON.store(mic, Ordering::Relaxed);
    PAUSED.store(false, Ordering::Relaxed);
    PAUSED_MS.store(0, Ordering::Relaxed);
    *PAUSE_STARTED.lock() = None;
    *CHIP_HANDLE.lock() = Some(handle.into());
    Ok(xid)
}

/// Logical window origin that sets the pill above the top-right corner
/// of `anchor` (root pixels), kept on the display: inside the rect when
/// there is no room above it. `None` anchors to the main display.
fn anchor_origin(cx: &App, anchor: Option<iris_lib::capture::WinRect>) -> (f32, f32) {
    let s = iris_lib::capture::root_scale();
    let (x, y, w) = match anchor {
        Some(r) => (r.x as f32 / s, r.y as f32 / s, r.width as f32 / s),
        None => match crate::sys::window::primary_monitor_rect(cx) {
            Some((mx, my, mw, _)) => (mx, my + CHIP_H + 24.0, mw - 24.0),
            None => (0.0, 0.0, 0.0),
        },
    };
    let win_w = CHIP_W + 2.0 * CHIP_BLEED;
    let win_h = CHIP_H + 2.0 * CHIP_BLEED;
    let top = crate::sys::window::primary_monitor_rect(cx).map_or(0.0, |m| m.1);
    ((x + w - win_w).max(x), (y - win_h).max(top))
}

/// Close the chip window, if open.
pub fn close(cx: &mut App) {
    CHIP_XID.store(0, Ordering::SeqCst);
    if let Some(handle) = CHIP_HANDLE.lock().take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}
/// The chip window's X11 id: the one top-level window whose WM_CLASS
/// is dev.iris.chip. X11-only: other platforms position the chip by
/// the bounds GPUI requests, not by a server window id.
#[cfg(target_os = "linux")]
pub fn find_chip_xid() -> Option<u32> {
    let (conn, _) = iris_lib::capture::x11::shared_conn().ok()?;
    find_chip_xid_on(conn)
}

/// Same lookup on a caller-owned connection. Uses _NET_CLIENT_LIST:
/// WMs reparent client windows into frames, so the chip is not a
/// direct child of the root.
#[cfg(target_os = "linux")]
pub fn find_chip_xid_on(conn: &impl x11rb::connection::Connection) -> Option<u32> {
    crate::sys::window::find_xid_by_class_on(conn, "iris.chip")
}

impl Render for Chip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let paused = paused();
        // Repaint on a 30Hz timer, not vsync: the pulse is a 1.6s
        // sine, so 30fps reads as smoothly as 60 while halving the
        // compositor wakes a recording pays for the whole session.
        // Paused skips the notify, so a still pill costs no wakes.
        if !self.timer_started {
            self.timer_started = true;
            cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(33))
                    .await;
                let alive = this.update(cx, |_, cx| {
                    if !crate::chip::paused() {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            })
            .detach();
        }
        let mic_on = MIC_ON.load(Ordering::Relaxed);
        // Subtract wall time spent paused: the file has no frames for
        // that span, so the clock must not count it.
        let paused_ms = PAUSED_MS.load(Ordering::Relaxed) as u64
            + PAUSE_STARTED
                .lock()
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0);
        let t = self
            .started
            .elapsed()
            .as_millis()
            .saturating_sub(paused_ms as u128) as f32
            / 1000.0;
        let pulse = if paused {
            0.35
        } else {
            0.55 + 0.45 * (t * std::f32::consts::TAU / 1.6).sin().abs()
        };
        let secs = t as u64;
        if self.timer_at != secs {
            self.timer_at = secs;
            self.timer_text = SharedString::from(format!("{:02}:{:02}", secs / 60, secs % 60));
        }
        let timer = self.timer_text.clone();

        div()
            .absolute()
            .left(px(CHIP_BLEED))
            .top(px(CHIP_BLEED))
            .w(px(CHIP_W))
            .h(px(CHIP_H))
            .rounded_full()
            .font_family(theme::FONT)
            .bg(theme::alpha(theme::BG_ELEV, 0.92))
            .shadow(theme::shadow_float())
            .flex()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .child(
                div()
                    .w(px(9.))
                    .h(px(9.))
                    .rounded_full()
                    .bg(if paused {
                        theme::FG_FAINT
                    } else {
                        theme::DANGER
                    })
                    .opacity(pulse),
            )
            .child(
                div()
                    .text_size(px(theme::TEXT_BODY))
                    .text_color(theme::FG)
                    .font_family(theme::FONT)
                    .child(timer),
            )
            .child(
                div()
                    .id("chip-pause")
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| {
                        let _ = crate::daemon::dispatch(cx, &crate::daemon::Command::RecordPause);
                    }))
                    .child(crate::icons::icon(
                        if paused {
                            crate::icons::Icon::Play
                        } else {
                            crate::icons::Icon::Pause
                        },
                        theme::FG_DIM,
                        14.0,
                    )),
            )
            .child(
                div()
                    .id("chip-mic")
                    .cursor_pointer()
                    .on_click(cx.listener(|_, _, _, cx| {
                        let _ = crate::daemon::dispatch(cx, &crate::daemon::Command::RecordMic);
                    }))
                    .child(crate::icons::icon(
                        if mic_on {
                            crate::icons::Icon::Mic
                        } else {
                            crate::icons::Icon::MicOff
                        },
                        theme::FG_DIM,
                        14.0,
                    )),
            )
    }
}
