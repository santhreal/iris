//! Window moves from a handle: a title bar, a pinned image. A left press
//! on the handle arms the move, and the move begins once the held
//! pointer is more than `SLOP` from the press, keeping the press point
//! under the pointer. A click never moves the window.
//!
//! A double click runs the handle's `Double` action. The platform counts
//! clicks by window-local position, and a moving window carries the
//! press point with it: the press after a quick drag lands where the
//! drag began and is counted as the drag's second click. A press that
//! began a move therefore ends its click sequence, and the next press
//! counts as a first.

use std::cell::Cell;
use std::rc::Rc;

use gpui::*;

#[cfg(test)]
mod tests;

/// Pointer travel from the press, logical px, past which a held press
/// moves the window.
const SLOP: f32 = 4.0;

/// What a double click on a handle runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Double {
    /// Nothing: every press may move the window.
    Nothing,
    /// The platform's title bar action (`sys::window::title_double_click`).
    TitleBar,
    /// Close the window.
    Close,
}

/// A handle's press state, kept across frames in element state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Press {
    /// The held press that has not moved the window: where it landed,
    /// window-local, and the platform's click count for it.
    armed: Option<(Point<Pixels>, usize)>,
    /// The platform's click count for the press that began the last
    /// move. Later presses of the same click sequence count from it.
    moved_at: usize,
}

impl Press {
    /// A left press at `at`, the platform's `clicks`-th of its sequence.
    /// True for a double click, which runs `double` and arms no move.
    fn down(&mut self, at: Point<Pixels>, clicks: usize, double: Double) -> bool {
        if clicks <= 1 {
            self.moved_at = 0;
        }
        let is_double = double != Double::Nothing && clicks.saturating_sub(self.moved_at) == 2;
        self.armed = (!is_double).then_some((at, clicks));
        is_double
    }

    /// The pointer at `at`, the left button `held`. The press point once
    /// the pointer is past `SLOP` from it: the move begins from there,
    /// once per press.
    fn motion(&mut self, at: Point<Pixels>, held: bool) -> Option<Point<Pixels>> {
        let (from, clicks) = self.armed?;
        if !held {
            self.armed = None;
            return None;
        }
        let d = at - from;
        if f32::from(d.x).hypot(f32::from(d.y)) <= SLOP {
            return None;
        }
        self.armed = None;
        self.moved_at = clicks;
        Some(from)
    }

    /// The left button came up.
    fn up(&mut self) {
        self.armed = None;
    }
}

/// A handle over its parent's bounds that moves its window. Add it as
/// the parent's first child: controls drawn over it take their own
/// presses, and a control that stops a press keeps it from the handle.
pub fn move_handle(double: Double) -> impl IntoElement {
    canvas(
        |bounds, window, _| {
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            let press = window.with_global_id("move-handle".into(), |id, window| {
                window.with_element_state(id, |press: Option<Rc<Cell<Press>>>, _| {
                    let press = press.unwrap_or_default();
                    (press.clone(), press)
                })
            });
            (hitbox, press)
        },
        move |_, (hitbox, press), window, _| {
            let pressed = press.clone();
            window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, _| {
                if phase != DispatchPhase::Bubble
                    || ev.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let mut p = pressed.get();
                let is_double = p.down(ev.position, ev.click_count, double);
                pressed.set(p);
                if is_double {
                    match double {
                        Double::Nothing => {}
                        Double::TitleBar => crate::sys::window::title_double_click(window),
                        Double::Close => window.remove_window(),
                    }
                }
            });
            // Capture phase: the pointer may leave the handle, or the
            // window, before it is past SLOP, and no element may stop
            // the move or the release from reaching the handle.
            let moved = press.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, _| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                let mut p = moved.get();
                let from = p.motion(ev.position, ev.pressed_button == Some(MouseButton::Left));
                moved.set(p);
                if let Some(from) = from {
                    crate::sys::window::begin_wm_move(window, from);
                }
            });
            window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, _| {
                if phase == DispatchPhase::Capture && ev.button == MouseButton::Left {
                    let mut p = press.get();
                    p.up();
                    press.set(p);
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}
