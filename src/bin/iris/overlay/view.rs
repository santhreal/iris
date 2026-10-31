use gpui::*;

use super::{loupe::LOUPE_PX, shell::reveal, Overlay, Scope, HANDLE_PX, HOVER_FADE};
use crate::theme;

/// The drawn diameter of a resize handle; its hit square is HANDLE_PX.
const HANDLE_DOT: f32 = 9.0;

/// One overlay readout: coordinates, the selection's size, or the
/// loupe's pixel. A small dark tag with no border, as the size readouts
/// of the macOS document apps are, reads over a white page and a dark
/// desktop alike. Digits are tabular so a number that changes under the
/// pointer keeps its width.
fn readout(text: SharedString, color: Rgba) -> Div {
    div()
        .px(px(6.))
        .py(px(2.))
        .rounded(px(5.))
        .bg(theme::READOUT_FILL)
        .text_size(px(theme::TEXT_SMALL))
        .font_weight(FontWeight::MEDIUM)
        .font_features(theme::tabular())
        .whitespace_nowrap()
        .text_color(color)
        .child(text)
}

/// The selection's edge around a logical rect: a 1 px white line on the
/// rect's border and a 1 px dark rim just outside it.
fn selection_edge(x: f32, y: f32, w: f32, h: f32) -> Div {
    div()
        .absolute()
        .left(px(x - 1.0))
        .top(px(y - 1.0))
        .w(px(w + 2.0))
        .h(px(h + 2.0))
        .border_1()
        .border_color(theme::SELECTION_RIM)
        .child(
            div()
                .size_full()
                .border_1()
                .border_color(theme::SELECTION_EDGE),
        )
}

impl Overlay {
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
        if self.cursor_known
            && !self.over_bar
            && !self.dragging
            && self.current.is_none()
            && self.hovered.is_none()
        {
            let sf = window.scale_factor();
            let win = window.bounds().size;
            // The readout states root coordinates: the frame pixel
            // plus the virtual screen's origin.
            let (fx, fy) = Self::frame_pos(sf, sx, sy, self.cursor.0, self.cursor.1);
            let (fx, fy) = (fx + i64::from(self.origin.0), fy + i64::from(self.origin.1));
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
                readout(coord_label, theme::FG)
                    .absolute()
                    .left(px(lx))
                    .top(px(ly)),
            );
        }
        root
    }

    // Hover-snap highlight: un-dimmed reveal of the window's
    // rect under the pick tint, fading in; the window just left fades
    // out behind it. Before the frame lands there is nothing to
    // reveal, and a committed selection shows no window.
    pub(super) fn render_hover_snap(
        &mut self,
        mut root: Stateful<Div>,
        window: &mut Window,
        sx: f32,
        sy: f32,
    ) -> Stateful<Div> {
        if !self.dragging && self.current.is_none() && self.frame_img.is_some() {
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
                        .child(div().absolute().inset_0().bg(theme::WINDOW_PICK))
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
                    .child(div().absolute().inset_0().bg(theme::WINDOW_PICK))
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
            if let Some(fi) = self.frame_img.clone() {
                root = root.child(reveal(fi, x, y, w, h, window));
            }
            // The entire screen is the monitor, un-dimmed: no edge, no
            // size, and no handles to reshape it.
            if self.scope == Scope::Screen {
                return root;
            }
            root = root.child(selection_edge(x, y, w, h));
            let size_label = self.size_label.clone();
            root = root.child(
                readout(size_label, theme::FG)
                    .absolute()
                    .left(px(x))
                    .top(px(if y < 28.0 { y + 6.0 } else { y - 24.0 })),
            );
            // Resize handles on a committed selection: 8 grab points
            // the cursor can pull to reshape the rect before capture.
            if !self.dragging {
                for (hx, hy) in Self::handles(x, y, w, h) {
                    root = root.child(
                        div()
                            .absolute()
                            .left(px(hx - HANDLE_DOT / 2.0))
                            .top(px(hy - HANDLE_DOT / 2.0))
                            .size(px(HANDLE_DOT))
                            .rounded_full()
                            .bg(theme::SELECTION_EDGE)
                            .border_1()
                            .border_color(theme::SELECTION_RIM)
                            .shadow(theme::shadow_knob()),
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
                        .w(px(LOUPE_PX as f32))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .w(px(LOUPE_PX as f32))
                                .h(px(LOUPE_PX as f32))
                                .rounded(px(theme::RADIUS_GROUP))
                                .overflow_hidden()
                                .border_1()
                                .border_color(theme::SELECTION_EDGE)
                                // Lift the loupe off the dimmed desktop so
                                // it reads as floating chrome, like the
                                // toast and chip.
                                .shadow(theme::shadow_float())
                                .child(img(ImageSource::Render(loupe.clone())).size_full()),
                        )
                        .child(readout(info.clone(), theme::FG)),
                );
            }
        }
        root
    }
}
