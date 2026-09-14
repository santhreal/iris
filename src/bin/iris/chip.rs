//! Recording chip: the frosted badge pinned above the recorded window.
//!
//! A small transparent popup with the pulsing record dot, a tabular
//! elapsed timer, and the mic badge. The border thread in
//! record::x11 moves it by XID (x11rb configure_window), so it follows
//! the target window without going through the app event loop. The
//! chip's input shape is emptied, so clicks pass straight through.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use gpui::*;

use crate::theme;

const CHIP_W: f32 = 148.0;
const CHIP_H: f32 = 36.0;
/// Transparent margin around the pill so its shadow is not clipped
/// by the window bounds.
const CHIP_BLEED: f32 = 16.0;

/// The chip window's X11 id, read by the record thread's chip follower.
pub static CHIP_XID: AtomicU32 = AtomicU32::new(0);

/// The open chip window, so `close` can remove it without downcasting.
static CHIP_HANDLE: parking_lot::Mutex<Option<AnyWindowHandle>> =
    parking_lot::Mutex::new(None);

pub struct Chip {
    started: Instant,
    mic: bool,
}

/// Open the chip. Returns the X11 window id on X11, 0 elsewhere.
pub fn open(cx: &mut App, mic: bool) -> Result<u32, String> {
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
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
                    mic,
                })
            },
        )
        .map_err(|e| format!("open chip window: {e}"))?;

    let xid = find_chip_xid().unwrap_or(0);
    if let Ok((conn, _)) = x11rb::connect(None) {
        // Clicks pass through the chip to the window beneath it.
        use x11rb::protocol::shape::SK;
        use x11rb::protocol::xfixes::ConnectionExt;
        if xid != 0 {
            let _ = conn
                .xfixes_set_window_shape_region(xid, SK::INPUT, 0, 0, x11rb::NONE)
                .map(|c| c.check());
            crate::xwin::suppress_decorations_on(&conn, xid);
        }
    }
    // The XID lookup above can beat the map; strip decorations with
    // the persistent fixup: openbox decorates at map time and only
    // re-reads the hint when forced to re-frame the window.
    crate::xwin::place_after_map("dev.iris.chip".to_string(), 0.0, 0.0);
    CHIP_XID.store(xid, Ordering::SeqCst);
    *CHIP_HANDLE.lock() = Some(handle.into());
    Ok(xid)
}

/// Close the chip window, if open.
pub fn close(cx: &mut App) {
    CHIP_XID.store(0, Ordering::SeqCst);
    if let Some(handle) = CHIP_HANDLE.lock().take() {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

/// The chip window's X11 id: the one top-level window whose WM_CLASS
/// is dev.iris.chip.
pub fn find_chip_xid() -> Option<u32> {
    let (conn, _) = x11rb::connect(None).ok()?;
    find_chip_xid_on(&conn)
}

/// Same lookup on a caller-owned connection. Uses _NET_CLIENT_LIST:
/// WMs reparent client windows into frames, so the chip is not a
/// direct child of the root.
pub fn find_chip_xid_on(conn: &impl x11rb::connection::Connection) -> Option<u32> {
    crate::xwin::find_xid_by_class_on(conn, "iris.chip")
}

impl Render for Chip {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // rAF-driven: the dot pulse and timer both tick every frame.
        window.request_animation_frame();
        let t = self.started.elapsed().as_secs_f32();
        let pulse = 0.55 + 0.45 * (t * std::f32::consts::TAU / 1.6).sin().abs();
        let secs = t as u64;
        let timer = format!("{:02}:{:02}", secs / 60, secs % 60);

        let mut row = div()
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
                    .bg(theme::DANGER)
                    .opacity(pulse),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme::FG)
                    .font_family("monospace")
                    .child(timer),
            );
        if self.mic {
            row = row.child(crate::icons::icon(crate::icons::Icon::Mic, theme::FG_DIM, 14.0));
        }
        row
    }
}
