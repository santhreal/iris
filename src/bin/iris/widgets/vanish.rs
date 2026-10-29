//! Closing windows before a command reads the screen. A window closed
//! in the same turn as a grab is still on the screen the grab reads, and
//! a compositor's close animation keeps it there longer. On a screen
//! that blends windows, each window first draws itself empty and closes
//! once that frame is on the screen; on one that does not, it closes at
//! once. The command runs once the last window is gone and the screen
//! has had `VANISH` to drop it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AnyWindowHandle, App, Window};

use crate::daemon;

/// After a window closes, how long the command waits for the compositor
/// to drop its last pixels from the screen.
const VANISH: Duration = Duration::from_millis(34);
/// The longest the close waits for the windows' empty frames: a window
/// that gets no frames (minimized, or fully covered) still closes and
/// the command still runs.
const VANISH_FALLBACK: Duration = Duration::from_millis(250);

/// Windows closing ahead of a command, and the command, which runs once.
struct Clear {
    open: Cell<usize>,
    cmd: Cell<Option<daemon::Command>>,
}

impl Clear {
    fn new(open: usize, cmd: daemon::Command) -> Rc<Clear> {
        Rc::new(Clear {
            open: Cell::new(open),
            cmd: Cell::new(Some(cmd)),
        })
    }

    /// One window left the screen; the last one runs the command.
    fn closed(&self, cx: &mut App) {
        let open = self.open.get().saturating_sub(1);
        self.open.set(open);
        if open == 0 {
            if let Some(cmd) = self.cmd.take() {
                run_after_vanish(cx, cmd);
            }
        }
    }
}

/// Close `window`, which the caller has set to draw empty from its next
/// frame, once it is off the screen, then run `cmd`.
pub fn close_then_run(window: &mut Window, cx: &mut App, cmd: daemon::Command) {
    let clear = Clear::new(1, cmd);
    let handle = window.window_handle();
    close(window, cx, clear.clone());
    fall_back(cx, vec![handle], clear);
}

/// Close `windows`, each set by its owner to draw empty from its next
/// frame, once each is off the screen, then run `cmd`. Called outside
/// any of their updates; a window already gone counts as closed.
pub fn clear_then_run(cx: &mut App, windows: Vec<AnyWindowHandle>, cmd: daemon::Command) {
    let clear = Clear::new(windows.len(), cmd);
    for handle in &windows {
        let mine = clear.clone();
        if handle
            .update(cx, |_, window, cx| close(window, cx, mine))
            .is_err()
        {
            clear.closed(cx);
        }
    }
    fall_back(cx, windows, clear);
}

/// Close `window` once it is off the screen: on a screen that blends it,
/// after its empty frame is presented (next-frame callbacks run before
/// that frame draws: the first before the empty frame, the second after
/// it presented); on one that does not, at once, since the X server
/// repaints what lay beneath before it reads the screen for a later
/// request.
fn close(window: &mut Window, cx: &mut App, clear: Rc<Clear>) {
    if !crate::sys::window::blended(window) {
        window.remove_window();
        clear.closed(cx);
        return;
    }
    window.refresh();
    window.on_next_frame(move |window, _| {
        window.on_next_frame(move |window, cx| {
            window.remove_window();
            clear.closed(cx);
        });
    });
}

/// After `VANISH_FALLBACK`, close what is still open of `windows` and
/// run the command, unless it ran.
fn fall_back(cx: &mut App, windows: Vec<AnyWindowHandle>, clear: Rc<Clear>) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(VANISH_FALLBACK).await;
        cx.update(|cx| {
            if let Some(cmd) = clear.cmd.take() {
                for handle in windows {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
                run_after_vanish(cx, cmd);
            }
        });
    })
    .detach();
}

/// Run `cmd` once the compositor has had `VANISH` to drop a window that
/// just closed.
fn run_after_vanish(cx: &mut App, cmd: daemon::Command) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(VANISH).await;
        cx.update(|cx| daemon::run(cx, &cmd));
    })
    .detach();
}
