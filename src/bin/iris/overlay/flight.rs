//! The capture flight: the committed region springs from the selection
//! rect to the toast's resting corner rect over the live desktop, then
//! hands off to the toast.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::*;

use super::Overlay;
use crate::{stage, theme};

#[derive(Clone)]
pub(super) struct Flight {
    pub(super) img: Arc<RenderImage>,
    pub(super) from: (f32, f32, f32, f32),
    /// The toast card's resting rect.
    pub(super) to: (f32, f32, f32, f32),
    /// The toast card, which holds the capture's rect on it: the
    /// capture flies to that rect, the card's fill grows around it.
    pub(super) card: stage::Card,
    pub(super) started: Instant,
    /// The committing window's screen rect: the toast lands on this
    /// monitor, not wherever the primary display happens to be.
    pub(super) screen: (f32, f32, f32, f32),
    /// The selection's screen rect, which a pin of the capture opens
    /// over.
    pub(super) source: (f32, f32, f32, f32),
    /// Whether the flight has opened its toast: the session is over,
    /// and the window stays only for the shadow's turn.
    pub(super) handed: bool,
    /// When the toast's first frame was on screen: from then the card's
    /// shadow passes to the toast's (`motion::turnover`).
    pub(super) turned: Option<Instant>,
}

/// The flight card's rect at spring progress `e`.
pub(super) fn flight_rect(
    from: (f32, f32, f32, f32),
    to: (f32, f32, f32, f32),
    e: f32,
) -> (f32, f32, f32, f32) {
    (
        from.0 + (to.0 - from.0) * e,
        from.1 + (to.1 - from.1) * e,
        from.2 + (to.2 - from.2) * e,
        from.3 + (to.3 - from.3) * e,
    )
}

/// True once every edge of the card is within half a logical pixel of
/// its resting rect. The spring's tail past that point moves nothing
/// on screen, so the flight is over there, not at the end of its clock.
pub(super) fn flight_at_rest(from: (f32, f32, f32, f32), to: (f32, f32, f32, f32), e: f32) -> bool {
    let travel = [
        to.0 - from.0,
        to.1 - from.1,
        (to.0 + to.2) - (from.0 + from.2),
        (to.1 + to.3) - (from.1 + from.3),
    ];
    travel.iter().all(|d| (d * (1.0 - e)).abs() < 0.5)
}

impl Overlay {
    /// The capture flight: the committed region springs from the
    /// selection rect to the toast's corner rect over the live desktop.
    /// Once the card is at rest and the background finalize has landed,
    /// the toast opens underneath; the overlay keeps the card on screen
    /// until the toast has presented its first frame, then parks.
    pub(super) fn render_flight(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        const FLIGHT: Duration = Duration::from_millis(280);
        // Under reduced motion the card does not fly: it fades in on
        // its resting rect, over the shared fade.
        let reduced = cx.reduce_motion();
        let f = self.flight.clone().expect("flight checked");
        let flown = f.started.elapsed().as_secs_f32();
        let over = if reduced { crate::motion::FADE } else { FLIGHT };
        let t = (flown / crate::motion::tempo(over).as_secs_f32()).min(1.0);
        let (e, opacity) = if reduced {
            (1.0, crate::motion::ease_out(t))
        } else {
            (crate::motion::spring(t), 1.0)
        };
        // The clock's end bounds the flight whatever the curve does.
        let image_to = f.card.image_at(f.to.0, f.to.1);
        let at_rest = t >= 1.0
            || (!reduced && flight_at_rest(f.from, f.to, e) && flight_at_rest(f.from, image_to, e));
        if at_rest {
            // Taken once: later frames of this session never reopen it.
            // Until then a slow finalize's notify re-renders this.
            if let Some(landed) = self.landed.take() {
                self.hand_off(landed, &f, window, cx);
            }
        }
        let (cast, _) = crate::motion::turnover(f.turned, Instant::now());
        if !at_rest || (f.turned.is_some() && cast > 0.0) {
            window.request_animation_frame();
        }
        // The card lifts off the page: square and flat where it leaves
        // the selection, and at rest the toast's card to the pixel, its
        // rect, corners, fill, and shadow, so the handoff changes
        // nothing. The capture and the card leave from the selection
        // together; where the card is wider or taller than the capture,
        // its fill grows out around the capture on the way.
        let (e, (x, y, w, h), (ix, iy, iw, ih)) = if at_rest {
            (1.0, f.to, image_to)
        } else {
            (
                e,
                flight_rect(f.from, f.to, e),
                flight_rect(f.from, image_to, e),
            )
        };
        // The card is built as the toast's is: the capture and the
        // outline sit under the card's rounded clip, so at rest the two
        // draw the same corner pixels.
        let mut card = div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w.max(1.0)))
            .h(px(h.max(1.0)))
            .rounded(px(stage::RADIUS * e))
            .overflow_hidden()
            .shadow(theme::card_shadow(e * cast));
        if f.card.letterboxed() {
            card = card.bg(theme::alpha(theme::BG_ELEV, e.clamp(0.0, 1.0)));
        }
        // At rest the capture sits where the toast places it on its card.
        let (ox, oy) = if at_rest {
            let (ox, oy, _, _) = f.card.image_at(0.0, 0.0);
            (ox, oy)
        } else {
            (ix - x, iy - y)
        };
        let card = card
            .child(
                img(ImageSource::Render(f.img.clone()))
                    .absolute()
                    .left(px(ox))
                    .top(px(oy))
                    .w(px(iw.max(1.0)))
                    .h(px(ih.max(1.0)))
                    .object_fit(ObjectFit::Fill)
                    .rounded(px(f.card.image_radius() * e)),
            )
            .child(stage::edge(stage::RADIUS * e, e).inset_0());
        div().size_full().opacity(opacity).child(card)
    }

    /// Open the landed toast. Once the toast is on screen the card's
    /// shadow passes to it, and the window parks when the turn ends; with
    /// no toast, or where no compositor blends, it parks at once. Each
    /// step checks the flight's start instant: a session re-armed in
    /// between owns the window and is left alone.
    fn hand_off(
        &mut self,
        (path, thumb): (PathBuf, Result<stage::Thumb, String>),
        f: &Flight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(overlay) = window.window_handle().downcast::<Overlay>() else {
            self.park(window, cx);
            return;
        };
        if let Some(flight) = self.flight.as_mut() {
            flight.handed = true;
        }
        let (started, screen, source) = (f.started, f.screen, f.source);
        let park = Box::new(move |cx: &mut App, turned: Option<Instant>| {
            let turning = overlay.update(cx, |this, window, cx| {
                let Some(f) = this.flight.as_mut().filter(|f| f.started == started) else {
                    return false;
                };
                match turned.filter(|_| crate::sys::window::blends()) {
                    Some(turned) => {
                        f.turned = Some(turned);
                        cx.notify();
                        true
                    }
                    None => {
                        this.park(window, cx);
                        false
                    }
                }
            });
            if !turning.unwrap_or(false) {
                return;
            }
            cx.spawn(async move |cx| {
                let turn = crate::motion::tempo(crate::motion::FADE);
                cx.background_executor().timer(turn).await;
                cx.update(|cx| {
                    let _ = overlay.update(cx, |this, window, cx| {
                        if this.flight.as_ref().is_some_and(|f| f.started == started) {
                            this.park(window, cx);
                        }
                    });
                });
            })
            .detach();
        });
        // Window ops from inside a render are dropped by GPUI's effect
        // queue; open the toast after this frame.
        cx.defer(move |cx| {
            stage::show_toast_landed(cx, &path, thumb, Some(screen), Some(source), park)
        });
    }
}
