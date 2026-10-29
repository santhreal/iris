//! Closing a window before a command reads the screen. A window closed
//! in the same turn as a grab is still on the screen the grab reads, and
//! a window manager's close animation keeps it there longer. The window
//! first draws itself empty, closes once that frame is on the screen,
//! and the command runs once the compositor has dropped it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Window};

use crate::daemon;

/// After a window closes, how long the command waits for the compositor
/// to drop its last pixels from the screen.
const VANISH: Duration = Duration::from_millis(34);
/// The longest the close waits for the window's empty frame: a window
/// that gets no frames (minimized, or fully covered) still closes and
/// runs the command.
const VANISH_FALLBACK: Duration = Duration::from_millis(250);

/// Close `window`, which the caller has set to draw empty from its next
/// frame, once that frame is on the screen, then run `cmd`.
pub fn close_then_run(window: &mut Window, cx: &mut App, cmd: daemon::Command) {
    window.refresh();
    let pending = Rc::new(Cell::new(Some(cmd)));
    // Next-frame callbacks run before that frame draws: the first runs
    // before the empty frame, the second after it presented.
    let on_frame = pending.clone();
    window.on_next_frame(move |window, _| {
        window.on_next_frame(move |window, cx| {
            if let Some(cmd) = on_frame.take() {
                window.remove_window();
                run_after_vanish(cx, cmd);
            }
        });
    });
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(VANISH_FALLBACK).await;
        cx.update(|cx| {
            if let Some(cmd) = pending.take() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                run_after_vanish(cx, cmd);
            }
        });
    })
    .detach();
}

/// Run `cmd` once the compositor has had `VANISH` to drop a window that
/// just closed.
pub fn run_after_vanish(cx: &mut App, cmd: daemon::Command) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(VANISH).await;
        cx.update(|cx| daemon::run(cx, &cmd));
    })
    .detach();
}
