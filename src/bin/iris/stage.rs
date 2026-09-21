//! Toast stage: the macOS floating screenshot thumbnail.
//!
//! The particulars this matches: raw capture pixels only, aspect-true,
//! no chrome of any kind (no border stroke, no inner highlight, no
//! buttons, no labels, no hover or pressed state). ~200px long edge,
//! 9px corners, one soft deep shadow. It slides in from beyond the
//! right edge in ~450ms with a hard deceleration and stops dead: no
//! overshoot, no fade, no scale. It sits visually inert for ~5s;
//! hovering only suspends the clock; then accelerates back off the
//! right edge, fading in the last third. A rightward flick dismisses
//! it 1:1 under the pointer; any other drag is a file drag through
//! XDnD. Click morphs into Markup. A new capture replaces it.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use gpui::*;

use crate::{motion, theme};

mod actions;
mod window;

#[cfg(test)]
mod tests;

pub(super) use actions::card_shadow;
pub(super) use actions::prepare_thumb;
pub use window::{card_rest_rect, show_toast, show_toast_landed};

pub(super) const MAX_W: f32 = 200.0;
pub(super) const MAX_H: f32 = 140.0;
pub(super) const MARGIN: f32 = 12.0;
pub(super) const BLEED: f32 = 44.0;
pub(super) const RADIUS: f32 = 12.0;
pub(super) const ENTER: Duration = motion::ENTER;
pub(super) const EXIT: Duration = Duration::from_millis(300);
/// Replaced by a newer capture: a fast fade, not the full fly-off.
pub(super) const REPLACE_EXIT: Duration = Duration::from_millis(180);
pub(super) const SWIPE_RETURN: Duration = Duration::from_millis(260);
pub(super) const MENU_FADE: Duration = motion::FADE;
/// Safety net for the editor-morph handshake: if the editor never
/// renders its first frame, the toast gives up waiting and closes.
pub(super) const MORPH_WAIT: Duration = Duration::from_millis(2000);
/// How long the toast lingers after the editor's first render: a
/// rendered frame still has map and compositor latency ahead of it.
/// While it lingers, the editor is still transparent and the toast's
/// pixels show through as the morph's opening frame.
pub(super) const PRESENT_GRACE: Duration = Duration::from_millis(60);
/// A velocity sample older than this at release is a stopped finger,
/// not a flick.
pub(super) const SWIPE_STALE: Duration = Duration::from_millis(120);
/// Past this travel, or released faster than this velocity, a
/// rightward swipe dismisses instead of springing back.
pub(super) const SWIPE_TRAVEL: f32 = 100.0;
pub(super) const SWIPE_VELOCITY: f32 = 400.0;

pub struct ToastStage {
    pub(super) path: PathBuf,
    pub(super) thumb: Arc<RenderImage>,
    pub(super) thumb_rgba: Arc<Vec<u8>>,
    pub(super) dims: (f32, f32),
    pub(super) card_screen: (f32, f32, f32, f32),
    pub(super) opened: Option<Instant>,
    pub(super) hover_paused: bool,
    pub(super) pinned: bool,
    /// Dismiss-arm generation: each arm_dismiss bumps it, and a timer
    /// task that wakes on an older generation is a no-op. Without it
    /// every hover-out spawned a live timer and the earliest one won,
    /// dismissing the toast ahead of the last hover's deadline.
    pub(super) dismiss_gen: u64,
    pub(super) closing_at: Option<Instant>,
    /// Rightward offset the exit flight starts from, when a swipe
    /// carried the card before the dismiss.
    pub(super) exit_from: f32,
    pub(super) drag_start: Option<(f32, f32)>,
    pub(super) gesture: Gesture,
    pub(super) swipe: Option<Swipe>,
    pub(super) swipe_return: Option<(Instant, f32)>,
    /// Right-click context menu position in window coordinates.
    pub(super) menu_at: Option<(f32, f32)>,
    /// Menu entrance clock; the menu fades in and rises 3px.
    pub(super) menu_opened: Option<Instant>,
    /// A left press that only dismissed the open menu is not a click.
    pub(super) swallow_click: bool,
    /// Editor-morph handshake: the toast stays put until the editor
    /// has painted the image over the exact same pixels, so the
    /// handoff has no blank blink. The instant bounds the wait.
    pub(super) pending_morph: Option<(Arc<AtomicBool>, Instant)>,
    /// When the editor's first render was seen; the toast lingers
    /// PRESENT_GRACE past it because a rendered frame is not yet a
    /// presented one: X11 map and compositor latency would otherwise
    /// open a black gap between the two windows.
    pub(super) morph_ready_at: Option<Instant>,
    /// Exit duration: full fly-off normally, a fast fade on replace.
    pub(super) closing_dur: Duration,
    /// The config snapshot taken at construction: render reads it
    /// several times a frame, and Config::load() hits the disk each
    /// call.
    pub(super) cfg: iris_lib::config::Config,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Gesture {
    Undecided,
    Swipe,
    FileDrag,
}

/// A rightward dismiss swipe in progress: 1:1 travel under the
/// pointer plus a smoothed velocity for the release decision.
pub(super) struct Swipe {
    pub(super) dx: f32,
    pub(super) vel: f32,
    pub(super) last_at: Instant,
    pub(super) last_dx: f32,
}

impl Render for ToastStage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = self.dims;
        let cfg = &self.cfg;
        let is_left = matches!(
            cfg.toast_position,
            iris_lib::config::ToastPosition::BottomLeft | iris_lib::config::ToastPosition::TopLeft
        );
        let is_top = matches!(
            cfg.toast_position,
            iris_lib::config::ToastPosition::TopLeft | iris_lib::config::ToastPosition::TopRight
        );

        // Entrance: decelerating hard into the corner.
        let opened = *self.opened.get_or_insert_with(Instant::now);
        let enter_t =
            (opened.elapsed().as_secs_f32() / motion::tempo(ENTER).as_secs_f32()).min(1.0);
        let ease = 1.0 - (1.0 - enter_t).powi(4);
        let mut offset = MARGIN - (1.0 - ease) * (w + 2.0 * MARGIN);
        let mut opacity = 1.0f32;
        let shadow_vis = ease;
        let mut animating = enter_t < 1.0;

        // Dismiss swipe: the card tracks the pointer toward the screen edge.
        if let Some(s) = &self.swipe {
            offset -= s.dx;
        }
        // A short swipe released: spring back to the corner.
        if let Some((started, dx0)) = self.swipe_return {
            let t = (started.elapsed().as_secs_f32() / motion::tempo(SWIPE_RETURN).as_secs_f32())
                .min(1.0);
            offset -= dx0 * (1.0 - motion::spring(t));
            if t >= 1.0 {
                self.swipe_return = None;
            } else {
                animating = true;
            }
        }

        // Exit: accelerate off the edge from wherever the card sits.
        if let Some(started) = self.closing_at {
            let t = (started.elapsed().as_secs_f32()
                / motion::tempo(self.closing_dur).as_secs_f32())
            .min(1.0);
            if self.closing_dur > REPLACE_EXIT {
                offset = MARGIN - self.exit_from - (w + 2.0 * MARGIN + 24.0) * t * t;
                opacity = 1.0 - ((t - 0.66) / 0.34).clamp(0.0, 1.0);
            } else {
                opacity = 1.0 - t;
            }
            if t >= 1.0 {
                let thumb = self.thumb.clone();
                cx.defer(move |cx| crate::widgets::release_render(&thumb, cx));
                window.remove_window();
            } else {
                animating = true;
            }
        }

        // Editor-morph handshake.
        if let Some((ready, since)) = &self.pending_morph {
            offset = MARGIN;
            opacity = 1.0;
            if ready.load(Ordering::Acquire) && self.morph_ready_at.is_none() {
                self.morph_ready_at = Some(Instant::now());
            }
            let grace_done = self
                .morph_ready_at
                .is_some_and(|at| at.elapsed() > PRESENT_GRACE);
            if grace_done || since.elapsed() > MORPH_WAIT {
                let thumb = self.thumb.clone();
                cx.defer(move |cx| crate::widgets::release_render(&thumb, cx));
                window.remove_window();
            } else {
                window.request_animation_frame();
            }
        }
        if animating {
            window.request_animation_frame();
        }

        let thumb = self.thumb.clone();
        let menu_el = self.render_menu(opacity, window, cx);

        div()
            .size_full()
            .font_family(theme::FONT)
            // A press outside the menu dismisses it; that press is
            // not also a click on whatever lies beneath.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|stage, ev: &MouseDownEvent, _, cx| {
                    stage.swallow_click = false;
                    if let Some((mx, my)) = stage.menu_at {
                        let (px_, py_): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                        let menu_h = 6.0 * crate::widgets::MENU_ROW_H + 10.0;
                        let inside = px_ >= mx
                            && px_ <= mx + crate::widgets::MENU_W
                            && py_ >= my
                            && py_ <= my + menu_h;
                        if !inside {
                            stage.menu_at = None;
                            stage.menu_opened = None;
                            stage.swallow_click = true;
                            cx.notify();
                        }
                    }
                }),
            )
            // Gesture tracking lives on the root, not the card:
            // move/up events only reach the element under the cursor,
            // and a committed drag or flick leaves the card within a
            // few pixels. The window's bleed margin keeps the gesture
            // alive until the XDnD engine's own pointer polling (or
            // the swipe release) takes over.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|stage, _, _, cx| {
                    if stage.gesture == Gesture::Swipe {
                        stage.release_swipe(cx);
                    }
                    stage.drag_start = None;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|stage, ev: &MouseMoveEvent, _, cx| {
                if ev.pressed_button != Some(MouseButton::Left)
                    || stage.gesture == Gesture::FileDrag
                {
                    return;
                }
                let Some((sx, sy)) = stage.drag_start else {
                    return;
                };
                let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                let (dx, dy) = (mx - sx, my - sy);
                let is_left = matches!(
                    stage.cfg.toast_position,
                    iris_lib::config::ToastPosition::BottomLeft
                        | iris_lib::config::ToastPosition::TopLeft
                );
                let dismiss_dx = if is_left { -dx } else { dx };
                if stage.gesture == Gesture::Swipe {
                    let now = Instant::now();
                    if let Some(s) = &mut stage.swipe {
                        let dt = now.duration_since(s.last_at).as_secs_f32().max(0.001);
                        let v = (dismiss_dx - s.last_dx) / dt;
                        s.vel = 0.65 * s.vel + 0.35 * v;
                        s.last_at = now;
                        s.last_dx = dismiss_dx;
                        s.dx = dismiss_dx.max(0.0);
                    }
                    cx.notify();
                    return;
                }
                if dx.hypot(dy) < 8.0 {
                    return;
                }
                if dismiss_dx > 6.0 && dismiss_dx.abs() > 2.0 * dy.abs() {
                    // Dominantly toward screen edge: a dismiss swipe.
                    stage.gesture = Gesture::Swipe;
                    stage.swipe = Some(Swipe {
                        dx: dismiss_dx.max(0.0),
                        vel: 0.0,
                        last_at: Instant::now(),
                        last_dx: dismiss_dx,
                    });
                } else if stage.cfg.toast_drag_enabled {
                    stage.gesture = Gesture::FileDrag;
                    let icon = iris_lib::dragcopy::DragIcon {
                        width: stage.dims.0 as u32,
                        height: stage.dims.1 as u32,
                        // The stage already holds the pixels behind
                        // an Arc: a refcount, not a multi-MB clone.
                        rgba: stage.thumb_rgba.clone(),
                    };
                    if let Err(e) = iris_lib::dragcopy::start_file_drag_at_cursor(
                        vec![stage.path.clone()],
                        Some(icon),
                    ) {
                        iris_lib::ilog!("drag: {e}");
                    }
                }
                cx.notify();
            }))
            .child({
                let mut card = div()
                    .id("card")
                    .absolute()
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(RADIUS))
                    .overflow_hidden()
                    .shadow(card_shadow(shadow_vis))
                    .opacity(opacity);

                if is_left {
                    card = card.left(px(offset));
                } else {
                    card = card.right(px(offset));
                }
                if is_top {
                    card = card.top(px(MARGIN));
                } else {
                    card = card.bottom(px(MARGIN));
                }

                card = card.child(
                    div().absolute().top_0().left_0().size_full().child(
                        img(ImageSource::Render(thumb))
                            .size_full()
                            .object_fit(ObjectFit::Fill)
                            .rounded(px(RADIUS)),
                    ),
                );

                if let Some(actions) = self.render_action_bar(cx) {
                    card = card.child(actions);
                }

                card.on_hover(cx.listener(|stage, hovering, _window, cx| {
                    stage.hover_paused = *hovering;
                    if !*hovering {
                        stage.arm_dismiss(cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|stage, ev: &MouseDownEvent, _, cx| {
                        stage.drag_start = Some((ev.position.x.into(), ev.position.y.into()));
                        stage.gesture = Gesture::Undecided;
                        stage.swipe_return = None;
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|stage, ev: &MouseDownEvent, _, cx| {
                        let menu_h = 6.0 * crate::widgets::MENU_ROW_H + 10.0;
                        let win_w = stage.dims.0 + BLEED + MARGIN;
                        let win_h = stage.dims.1 + BLEED + MARGIN;
                        let mx: f32 = ev.position.x.into();
                        let my: f32 = ev.position.y.into();
                        stage.menu_at = Some((
                            mx.min(win_w - crate::widgets::MENU_W - 2.0).max(2.0),
                            my.min(win_h - menu_h - 2.0).max(2.0),
                        ));
                        stage.menu_opened = None;
                        cx.notify();
                    }),
                )
                .on_click(cx.listener(
                    |stage, _, window, cx| match stage.gesture {
                        Gesture::Swipe => {
                            stage.release_swipe(cx);
                            stage.gesture = Gesture::Undecided;
                        }
                        Gesture::FileDrag => {
                            stage.gesture = Gesture::Undecided;
                        }
                        Gesture::Undecided => {
                            if stage.swallow_click {
                                stage.swallow_click = false;
                                return;
                            }
                            stage.perform_click_action(window, cx);
                        }
                    },
                ))
            })
            .children(menu_el)
    }
}
