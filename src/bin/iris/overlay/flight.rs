//! The capture flight: the committed region springs from the selection
//! rect to the toast's resting corner rect, then hands off to the toast.

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
    pub(super) to: (f32, f32, f32, f32),
    pub(super) started: Instant,
    /// The committing window's screen rect: the toast lands on this
    /// monitor, not wherever the primary display happens to be.
    pub(super) screen: (f32, f32, f32, f32),
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
    /// selection rect to the toast's corner rect while the dim lifts.
    /// Once the card is at rest and the background finalize has landed,
    /// the toast opens underneath; the overlay keeps the card on screen
    /// until the toast has presented its first frame, then parks.
    pub(super) fn render_flight(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        const FLIGHT: Duration = Duration::from_millis(500);
        let f = self.flight.clone().expect("flight checked");
        let flown = f.started.elapsed().as_secs_f32();
        let t = (flown / crate::motion::tempo(FLIGHT).as_secs_f32()).min(1.0);
        let e = crate::motion::spring(t);
        // The clock's end bounds the flight whatever the curve does.
        let at_rest = t >= 1.0 || flight_at_rest(f.from, f.to, e);
        // The dim lifts as the card leaves.
        let lift = (flown / crate::motion::tempo(crate::motion::FADE).as_secs_f32()).min(1.0);
        let dim = self.entrance_dim().0 * (1.0 - crate::motion::ease_out(lift));
        if at_rest {
            // Taken once: later frames of this session never reopen it.
            // Until then a slow finalize's notify re-renders this.
            if let Some(landed) = self.landed.take() {
                self.hand_off(landed, &f, window, cx);
            }
        }
        if !at_rest || lift < 1.0 {
            window.request_animation_frame();
        }
        // The card lifts off the page: square and flat where it leaves
        // the selection, and at rest the toast's card to the pixel, its
        // rect, corners, and shadow, so the handoff changes nothing.
        let (e, (x, y, w, h)) = if at_rest {
            (1.0, f.to)
        } else {
            (e, flight_rect(f.from, f.to, e))
        };
        let radius = px(stage::RADIUS * e);
        div()
            .size_full()
            // The frozen frame: the desktop as it was.
            .children(self.frame_img.clone().map(|i| {
                img(ImageSource::Render(i))
                    .size_full()
                    .object_fit(ObjectFit::Fill)
            }))
            .children((dim > 0.0).then(|| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(theme::alpha(theme::BG, dim))
            }))
            .child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w.max(1.0)))
                    .h(px(h.max(1.0)))
                    .rounded(radius)
                    .shadow(theme::card_shadow(e))
                    .child(
                        // The image rounds itself: a parent's rounded
                        // overflow clip is a rectangle.
                        img(ImageSource::Render(f.img.clone()))
                            .size_full()
                            .object_fit(ObjectFit::Fill)
                            .rounded(radius),
                    ),
            )
    }

    /// Open the landed toast and park this window once the toast is on
    /// screen. The park checks the flight's start instant: a session
    /// re-armed in between owns the window and is left alone.
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
        let (started, screen) = (f.started, f.screen);
        let park = Box::new(move |cx: &mut App| {
            let _ = overlay.update(cx, |this, window, cx| {
                if this.flight.as_ref().is_some_and(|f| f.started == started) {
                    this.park(window, cx);
                }
            });
        });
        // Window ops from inside a render are dropped by GPUI's effect
        // queue; open the toast after this frame.
        cx.defer(move |cx| stage::show_toast_landed(cx, &path, thumb, Some(screen), park));
    }
}
