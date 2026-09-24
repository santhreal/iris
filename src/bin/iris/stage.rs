//! Toast stage: the floating screenshot thumbnail.
//!
//! Raw capture pixels, aspect-true, at most 200x140 logical px and
//! scaled to the display's device pixels, 12px corners,
//! one soft deep shadow. It slides in from beyond the screen edge in
//! 380ms with a hard deceleration and stops dead, sits for the
//! configured duration (hovering suspends the clock), then accelerates
//! back off the edge, fading in the last third. A flick toward the edge
//! dismisses it 1:1 under the pointer; any other drag is a file
//! drag-out. A click runs the configured click action; the default,
//! Markup, morphs the card into the editor. The optional action bar
//! and the right-click menu act on the capture, and an action's result
//! replaces the bar with a status line. A new capture replaces it.

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
mod thumb;
mod window;

#[cfg(test)]
mod tests;

use theme::card_shadow;
pub(super) use thumb::{prepare_thumb, Thumb};
use window::publish_rest;
pub use window::{card_rest_rect, live_card_rect, show_toast, show_toast_landed};

pub(super) const MAX_W: f32 = 200.0;
pub(super) const MAX_H: f32 = 140.0;
pub(super) const MARGIN: f32 = 12.0;
pub(super) const BLEED: f32 = theme::CARD_BLEED;
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

/// The card's logical size for a capture of `img_w`x`img_h` pixels:
/// aspect-true within MAX_W x MAX_H, never enlarged, whole pixels. The
/// flight's landing rect and the toast's card both come from here, so
/// the handoff between them does not move by a pixel.
pub(super) fn card_size(img_w: f32, img_h: f32) -> (f32, f32) {
    let fit = (MAX_W / img_w).min(MAX_H / img_h).min(1.0);
    (
        (img_w * fit).round().max(1.0),
        (img_h * fit).round().max(1.0),
    )
}

pub struct ToastStage {
    pub(super) path: PathBuf,
    pub(super) thumb: Thumb,
    /// A re-scale of `thumb` is in flight: render does not queue another.
    pub(super) rescaling: bool,
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
    /// The result of the last toast action (copy, OCR, delete, pin),
    /// drawn over the card's lower edge in place of the action bar.
    pub(super) status: Option<SharedString>,
    /// OCR in flight: the dismiss timer re-arms instead of closing, so
    /// the result has a card to land on.
    pub(super) busy: bool,
    /// The action bar's visibility: a landed toast opens without it.
    pub(super) bar: Bar,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Gesture {
    Undecided,
    Swipe,
    FileDrag,
}

/// The action bar's visibility. A landed toast opens with the bar
/// hidden, so its first frame matches the flight card it replaces, and
/// fades the bar in once the overlay is gone.
#[derive(Clone, Copy)]
pub(super) enum Bar {
    Shown,
    Hidden,
    FadingIn(Instant),
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
        let (w, h) = self.thumb.dims;
        // The pixels were sized for the scale the toast was expected to
        // open at; a window that renders at another one re-sizes them.
        let scale = window.scale_factor();
        if (scale - self.thumb.scale).abs() > 1e-3 && !self.rescaling {
            self.rescale_thumb(scale, cx);
        }
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
        let bar = match self.bar {
            Bar::Shown => 1.0,
            Bar::Hidden => 0.0,
            Bar::FadingIn(from) => {
                let t = (from.elapsed().as_secs_f32() / motion::tempo(motion::FADE).as_secs_f32())
                    .min(1.0);
                animating |= t < 1.0;
                motion::ease_out(t)
            }
        };

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
                crate::widgets::release_render(&self.thumb.render, cx);
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
                crate::widgets::release_render(&self.thumb.render, cx);
                window.remove_window();
            } else {
                window.request_animation_frame();
            }
        }
        if animating {
            window.request_animation_frame();
        }
        let resting = self.closing_at.is_none() && self.pending_morph.is_none();
        publish_rest(window.window_handle(), resting.then_some(self.card_screen));

        let thumb = self.thumb.render.clone();
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
            .on_mouse_move(cx.listener(|stage, ev: &MouseMoveEvent, window, cx| {
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
                        width: stage.thumb.px.0,
                        height: stage.thumb.px.1,
                        // The stage already holds the pixels behind
                        // an Arc: a refcount, not a multi-MB clone.
                        rgba: stage.thumb.rgba.clone(),
                    };
                    if let Err(e) = crate::sys::window::start_file_drag(
                        window,
                        vec![stage.path.clone()],
                        Some(icon),
                    ) {
                        iris_lib::ilog!("drag: {e}");
                        stage.show_status(format!("Drag failed: {e}"), cx);
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

                if let Some(status) = &self.status {
                    card = card.child(actions::status_band(status.clone()));
                } else if bar > 0.0 {
                    if let Some(actions) = self.render_action_bar(cx) {
                        card = card.child(actions.opacity(bar));
                    }
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
                        let win_w = stage.thumb.dims.0 + BLEED + MARGIN;
                        let win_h = stage.thumb.dims.1 + BLEED + MARGIN;
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
