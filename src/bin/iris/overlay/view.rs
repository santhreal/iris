use std::time::Instant;

use gpui::*;

use super::{loupe::LOUPE_PX, shell::reveal, Overlay, DIM, DIM_FADE, HANDLE_PX, HOVER_FADE};
use crate::theme;

impl Overlay {
    /// The dim over the frozen frame as it fades in from the overlay's
    /// first render, and whether it is still fading.
    pub(super) fn entrance_dim(&mut self) -> (f32, bool) {
        let opened = *self.opened.get_or_insert_with(Instant::now);
        let dim_t = (opened.elapsed().as_secs_f32() / crate::motion::tempo(DIM_FADE).as_secs_f32())
            .min(1.0);
        (DIM * crate::motion::ease_out(dim_t), dim_t < 1.0)
    }

    /// The 8 resize handles of a committed selection, in logical px:
    /// 4 corners (NW, NE, SW, SE) then 4 edges (N, S, W, E). Each is a
    /// HANDLE_PX square centered on the point it drags.
    pub(super) fn handles(x: f32, y: f32, w: f32, h: f32) -> [(f32, f32); 8] {
        [
            (x, y),
            (x + w, y),
            (x, y + h),
            (x + w, y + h),
            (x + w / 2.0, y),
            (x + w / 2.0, y + h),
            (x, y + h / 2.0),
            (x + w, y + h / 2.0),
        ]
    }

    /// Which resize handle, if any, the logical point `p` is inside.
    pub(super) fn handle_at(&self, p: (f32, f32)) -> Option<usize> {
        let (x, y, w, h) = self.current?;
        let half = HANDLE_PX / 2.0;
        Self::handles(x, y, w, h).iter().position(|(hx, hy)| {
            p.0 >= hx - half && p.0 <= hx + half && p.1 >= hy - half && p.1 <= hy + half
        })
    }

    // Idle crosshair coordinates, like macOS region capture:
    // physical pixels next to the cursor, flipping at the edges.
    pub(super) fn render_crosshair_coord(
        &mut self,
        mut root: Stateful<Div>,
        window: &Window,
        sx: f32,
        sy: f32,
    ) -> Stateful<Div> {
        if !self.dragging && self.current.is_none() && self.hovered.is_none() {
            let sf = window.scale_factor();
            let win = window.bounds().size;
            let (fx, fy) = self.frame_pos(sf, sx, sy, self.cursor.0, self.cursor.1);
            if self.coord_at != Some((fx, fy)) {
                self.coord_at = Some((fx, fy));
                self.coord_label = SharedString::from(format!("{fx}, {fy}"));
            }
            let coord_label = self.coord_label.clone();
            let lx = if self.cursor.0 + 90.0 > f32::from(win.width) {
                self.cursor.0 - 82.0
            } else {
                self.cursor.0 + 18.0
            };
            let ly = if self.cursor.1 + 40.0 > f32::from(win.height) {
                self.cursor.1 - 30.0
            } else {
                self.cursor.1 + 16.0
            };
            root = root.child(
                div()
                    .absolute()
                    .left(px(lx))
                    .top(px(ly))
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .bg(theme::alpha(theme::BG_ELEV, 0.9))
                    .text_size(px(theme::TEXT_SMALL))
                    .child(coord_label),
            );
        }
        root
    }

    // Hover-snap highlight: un-dimmed reveal of the window's
    // rect, fading in; the window just left fades out behind it.
    // Before the frame lands there is nothing to reveal.
    pub(super) fn render_hover_snap(
        &mut self,
        mut root: Stateful<Div>,
        window: &mut Window,
        sx: f32,
        sy: f32,
    ) -> Stateful<Div> {
        if !self.dragging && self.frame_img.is_some() {
            if let Some((old, since)) = self.hover_out {
                let t = (since.elapsed().as_secs_f32()
                    / crate::motion::tempo(HOVER_FADE).as_secs_f32())
                .min(1.0);
                if t >= 1.0 {
                    self.hover_out = None;
                } else {
                    let sf = window.scale_factor();
                    let (rx, ry, rw, rh) = Self::to_logical(self.origin, &old, sf, sx, sy);
                    root = root.child(
                        reveal(
                            self.frame_img.clone().expect("checked"),
                            rx,
                            ry,
                            rw,
                            rh,
                            window,
                        )
                        .opacity(1.0 - t),
                    );
                    window.request_animation_frame();
                }
            }
            if let Some(w) = self.hovered {
                let sf = window.scale_factor();
                let (rx, ry, rw, rh) = Self::to_logical(self.origin, &w, sf, sx, sy);
                let mut alpha = 1.0f32;
                if let Some(since) = self.hover_in {
                    let t = (since.elapsed().as_secs_f32()
                        / crate::motion::tempo(HOVER_FADE).as_secs_f32())
                    .min(1.0);
                    alpha = t;
                    if t < 1.0 {
                        window.request_animation_frame();
                    }
                }
                root = root.child(
                    reveal(
                        self.frame_img.clone().expect("checked"),
                        rx,
                        ry,
                        rw,
                        rh,
                        window,
                    )
                    .opacity(alpha),
                );
            }
        }
        root
    }

    // Committed or in-progress selection.
    pub(super) fn render_selection(
        &mut self,
        mut root: Stateful<Div>,
        window: &Window,
        sx: f32,
        sy: f32,
    ) -> Stateful<Div> {
        if let Some((x, y, w, h)) = self.current {
            let sf = window.scale_factor();
            let phys = ((w * sf * sx).round() as u32, (h * sf * sy).round() as u32);
            if self.size_at != Some(phys) {
                self.size_at = Some(phys);
                self.size_label = SharedString::from(format!("{} × {}", phys.0, phys.1));
            }
            let size_label = self.size_label.clone();
            if let Some(fi) = self.frame_img.clone() {
                root = root.child(reveal(fi, x, y, w, h, window));
            }
            root = root.child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(if y < 44.0 { y + 4.0 } else { y - 28.0 }))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(6.))
                    .bg(theme::alpha(theme::BG_ELEV, 0.9))
                    .text_size(px(theme::TEXT_SMALL))
                    .text_color(theme::FG)
                    .child(size_label),
            );
            // A committed selection (drag released, not yet captured)
            // shows what confirms and what cancels. While still
            // dragging the loupe is the feedback; the hint would
            // flicker under it.
            if !self.dragging {
                let hint = self.hint.clone();
                root = root.child(
                    div()
                        .absolute()
                        .left(px(x))
                        .top(px(y + h + 6.0))
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(px(6.))
                        .bg(theme::alpha(theme::BG_ELEV, 0.9))
                        .text_size(px(theme::TEXT_SMALL))
                        .text_color(theme::FG_DIM)
                        .child(hint),
                );
            }
            // Resize handles on a committed selection: 8 grab points
            // the cursor can pull to reshape the rect before capture.
            if !self.dragging {
                for (hx, hy) in Self::handles(x, y, w, h) {
                    root = root.child(
                        div()
                            .absolute()
                            .left(px(hx - HANDLE_PX / 2.0))
                            .top(px(hy - HANDLE_PX / 2.0))
                            .w(px(HANDLE_PX))
                            .h(px(HANDLE_PX))
                            .rounded(px(2.))
                            .bg(theme::FG)
                            .border_1()
                            .border_color(theme::alpha(theme::BG, 0.6))
                            .shadow(theme::shadow_float()),
                    );
                }
            }
        }
        root
    }

    // Loupe while dragging or resizing: the window-snap highlight
    // is the hover feedback; no circle chasing the cursor. Flips
    // to the other side of the cursor near the screen edges.
    pub(super) fn render_loupe_widget(
        &self,
        mut root: Stateful<Div>,
        window: &Window,
    ) -> Stateful<Div> {
        if self.dragging || self.resize.is_some() {
            if let Some((loupe, info)) = &self.loupe {
                let win = window.bounds().size;
                let span = LOUPE_PX as f32 + 30.0;
                let lx = if self.cursor.0 + 24.0 + span > f32::from(win.width) {
                    self.cursor.0 - 24.0 - LOUPE_PX as f32
                } else {
                    self.cursor.0 + 24.0
                };
                let ly = if self.cursor.1 + 24.0 + span > f32::from(win.height) {
                    self.cursor.1 - 24.0 - LOUPE_PX as f32
                } else {
                    self.cursor.1 + 24.0
                };
                root = root.child(
                    div()
                        .absolute()
                        .left(px(lx))
                        .top(px(ly))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .w(px(LOUPE_PX as f32))
                                .h(px(LOUPE_PX as f32))
                                .rounded(px(theme::RADIUS_SM))
                                .overflow_hidden()
                                .border_1()
                                .border_color(theme::HAIRLINE)
                                // Lift the loupe off the dimmed desktop so
                                // it reads as floating chrome, like the
                                // toast and chip.
                                .shadow(theme::shadow_float())
                                .child(img(ImageSource::Render(loupe.clone())).size_full()),
                        )
                        .child(
                            div()
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(6.))
                                .bg(theme::alpha(theme::BG_ELEV, 0.9))
                                .text_size(px(theme::TEXT_SMALL))
                                .text_color(theme::FG_DIM)
                                .child(info.clone()),
                        ),
                );
            }
        }
        root
    }
}
