//! Stage rendering: image canvas, committed action overlays, crop & selection handles, mouse/scroll listeners.

use std::rc::Rc;

use gpui::*;
use iris_lib::history::Edit;

use super::action::{hex_rgba, text_size, Tool};
use super::paint::paint_action;
use super::Editor;
use crate::theme;

impl Editor {
    pub(super) fn render_stage(
        &mut self,
        stage_rect: (f32, f32, f32, f32),
        morph_radius: f32,
        chrome: f32,
        scale: f32,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (sx, sy, sw, sh) = stage_rect;

        // Stage: the image and everything drawn on it.
        let mut stage = div()
            .id("stage")
            .absolute()
            .left(px(sx))
            .top(px(sy))
            .w(px(sw))
            .h(px(sh))
            .rounded(px(morph_radius))
            .overflow_hidden()
            .border_1()
            // The toast has no border and one shadow; the stage's
            // hairline and contact shadow fade in with the chrome so
            // the flying card never gains edges mid-flight.
            .border_color(theme::alpha(theme::HAIRLINE, 0.09 * chrome))
            .shadow(vec![
                BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.35),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(24.),
                    spread_radius: px(0.),
                },
                BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.22 * chrome),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(6.),
                    spread_radius: px(0.),
                },
            ])
            .child(
                img(ImageSource::Render(self.base_img.clone()))
                    .size_full()
                    .rounded(px(morph_radius)),
            );

        // Committed vector actions + blur patches + text.
        let actions = Rc::clone(&self.actions);
        let current = self.current.clone();
        let (base_w, base_h) = (self.base.width(), self.base.height());
        stage = stage.child(
            canvas(
                move |_, _, _| (actions.clone(), current.clone()),
                move |bounds, (actions, current), window, _cx| {
                    let cur = current.as_ref().map(|rc| rc.borrow());
                    for action in actions.borrow().iter().chain(cur.as_deref()) {
                        paint_action(action, bounds, scale, base_w, base_h, window);
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
        let cur = self.current.as_ref().map(|rc| rc.borrow());
        for action in self.actions.borrow().iter().chain(cur.as_deref()) {
            if action.tool == Tool::Blur {
                if let Some(patch) = &action.blur_patch {
                    let (x, y, w, h) = action.blur_rect;
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px(x * scale))
                            .top(px(y * scale))
                            .w(px(w * scale))
                            .h(px(h * scale))
                            .child(img(ImageSource::Render(patch.clone())).size_full()),
                    );
                }
            } else if action.tool == Tool::Text {
                if let (Some(text), Some(p)) = (&action.text, action.points.first()) {
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px(p.0 * scale))
                            .top(px((p.1 - action.font_size) * scale))
                            .text_color(hex_rgba(action.color))
                            .text_size(px(action.font_size * scale))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(text.clone()),
                    );
                }
            } else if action.tool == Tool::Counter {
                if let Some(p) = action.points.first() {
                    let num = action.step_label.clone();
                    let nw = num.chars().count() as f32 * action.font_size * 0.6;
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px((p.0 - nw / 2.0) * scale))
                            .top(px((p.1 - action.font_size * 0.55) * scale))
                            .text_color(theme::FG)
                            .text_size(px(action.font_size * scale))
                            .font_weight(FontWeight::BOLD)
                            .child(num),
                    );
                }
            }
        }

        // Active text entry with a blinking caret. The blink is a
        // 1.06s cycle; an 8Hz timer repaints it instead of pinning the
        // whole window to vsync for the entry's lifetime.
        if let Some(entry) = &self.text_entry {
            if !self.caret_timer {
                self.caret_timer = true;
                cx.spawn(async move |this, cx| loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(125))
                        .await;
                    let alive = this.update(cx, |this, cx| {
                        if this.text_entry.is_some() {
                            cx.notify();
                            true
                        } else {
                            this.caret_timer = false;
                            false
                        }
                    });
                    if matches!(alive, Ok(false) | Err(_)) {
                        break;
                    }
                })
                .detach();
            }
            let blink_on = (self.caret_started.elapsed().as_millis() % 1060) < 580;
            let p = entry.point;
            let size = text_size(base_w);
            stage = stage.child(
                div()
                    .absolute()
                    .left(px(p.0 * scale))
                    .top(px((p.1 - size) * scale))
                    .text_color(hex_rgba(self.color))
                    .text_size(px(size * scale))
                    .child(if blink_on {
                        entry.caret.clone()
                    } else {
                        entry.buffer_str.clone()
                    }),
            );
        }

        // Selection outline around the active action.
        if let Some(i) = self.selected {
            if let Some((x, y, w, h)) = self.actions.borrow().get(i).and_then(|a| a.bbox) {
                stage = stage.child(
                    div()
                        .absolute()
                        .left(px(x * scale))
                        .top(px(y * scale))
                        .w(px(w * scale))
                        .h(px(h * scale))
                        .border_1()
                        .border_color(theme::alpha(theme::FG, 0.75)),
                );
            }
        }

        // Pending crop: dim everything outside the rect, hairline and
        // corner handles on it.
        if let Some((x, y, w, h)) = self.crop_rect {
            let (rx, ry, rw, rh) = (x * scale, y * scale, w * scale, h * scale);
            let dim = hsla(0.0, 0.0, 0.0, 0.45);
            for (bx, by, bw, bh) in [
                (0.0, 0.0, sw, ry),
                (0.0, ry + rh, sw, sh - ry - rh),
                (0.0, ry, rx, rh),
                (rx + rw, ry, sw - rx - rw, rh),
            ] {
                if bw > 0.0 && bh > 0.0 {
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px(bx))
                            .top(px(by))
                            .w(px(bw))
                            .h(px(bh))
                            .bg(dim),
                    );
                }
            }
            stage = stage.child(
                div()
                    .absolute()
                    .left(px(rx))
                    .top(px(ry))
                    .w(px(rw))
                    .h(px(rh))
                    .border_1()
                    .border_color(theme::FG),
            );
            for (hx, hy) in [(rx, ry), (rx + rw, ry), (rx, ry + rh), (rx + rw, ry + rh)] {
                stage = stage.child(
                    div()
                        .absolute()
                        .left(px(hx - 3.5))
                        .top(px(hy - 3.5))
                        .w(px(7.))
                        .h(px(7.))
                        .rounded(px(2.))
                        .bg(theme::FG),
                );
            }
        }

        // The cursor follows the active tool: I-beam over text,
        // crosshair for everything drawable, arrow for Select.
        stage = match self.tool {
            Tool::Text => stage.cursor_text(),
            Tool::Select => stage,
            _ => stage.cursor_crosshair(),
        };

        // Stage input.
        stage = stage
            // Scroll zooms around the cursor: the image point under the
            // pointer stays put while the scale changes.
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                let dy: f32 = match ev.delta {
                    ScrollDelta::Pixels(p) => f32::from(p.y),
                    ScrollDelta::Lines(p) => p.y * 20.0,
                };
                if dy.abs() < 0.5 {
                    return;
                }
                let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                let (ox, oy, scale) = this.view(window);
                let factor = if dy > 0.0 { 1.12 } else { 1.0 / 1.12 };
                let new_zoom = (this.zoom * factor).clamp(0.1, 16.0);
                if (new_zoom - this.zoom).abs() < f32::EPSILON {
                    return;
                }
                // Keep the image point under the cursor fixed:
                // mx = ox' + ix * scale' => solve for new pan.
                let (ix, iy) = ((mx - ox) / scale, (my - oy) / scale);
                let (nfx, nfy) = this.fit_origin(window, (scale / this.zoom) * new_zoom);
                let new_scale = (scale / this.zoom) * new_zoom;
                this.pan = (mx - nfx - ix * new_scale, my - nfy - iy * new_scale);
                this.zoom = new_zoom;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    this.pan_drag = Some((ev.position.x.into(), ev.position.y.into()));
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                    if this.space_pan {
                        this.pan_drag = Some((mx, my));
                        cx.notify();
                        return;
                    }
                    let p = this.to_image(ev.position, window);
                    if this.tool == Tool::Select {
                        this.commit_text(true);
                        this.selected = this.hit_action(p);
                        if let Some(i) = this.selected {
                            if let Some(a) = this.actions.borrow().get(i) {
                                if let Some(bb) = a.bbox {
                                    this.move_drag = Some((i, p, a.clone(), bb));
                                }
                            }
                        }
                    } else if this.tool == Tool::Text {
                        this.commit_text(true);
                        this.text_entry = Some(super::action::TextEntry {
                            point: p,
                            buffer: String::new(),
                            caret: "▏".into(),
                            buffer_str: "".into(),
                        });
                        this.caret_started = std::time::Instant::now();
                    } else if this.tool == Tool::Crop {
                        this.commit_text(true);
                        // Hit-test handles first; inside rect starts a
                        // move drag; outside starts a fresh crop.
                        let inside = this
                            .crop_rect
                            .map(|(x, y, w, h)| {
                                p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h
                            })
                            .unwrap_or(false);
                        if inside {
                            this.crop_move = Some((p, this.crop_rect.unwrap()));
                        } else {
                            this.crop_rect = Some((p.0, p.1, 0.0, 0.0));
                            this.crop_anchor = Some(p);
                        }
                    } else {
                        this.commit_text(true);
                        let a = this.new_action(p);
                        this.current = Some(Rc::new(std::cell::RefCell::new(a)));
                    }
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, window, cx| {
                let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                if let Some(last) = this.pan_drag {
                    this.pan.0 += mx - last.0;
                    this.pan.1 += my - last.1;
                    this.pan_drag = Some((mx, my));
                    cx.notify();
                    return;
                }
                let p = this.to_image(ev.position, window);
                if let Some((i, last, _, start_bbox)) = this.move_drag {
                    let d = (p.0 - last.0, p.1 - last.1);
                    let (dx, dy) = this.move_action(i, d, start_bbox);
                    this.move_drag = Some((
                        i,
                        (last.0 + dx, last.1 + dy),
                        this.move_drag.as_ref().unwrap().2.clone(),
                        start_bbox,
                    ));
                    cx.notify();
                } else if let Some(anchor) = this.crop_anchor {
                    let (x0, y0) = (anchor.0.min(p.0), anchor.1.min(p.1));
                    let (w, h) = ((p.0 - anchor.0).abs(), (p.1 - anchor.1).abs());
                    this.crop_rect = Some((x0, y0, w, h));
                    cx.notify();
                } else if let Some((last, (x, y, w, h))) = this.crop_move {
                    let d = (p.0 - last.0, p.1 - last.1);
                    let iw = this.base.width() as f32;
                    let ih = this.base.height() as f32;
                    let nx = (x + d.0).clamp(0.0, (iw - w).max(0.0));
                    let ny = (y + d.1).clamp(0.0, (ih - h).max(0.0));
                    this.crop_rect = Some((nx, ny, w, h));
                    cx.notify();
                } else if let Some(rc) = &this.current {
                    let mut action = rc.borrow_mut();
                    match action.tool {
                        Tool::Pen | Tool::Highlight => {
                            let points = Rc::make_mut(&mut action.points);
                            let last = points.last().copied().unwrap_or(p);
                            if (last.0 - p.0).hypot(last.1 - p.1) > 2.0 {
                                points.push(p);
                            }
                        }
                        Tool::Line | Tool::Arrow => {
                            let points = Rc::make_mut(&mut action.points);
                            if points.len() < 2 {
                                points.push(p);
                            } else {
                                points[1] = p;
                            }
                        }
                        Tool::Ellipse | Tool::Rect | Tool::Blur => {
                            let points = Rc::make_mut(&mut action.points);
                            if points.len() < 2 {
                                points.push(p);
                            } else {
                                points[1] = p;
                            }
                        }
                        Tool::Select | Tool::Text | Tool::Crop | Tool::Counter => {}
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, _ev: &MouseUpEvent, _, cx| {
                    this.pan_drag = None;
                    cx.notify();
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _ev: &MouseUpEvent, _, cx| {
                    if this.pan_drag.is_some() {
                        this.pan_drag = None;
                        cx.notify();
                        return;
                    }
                    if let Some((i, _, old_action, _)) = this.move_drag.take() {
                        let new_action = this.actions.borrow().get(i).cloned();
                        if let Some(new_action) = new_action {
                            let edit = Edit::Move(i, old_action, new_action);
                            this.rebuild_for_edit(&edit);
                            this.push_edit(edit);
                        }
                    }
                    this.crop_anchor = None;
                    this.crop_move = None;
                    this.commit_current();
                    cx.notify();
                }),
            );

        stage
    }
}
