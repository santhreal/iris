//! Stage rendering: image canvas, committed action overlays, crop & selection handles, mouse/scroll listeners.

use std::rc::Rc;

use gpui::*;
use iris_lib::history::Edit;

use super::action::{hex_rgba, text_size, Tool};
use super::paint::paint_action;
use super::raster::{counter_label_at, glyph_span, text_advance, TEXT_LINE};
use super::render_chrome::STAGE_TOP;
use super::Editor;
use crate::theme;
use crate::widgets::selection;

/// The caret's blink: shown for the first 580 ms of each 1060 ms from
/// the entry's open or its last key, hidden for the rest.
const CARET_SHOWN_MS: u128 = 580;
const CARET_CYCLE_MS: u128 = 1060;

/// Whether the caret shows `elapsed` into its blink.
pub(super) fn caret_shown(elapsed: std::time::Duration) -> bool {
    elapsed.as_millis() % CARET_CYCLE_MS < CARET_SHOWN_MS
}

/// How long after `elapsed` into its blink the caret next shows or
/// hides: a wait that long, or longer, lands in the next phase.
pub(super) fn caret_flip_in(elapsed: std::time::Duration) -> std::time::Duration {
    let at = elapsed.as_millis() % CARET_CYCLE_MS;
    let flip = if at < CARET_SHOWN_MS {
        CARET_SHOWN_MS
    } else {
        CARET_CYCLE_MS
    };
    std::time::Duration::from_millis((flip - at) as u64)
}

/// The space, logical px, between a selected mark's bounds and the
/// white line of its frame.
const MARK_FRAME_GAP: f32 = 3.0;

impl Editor {
    pub(super) fn render_stage(
        &mut self,
        stage_rect: (f32, f32, f32, f32),
        morph_radius: f32,
        chrome: f32,
        scale: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (sx, sy, sw, sh) = stage_rect;

        // Stage: the image and everything drawn on it, filling the rect
        // the view maps the image to.
        let mut stage = div().id("stage").size_full().overflow_hidden().child(
            img(ImageSource::Render(self.base_img.clone()))
                .size_full()
                .rounded(px(morph_radius)),
        );

        // Committed vector actions + blur patches + text.
        let actions = Rc::clone(&self.actions);
        let current = self.current.clone();
        let (base_w, base_h) = self.base_dims;
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
        // A reopened label shows as its entry, not as the mark.
        let reopened = self.text_entry.as_ref().and_then(|e| e.edits);
        for (i, action) in self
            .actions
            .borrow()
            .iter()
            .chain(cur.as_deref())
            .enumerate()
        {
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
            } else if action.tool == Tool::Text && reopened != Some(i) {
                if let (Some(text), Some(p)) = (&action.text, action.points.first()) {
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px(p.0 * scale))
                            .top(px((p.1 - action.font_size) * scale))
                            .text_color(hex_rgba(action.color))
                            .text_size(px(action.font_size * scale))
                            .line_height(px(TEXT_LINE * action.font_size * scale))
                            .font_weight(FontWeight::BOLD)
                            .child(text.clone()),
                    );
                }
            } else if action.tool == Tool::Counter {
                if let Some(p) = action.points.first() {
                    let num = action.step_label.clone();
                    let at = counter_label_at(*p, action.font_size, &num);
                    stage = stage.child(
                        div()
                            .absolute()
                            .left(px(at.0 * scale))
                            .top(px(at.1 * scale))
                            .text_color(white())
                            .text_size(px(action.font_size * scale))
                            .line_height(px(TEXT_LINE * action.font_size * scale))
                            .font_weight(FontWeight::BOLD)
                            .child(num),
                    );
                }
            }
        }

        // Active text entry with a blinking caret. A timer repaints it at
        // each flip of the blink instead of pinning the whole window to
        // vsync for the entry's lifetime.
        if let Some(entry) = &self.text_entry {
            if !self.caret_timer {
                self.caret_timer = true;
                let first = caret_flip_in(self.caret_started.elapsed());
                cx.spawn(async move |this, cx| {
                    let mut wait = first;
                    loop {
                        cx.background_executor().timer(wait).await;
                        // A key restarts the blink: a wake for the flip
                        // before it lands short of the new one, and
                        // waits again.
                        let next = this.update(cx, |this, cx| {
                            if this.text_entry.is_some() {
                                cx.notify();
                                Some(caret_flip_in(this.caret_started.elapsed()))
                            } else {
                                this.caret_timer = false;
                                None
                            }
                        });
                        match next {
                            Ok(Some(next)) => wait = next,
                            Ok(None) | Err(_) => break,
                        }
                    }
                })
                .detach();
            }
            let blink_on = caret_shown(self.caret_started.elapsed());
            let p = entry.point;
            let size = text_size(base_w);
            let top = p.1 - size;
            // Selected text sits on the selection fill, drawn under the
            // text across the glyph span.
            let sel = entry.line.selection();
            if !sel.is_empty() {
                let text = entry.line.text();
                let (offset, height) = glyph_span(size);
                let from = text_advance(&text[..sel.start], size);
                let to = text_advance(&text[..sel.end], size);
                stage = stage.child(
                    div()
                        .absolute()
                        .left(px((p.0 + from) * scale))
                        .top(px((top + offset) * scale))
                        .w(px((to - from) * scale))
                        .h(px(height * scale))
                        .bg(theme::TEXT_SELECTION),
                );
            }
            stage = stage.child(
                div()
                    .absolute()
                    .left(px(p.0 * scale))
                    .top(px(top * scale))
                    .text_color(hex_rgba(self.color))
                    .text_size(px(size * scale))
                    .line_height(px(TEXT_LINE * size * scale))
                    .font_weight(FontWeight::BOLD)
                    .child(entry.buffer_str.clone()),
            );
            // The caret is a bar, not a glyph: no bundled face has one,
            // and a fallback face differs per system or is missing. It
            // sits a pixel past the advance of the text before it, clear
            // of a glyph with a narrow side bearing.
            if blink_on {
                let (offset, height) = glyph_span(size);
                let advance = text_advance(entry.line.split().0, size);
                stage = stage.child(
                    div()
                        .absolute()
                        .left(px((p.0 + advance) * scale + 1.0))
                        .top(px((top + offset) * scale))
                        .w(px((size * scale * 0.08).clamp(1.5, 3.0)))
                        .h(px(height * scale))
                        .bg(hex_rgba(self.color)),
                );
            }
        }

        // The selected mark's frame: the crop's white line and dark rim,
        // MARK_FRAME_GAP outside the mark's ink and on whole device
        // pixels, so it reads on a light image and a dark one and clear
        // of the stroke. It goes over the stage's frame, unclipped, as
        // the crop's edge does.
        let mut mark_frame = None;
        if let Some(i) = self.selected {
            if let Some((x, y, w, h)) = self.actions.borrow().get(i).and_then(|a| a.ink_box()) {
                let sf = window.scale_factor();
                let snap = |v: f32| (v * sf).round() / sf;
                let (l, t) = (
                    snap(x * scale - MARK_FRAME_GAP),
                    snap(y * scale - MARK_FRAME_GAP),
                );
                let (r, b) = (
                    snap((x + w) * scale + MARK_FRAME_GAP),
                    snap((y + h) * scale + MARK_FRAME_GAP),
                );
                let hair = selection::hairline(sf);
                mark_frame = Some(selection::selection_edge(l, t, r - l, b - t, hair));
            }
        }

        // A drag that runs or waits on a press shows its own cursor
        // over the tool's: a crop handle's arrow while it resizes, the
        // closed hand while a pan or the crop's move runs, and the open
        // hand while Space waits to pan.
        let grab = selection::grab_cursor(
            self.crop_resize.map(|(h, _)| h),
            self.pan_drag.is_some() || self.crop_move.is_some(),
            self.space_pan,
        );

        // Pending crop: dim everything outside the rect. Its edge,
        // handles, and size readout go over the stage's frame, which
        // does not clip them, so a crop against the image's edge keeps
        // them whole.
        let mut crop_frame = None;
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
            // The readout states the pixels the crop keeps.
            let (label, tag_w) = self.crop_tag.of((w as u32, h as u32), window);
            let hair = selection::hairline(window.scale_factor());
            // Above the crop where the options bar leaves room, inside
            // the part of the image the window shows.
            let visible = (
                (-sx).max(0.0),
                STAGE_TOP - sy,
                sw.min(f32::from(window.client_area().size.width) - sx),
            );
            let (tag_x, tag_y) = selection::size_readout_at((rx, ry, rw, rh), hair, visible, tag_w);
            let mut frame = div()
                .absolute()
                .left_0()
                .top_0()
                .w(px(sw))
                .h(px(sh))
                .child(selection::selection_edge(rx, ry, rw, rh, hair))
                .child(
                    selection::readout(label, theme::FG)
                        .absolute()
                        .left(px(tag_x))
                        .top(px(tag_y)),
                );
            // The handles, centered on the white line, once the crop is
            // drawn. A handle takes its own press: most of a corner's
            // square lies off an image the crop reaches the edge of.
            // The corners go on last, over the edges, so where squares
            // overlap on a small crop a corner takes the press, as
            // `selection::handle_at` picks.
            if self.crop_anchor.is_none() {
                // Inside the crop a press moves it: the open hand, under
                // the handles, which show their arrows.
                if self.tool == Tool::Crop && grab.is_none() {
                    frame = frame.child(
                        div()
                            .absolute()
                            .left(px(rx))
                            .top(px(ry))
                            .w(px(rw))
                            .h(px(rh))
                            .cursor(CursorStyle::OpenHand),
                    );
                }
                let (ex, ey) = (rx - hair / 2.0, ry - hair / 2.0);
                for (i, (hx, hy)) in selection::shown_handles(ex, ey, rw + hair, rh + hair).rev() {
                    let mut handle = selection::handle(("crop-handle", i), i, hx, hy);
                    if let Some(cursor) = grab {
                        handle = handle.cursor(cursor);
                    }
                    frame = frame.child(handle.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            if this.space_pan {
                                this.pan_drag = Some((ev.position.x.into(), ev.position.y.into()));
                            } else if let Some(rect) = this.crop_rect {
                                this.commit_text(true);
                                this.crop_resize = Some((i, rect));
                            }
                            cx.notify();
                        }),
                    ));
                }
            }
            crop_frame = Some(frame);
        }

        // The cursor follows the active tool (`Tool::cursor`), or the
        // drag that runs or waits.
        stage = stage.cursor(grab.unwrap_or(self.tool.cursor()));

        // Stage input.
        stage = stage
            // Scroll and pinch zoom around the cursor: the image point
            // under the pointer stays put while the scale changes.
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                let at = crate::widgets::frame_local(window, ev.position);
                let at = (at.x.into(), at.y.into());
                if this.zoom_at(at, crate::zoom::wheel_zoom(ev.delta), window) {
                    cx.notify();
                }
            }))
            .on_pinch(cx.listener(|this, ev: &PinchEvent, window, cx| {
                let at = crate::widgets::frame_local(window, ev.position);
                let at = (at.x.into(), at.y.into());
                if this.zoom_at(at, crate::zoom::pinch_zoom(ev.delta), window) {
                    cx.notify();
                }
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
                    // The backdrop behind the stage settles an open text
                    // entry on press; this press is the stage's own, so
                    // it must not reach the backdrop and commit the entry
                    // the Text tool opens below.
                    cx.stop_propagation();
                    let p = this.to_image(ev.position, window);
                    this.entry_drag = false;
                    // A press on the open entry's text moves its caret
                    // there, with Shift selecting to there, a drag from
                    // it selects, a double-click selects a word and a
                    // triple-click everything, and the entry stays open.
                    if this.press_entry(p, ev.click_count, ev.modifiers.shift) {
                        cx.notify();
                        return;
                    }
                    if this.tool == Tool::Select {
                        this.commit_text(true);
                        // A double-click on a label reopens it.
                        if ev.click_count >= 2 {
                            if let Some(i) = this.hit_text(p) {
                                this.edit_text(i, p, ev.click_count);
                                cx.notify();
                                return;
                            }
                        }
                        this.select(this.hit_action(p));
                        if let Some(i) = this.selected {
                            if let Some(a) = this.actions.borrow().get(i) {
                                if let Some(bb) = a.bbox {
                                    this.move_drag = Some((i, p, a.clone(), bb));
                                }
                            }
                        }
                    } else if this.tool == Tool::Text {
                        this.commit_text(true);
                        // A press on a label reopens it; anywhere else
                        // starts a new one.
                        match this.hit_text(p) {
                            Some(i) => this.edit_text(i, p, ev.click_count),
                            None => {
                                this.text_entry = Some(super::action::TextEntry {
                                    point: p,
                                    line: Default::default(),
                                    buffer_str: "".into(),
                                    edits: None,
                                });
                                this.caret_started = std::time::Instant::now();
                            }
                        }
                    } else if this.tool == Tool::Crop {
                        this.commit_text(true);
                        // Inside the rect a press moves it; outside it
                        // starts a fresh crop. A handle takes its own.
                        let inside = this
                            .crop_rect
                            .map(|(x, y, w, h)| {
                                p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h
                            })
                            .unwrap_or(false);
                        if inside {
                            this.crop_move = Some((p, this.crop_rect.unwrap()));
                        } else {
                            this.begin_crop(p);
                        }
                    } else {
                        this.commit_text(true);
                        let a = this.new_action(p);
                        this.current = Some(Rc::new(std::cell::RefCell::new(a)));
                    }
                    cx.notify();
                }),
            );

        // The stage's drags (a mark drawn or moved, a crop drawn, moved,
        // or resized, a text selection, a pan) follow the pointer past
        // the image's edge and out of the window, and end at the release
        // wherever it is: the listeners run in the capture phase, for
        // every move and release in the window, not under the stage.
        let this = cx.entity().downgrade();
        stage = stage.child(
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let moved = this.clone();
                    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            let _ = moved.update(cx, |this, cx| {
                                if this.stage_drag(ev, window) {
                                    cx.notify();
                                }
                            });
                        }
                    });
                    let released = this.clone();
                    window.on_mouse_event(move |ev: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            let _ = released.update(cx, |this, cx| {
                                if this.stage_release(ev.button) {
                                    cx.notify();
                                }
                            });
                        }
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );

        // The frame: the hairline ring and the shadow, a pixel outside
        // the stage, so no captured pixel lies under the ring and the
        // image is not squeezed by it. The toast has no border and one
        // shadow; the hairline and contact shadow fade in with the
        // chrome so the flying card never gains edges mid-flight.
        div()
            .absolute()
            .left(px(sx - 1.0))
            .top(px(sy - 1.0))
            .w(px(sw + 2.0))
            .h(px(sh + 2.0))
            .rounded(px(morph_radius + 1.0))
            .border_1()
            .border_color(theme::alpha(theme::HAIRLINE, 0.09 * chrome))
            .shadow(vec![
                BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.35),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(24.),
                    spread_radius: px(0.),
                    inset: false,
                },
                BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.22 * chrome),
                    offset: point(px(0.), px(2.)),
                    blur_radius: px(6.),
                    spread_radius: px(0.),
                    inset: false,
                },
            ])
            .child(stage)
            .children(mark_frame)
            .children(crop_frame)
    }
}

impl Editor {
    /// Start a fresh crop at image point `p`, or at the image's point
    /// nearest it for a press beside the image: the corner a drag from
    /// there reshapes.
    pub(super) fn begin_crop(&mut self, p: (f32, f32)) {
        let p = (
            p.0.clamp(0.0, self.base_dims.0 as f32),
            p.1.clamp(0.0, self.base_dims.1 as f32),
        );
        self.crop_rect = Some((p.0, p.1, 0.0, 0.0));
        self.crop_anchor = Some(p);
    }

    /// A pointer move, anywhere: it carries on the stage drag in
    /// progress, if any. Returns whether the editor needs a redraw.
    fn stage_drag(&mut self, ev: &MouseMoveEvent, window: &Window) -> bool {
        let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
        if let Some(last) = self.pan_drag {
            self.pan.0 += mx - last.0;
            self.pan.1 += my - last.1;
            self.pan_drag = Some((mx, my));
            return true;
        }
        let p = self.to_image(ev.position, window);
        if self.entry_drag {
            if ev.pressed_button == Some(MouseButton::Left) && self.drag_entry(p) {
                return true;
            }
            self.entry_drag = false;
        }
        if let Some((i, last, _, start_bbox)) = self.move_drag {
            let d = (p.0 - last.0, p.1 - last.1);
            let (dx, dy) = self.move_action(i, d, start_bbox);
            if let Some(drag) = &mut self.move_drag {
                drag.1 = (last.0 + dx, last.1 + dy);
            }
            return true;
        }
        // A crop dropped mid-drag, by Escape, Return, or Undo, ends the
        // drag rather than coming back under the pointer.
        if self.crop_rect.is_none() {
            self.crop_anchor = None;
            self.crop_move = None;
            self.crop_resize = None;
        }
        // A crop stays on the image: drawn, moved, or reshaped by a
        // pointer past its edge, its side stops at the edge. Drawing
        // one is dragging the SE corner of the rect at its anchor.
        let bounds = (self.base_dims.0 as f32, self.base_dims.1 as f32);
        if let Some(anchor) = self.crop_anchor {
            self.crop_rect = Some(selection::reshape(
                3,
                (anchor.0, anchor.1, 0.0, 0.0),
                p,
                bounds,
            ));
            return true;
        }
        if let Some((last, (x, y, w, h))) = self.crop_move {
            let d = (p.0 - last.0, p.1 - last.1);
            let nx = (x + d.0).clamp(0.0, (bounds.0 - w).max(0.0));
            let ny = (y + d.1).clamp(0.0, (bounds.1 - h).max(0.0));
            self.crop_rect = Some((nx, ny, w, h));
            return true;
        }
        if let Some((handle, start)) = self.crop_resize {
            self.crop_rect = Some(selection::reshape(handle, start, p, bounds));
            return true;
        }
        let Some(rc) = &self.current else {
            return false;
        };
        let mut action = rc.borrow_mut();
        match action.tool {
            Tool::Pen | Tool::Highlight => {
                let points = Rc::make_mut(&mut action.points);
                let last = points.last().copied().unwrap_or(p);
                if (last.0 - p.0).hypot(last.1 - p.1) > 2.0 {
                    points.push(p);
                }
            }
            Tool::Line | Tool::Arrow | Tool::Ellipse | Tool::Rect | Tool::Blur => {
                let points = Rc::make_mut(&mut action.points);
                if points.len() < 2 {
                    points.push(p);
                } else {
                    points[1] = p;
                }
            }
            Tool::Select | Tool::Text | Tool::Crop | Tool::Counter => {}
        }
        true
    }

    /// A button's release, anywhere: it ends the stage drag that button
    /// holds. Returns whether the editor needs a redraw.
    fn stage_release(&mut self, button: MouseButton) -> bool {
        match button {
            MouseButton::Middle => self.pan_drag.take().is_some(),
            MouseButton::Left => {
                self.entry_drag = false;
                if self.pan_drag.take().is_some() {
                    return true;
                }
                let held = self.move_drag.is_some()
                    || self.crop_anchor.is_some()
                    || self.crop_move.is_some()
                    || self.crop_resize.is_some()
                    || self.current.is_some();
                // A press that selects a mark without moving it is no
                // edit: it adds no undo step.
                if let Some((i, _, old_action, start_bbox)) = self.move_drag.take() {
                    let new_action = self.actions.borrow().get(i).cloned();
                    if let Some(new_action) = new_action.filter(|a| a.bbox != Some(start_bbox)) {
                        let edit = Edit::Move(i, old_action, new_action);
                        self.rebuild_for_edit(&edit);
                        self.push_edit(edit);
                    }
                }
                self.crop_anchor = None;
                self.crop_move = None;
                self.crop_resize = None;
                self.commit_current();
                held
            }
            _ => false,
        }
    }
}
