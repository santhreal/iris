//! The window controls of a window that draws its own frame, in each
//! platform's register and place:
//!
//! - macOS: AppKit's own close, minimize, and zoom buttons at the
//!   leading edge (`open_window` places them at `MAC_TRAFFIC_LIGHTS`);
//!   the toolbar starts its content at `MAC_LEADING`, clear of them.
//! - Windows: minimize, maximize, and close caption buttons flush with
//!   the trailing edge. Each is the window's nonclient hit-test area of
//!   its kind, so the system runs it and shows Snap Layouts over the
//!   maximize button.
//! - Linux: round buttons on the sides the desktop's button layout sets
//!   (`gtk-decoration-layout`, read through the settings portal), or
//!   minimize, maximize, and close at the trailing edge when it sets
//!   none. A fixed-size window has no maximize button.

use gpui::*;

use super::OnClose;
use crate::icons::{icon, icon_in_text_color, Icon};
use crate::theme;

/// Which platform's controls a window draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Register {
    Mac,
    Windows,
    Linux,
}

pub const REGISTER: Register = if cfg!(target_os = "macos") {
    Register::Mac
} else if cfg!(windows) {
    Register::Windows
} else {
    Register::Linux
};

/// macOS: the close button's origin, logical px from the window's
/// top-left corner. The 16px tall buttons center in the toolbar.
#[cfg(target_os = "macos")]
pub const MAC_TRAFFIC_LIGHTS: (f32, f32) = (20.0, (theme::TOOLBAR_H - 16.0) / 2.0);
/// macOS: where the toolbar's leading content starts, right of the
/// traffic lights.
pub const MAC_LEADING: f32 = 88.0;
/// Linux: a round button's diameter, the space between two, and the
/// toolbar's padding outside them.
pub const ROUND: f32 = 24.0;
pub const ROUND_GAP: f32 = 8.0;
pub const ROUND_EDGE: f32 = 12.0;
/// Windows: a caption button's width; it spans the toolbar's height.
pub const CAPTION_W: f32 = 46.0;

type Side = [Option<WindowButton>; 3];
const EMPTY: Side = [None; 3];

/// The buttons the toolbar draws at its leading (`left`) and trailing
/// (`right`) edges. `layout` is the desktop's button layout on Linux;
/// macOS draws none, as AppKit draws its own.
pub fn sides(
    register: Register,
    layout: Option<WindowButtonLayout>,
    resizable: bool,
) -> WindowButtonLayout {
    use WindowButton::{Close, Maximize, Minimize};
    let trailing = [Some(Minimize), Some(Maximize), Some(Close)];
    match register {
        Register::Mac => WindowButtonLayout {
            left: EMPTY,
            right: EMPTY,
        },
        Register::Windows => WindowButtonLayout {
            left: EMPTY,
            right: trailing,
        },
        Register::Linux => {
            let layout = layout.unwrap_or(WindowButtonLayout {
                left: EMPTY,
                right: trailing,
            });
            let keep = |side: Side| {
                let mut out = EMPTY;
                let kept = side
                    .into_iter()
                    .flatten()
                    .filter(|&b| resizable || b != Maximize);
                for (slot, b) in out.iter_mut().zip(kept) {
                    *slot = Some(b);
                }
                out
            };
            WindowButtonLayout {
                left: keep(layout.left),
                right: keep(layout.right),
            }
        }
    }
}

/// The controls of one side, or `None` for a side with none.
pub fn side(
    buttons: Side,
    resizable: bool,
    on_close: &Option<OnClose>,
    window: &Window,
) -> Option<Div> {
    if buttons.iter().all(Option::is_none) {
        return None;
    }
    let maximized = window.is_maximized();
    let mut row = div().flex().flex_none().items_center();
    row = match REGISTER {
        Register::Windows => row.h_full(),
        _ => row.gap(px(ROUND_GAP)),
    };
    for button in buttons.into_iter().flatten() {
        row = row.child(match REGISTER {
            Register::Windows => caption(button, resizable, maximized, window.is_window_active()),
            _ => round(button, maximized, on_close.clone()),
        });
    }
    Some(row)
}

/// A Windows caption button. The system runs it: a press and release
/// on its hit-test area minimizes, maximizes or restores, or posts
/// `WM_CLOSE`, which runs the window's close route. A fixed-size
/// window's maximize button is dimmed and drags the window, as the
/// system's own caption does.
fn caption(button: WindowButton, resizable: bool, maximized: bool, active: bool) -> Stateful<Div> {
    let (id, area, glyph) = match button {
        WindowButton::Minimize => ("win-min", WindowControlArea::Min, Icon::CaptionMinimize),
        WindowButton::Maximize => (
            "win-max",
            WindowControlArea::Max,
            if maximized {
                Icon::CaptionRestore
            } else {
                Icon::CaptionMaximize
            },
        ),
        WindowButton::Close => ("win-close", WindowControlArea::Close, Icon::CaptionClose),
    };
    let enabled = resizable || button != WindowButton::Maximize;
    let ink = match (enabled, active) {
        (false, _) => theme::FG_DISABLED,
        (true, true) => theme::FG,
        (true, false) => theme::FG_DIM,
    };
    let el = div()
        .id(id)
        .w(px(CAPTION_W))
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(ink)
        .occlude()
        .window_control_area(area)
        .child(icon_in_text_color(glyph, 18.));
    match (enabled, button) {
        (false, _) => el,
        (true, WindowButton::Close) => el
            .hover(|s| s.bg(theme::CAPTION_CLOSE).text_color(theme::ACCENT_INK))
            .active(|s| {
                s.bg(theme::CAPTION_CLOSE_PRESS)
                    .text_color(theme::alpha(theme::ACCENT_INK, 0.7))
            }),
        (true, _) => el
            .hover(|s| s.bg(theme::CAPTION_HOVER))
            .active(|s| s.bg(theme::CAPTION_PRESS)),
    }
}

/// A Linux round button, as libadwaita draws a header bar's window
/// controls.
fn round(button: WindowButton, maximized: bool, on_close: Option<OnClose>) -> Stateful<Div> {
    let (id, glyph) = match button {
        WindowButton::Minimize => ("win-min", Icon::Minimize),
        WindowButton::Maximize => (
            "win-max",
            if maximized {
                Icon::Restore
            } else {
                Icon::Maximize
            },
        ),
        WindowButton::Close => ("win-close", Icon::Close),
    };
    let el = div()
        .id(id)
        .size(px(ROUND))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme::ROUND_FILL)
        .hover(|s| s.bg(theme::ROUND_HOVER))
        .active(|s| s.bg(theme::ROUND_PRESS))
        // A press on a window control must not start a title-bar drag.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(icon(glyph, theme::FG, 16.));
    match button {
        WindowButton::Minimize => el.on_click(|_, window, _| window.minimize_window()),
        WindowButton::Maximize => {
            el.on_click(|_, window, _| crate::sys::window::zoom_control(window))
        }
        WindowButton::Close => el.on_click(move |_, window, cx| match &on_close {
            Some(close) => close(window, cx),
            None => window.remove_window(),
        }),
    }
}

#[cfg(test)]
mod tests;
