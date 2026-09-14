//! Toast stage: the macOS floating screenshot thumbnail.
//!
//! The particulars this matches: raw capture pixels only, aspect-true,
//! no chrome of any kind (no border stroke, no inner highlight, no
//! buttons, no labels, no hover or pressed state). ~200px long edge,
//! 9px corners, one soft deep shadow. It slides in from beyond the
//! right edge in ~450ms with a hard deceleration and stops dead: no
//! overshoot, no fade, no scale. It sits visually inert for ~5s —
//! hovering only suspends the clock — then accelerates back off the
//! right edge, fading in the last third. A rightward flick dismisses
//! it 1:1 under the pointer; any other drag is a file drag through
//! XDnD. Click morphs into Markup. A new capture replaces it.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use gpui::*;

use crate::{motion, theme};

const MAX_W: f32 = 200.0;
const MAX_H: f32 = 140.0;
const MARGIN: f32 = 12.0;
const BLEED: f32 = 44.0;
const RADIUS: f32 = 12.0;
const SIT: Duration = Duration::from_millis(5000);
const ENTER: Duration = motion::ENTER;
const EXIT: Duration = Duration::from_millis(300);
/// Replaced by a newer capture: a fast fade, not the full fly-off.
const REPLACE_EXIT: Duration = Duration::from_millis(180);
const SWIPE_RETURN: Duration = Duration::from_millis(260);
const MENU_FADE: Duration = motion::FADE;
/// Safety net for the editor-morph handshake: if the editor never
/// renders its first frame, the toast gives up waiting and closes.
const MORPH_WAIT: Duration = Duration::from_millis(2000);
/// How long the toast lingers after the editor's first render: a
/// rendered frame still has map and compositor latency ahead of it.
/// While it lingers, the editor is still transparent and the toast's
/// pixels show through as the morph's opening frame.
const PRESENT_GRACE: Duration = Duration::from_millis(60);
/// A velocity sample older than this at release is a stopped finger,
/// not a flick.
const SWIPE_STALE: Duration = Duration::from_millis(120);
/// Past this travel, or released faster than this velocity, a
/// rightward swipe dismisses instead of springing back.
const SWIPE_TRAVEL: f32 = 100.0;
const SWIPE_VELOCITY: f32 = 400.0;

/// The live toast, if any. A new capture replaces the floating
/// thumbnail, like macOS: the previous one fades out fast.
static TOAST_HANDLE: parking_lot::Mutex<Option<WindowHandle<ToastStage>>> =
    parking_lot::Mutex::new(None);

pub struct ToastStage {
    path: PathBuf,
    thumb: Arc<RenderImage>,
    thumb_rgba: Arc<Vec<u8>>,
    dims: (f32, f32),
    card_screen: (f32, f32, f32, f32),
    opened: Option<Instant>,
    hover_paused: bool,
    closing_at: Option<Instant>,
    /// Rightward offset the exit flight starts from, when a swipe
    /// carried the card before the dismiss.
    exit_from: f32,
    drag_start: Option<(f32, f32)>,
    gesture: Gesture,
    swipe: Option<Swipe>,
    swipe_return: Option<(Instant, f32)>,
    /// Right-click context menu position in window coordinates.
    menu_at: Option<(f32, f32)>,
    /// Menu entrance clock; the menu fades in and rises 3px.
    menu_opened: Option<Instant>,
    /// A left press that only dismissed the open menu is not a click.
    swallow_click: bool,
    /// Editor-morph handshake: the toast stays put until the editor
    /// has painted the image over the exact same pixels, so the
    /// handoff has no blank blink. The instant bounds the wait.
    pending_morph: Option<(Arc<AtomicBool>, Instant)>,
    /// When the editor's first render was seen; the toast lingers
    /// PRESENT_GRACE past it because a rendered frame is not yet a
    /// presented one: X11 map and compositor latency would otherwise
    /// open a black gap between the two windows.
    morph_ready_at: Option<Instant>,
    /// Exit duration: full fly-off normally, a fast fade on replace.
    closing_dur: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Gesture {
    Undecided,
    Swipe,
    FileDrag,
}

/// A rightward dismiss swipe in progress: 1:1 travel under the
/// pointer plus a smoothed velocity for the release decision.
struct Swipe {
    dx: f32,
    vel: f32,
    last_at: Instant,
    last_dx: f32,
}

/// Decode and pre-scale the thumbnail to its exact display size with a
/// high-quality filter: no resampling happens per frame afterwards.
fn prepare_thumb(
    path: &Path,
) -> Result<(Arc<RenderImage>, Arc<Vec<u8>>, (f32, f32)), String> {
    let img = image::open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?
        .to_rgba8();
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let scale = (MAX_W / iw).min(MAX_H / ih).min(1.0);
    let (w, h) = ((iw * scale).round().max(1.0), (ih * scale).round().max(1.0));
    let rgba = if scale < 1.0 {
        image::imageops::resize(&img, w as u32, h as u32, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    // RenderImage directly from the resized pixels: no PNG
    // re-encode, and the atlas tile stays freeable on dismiss.
    let render = crate::widgets::render_image_from_rgba(rgba.width(), rgba.height(), rgba.as_raw());
    Ok((render, Arc::new(rgba.into_raw()), (w, h)))
}

impl ToastStage {
    pub fn new(path: &Path, thumb: &Path) -> Result<Self, String> {
        let (thumb, thumb_rgba, dims) = prepare_thumb(thumb)?;
        Ok(Self {
            path: path.to_path_buf(),
            thumb,
            thumb_rgba,
            dims,
            card_screen: (0.0, 0.0, 0.0, 0.0),
            opened: None,
            hover_paused: false,
            closing_at: None,
            exit_from: 0.0,
            drag_start: None,
            gesture: Gesture::Undecided,
            swipe: None,
            swipe_return: None,
            menu_at: None,
            menu_opened: None,
            swallow_click: false,
            pending_morph: None,
            morph_ready_at: None,
            closing_dur: EXIT,
        })
    }

    /// Start the auto-dismiss countdown. When it fires, hovering pushes
    /// the deadline out instead of dismissing under the pointer.
    fn arm_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.closing_at.is_some() {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SIT).await;
            this.update(cx, |stage, cx| {
                if stage.hover_paused || stage.menu_at.is_some() {
                    stage.arm_dismiss(cx);
                } else {
                    stage.begin_close(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn begin_close(&mut self, cx: &mut Context<Self>) {
        if self.closing_at.is_some() || self.pending_morph.is_some() {
            return;
        }
        self.closing_at = Some(Instant::now());
        cx.notify();
    }

    /// Replaced by a newer capture: fade out fast from wherever the
    /// card sits. The full fly-off would read as two competing exits.
    fn fade_out_quick(&mut self, cx: &mut Context<Self>) {
        self.closing_dur = REPLACE_EXIT;
        self.begin_close(cx);
    }

    /// Open the markup editor over this toast and wait for its first
    /// frame before leaving: the editor paints the same image over the
    /// exact same screen pixels, so the morph has no blank blink.
    fn hand_off_to_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ready = Arc::new(AtomicBool::new(false));
        let result = crate::editor::open(
            cx,
            &self.path,
            Some(self.card_screen),
            Some(ready.clone()),
        );
        match result {
            Ok(()) => {
                self.pending_morph = Some((ready, Instant::now()));
                cx.notify();
            }
            Err(e) => {
                eprintln!("annotate: {e}");
                crate::widgets::release_render(&self.thumb, cx);
                window.remove_window();
            }
        }
    }

    /// Resolve a released dismiss swipe: fast or far enough flies the
    /// card off from its carried offset; otherwise it springs back to
    /// the corner. No-op once the swipe state is consumed.
    fn release_swipe(&mut self, cx: &mut Context<Self>) {
        let Some(mut s) = self.swipe.take() else {
            return;
        };
        // A finger that stopped moving before release is not a flick:
        // decay the smoothed velocity by the sample's age.
        if s.last_at.elapsed() > SWIPE_STALE {
            s.vel = 0.0;
        }
        if s.vel > SWIPE_VELOCITY || s.dx > SWIPE_TRAVEL {
            self.exit_from = s.dx;
            self.begin_close(cx);
        } else {
            self.swipe_return = Some((Instant::now(), s.dx));
        }
        cx.notify();
    }
}

/// One soft, deep shadow: the only thing separating the thumbnail
/// from the desktop. No contact layer, no hairline. During the
/// entrance the shadow fades in with the card's visible fraction, so
/// the blur never arrives ahead of the pixels casting it.
fn card_shadow(visibility: f32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.36 * visibility),
            offset: point(px(0.), px(12.)),
            blur_radius: px(32.),
            spread_radius: px(0.),
        },
        BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.20 * visibility),
            offset: point(px(0.), px(2.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
        },
    ]
}

impl Render for ToastStage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = self.dims;

        // Entrance: the card comes from beyond the screen's right
        // edge moving left, decelerating hard into the corner, and
        // stops dead. No overshoot, no fade, no scale. The clock
        // starts at first render: window mapping latency would
        // otherwise eat it. `right` is the distance from the window's
        // right edge; smaller values carry the card rightward.
        let opened = *self.opened.get_or_insert_with(Instant::now);
        let enter_t = (opened.elapsed().as_secs_f32() / motion::tempo(ENTER).as_secs_f32()).min(1.0);
        let ease = 1.0 - (1.0 - enter_t).powi(4);
        let mut right = MARGIN - (1.0 - ease) * (w + 2.0 * MARGIN);
        let mut opacity = 1.0f32;
        let shadow_vis = ease;
        let mut animating = enter_t < 1.0;

        // Dismiss swipe: the card tracks the pointer 1:1 rightward.
        if let Some(s) = &self.swipe {
            right -= s.dx;
        }
        // A short swipe released: spring back to the corner.
        if let Some((started, dx0)) = self.swipe_return {
            let t = (started.elapsed().as_secs_f32()
                / motion::tempo(SWIPE_RETURN).as_secs_f32())
            .min(1.0);
            right -= dx0 * (1.0 - motion::spring(t));
            if t >= 1.0 {
                self.swipe_return = None;
            } else {
                animating = true;
            }
        }

        // Exit: accelerate off the right edge from wherever the card
        // sits, fade in the last third, then the window goes away.
        if let Some(started) = self.closing_at {
            let t = (started.elapsed().as_secs_f32() / motion::tempo(self.closing_dur).as_secs_f32()).min(1.0);
            // A replace-fade stays in place and dissolves over the
            // whole run; the full exit flies and fades in its last
            // third.
            if self.closing_dur > REPLACE_EXIT {
                right = MARGIN - self.exit_from - (w + 2.0 * MARGIN + 24.0) * t * t;
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

        // Editor-morph handshake: hold the resting position, painted
        // over by the editor's first frame, then leave without any
        // exit animation. Snap to rest: the editor morphs from the
        // resting rect, so a click mid-entrance must not drift.
        if let Some((ready, since)) = &self.pending_morph {
            right = MARGIN;
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

        // Right-click context menu, macOS style: Markup, Copy, Delete,
        // Close. Fades in over 120ms rising 3px; shares the exit
        // opacity so it never outlives the card it belongs to.
        let mut menu_el = None;
        if let Some((menu_x, menu_y)) = self.menu_at {
            let opened = *self.menu_opened.get_or_insert_with(Instant::now);
            let mt = (opened.elapsed().as_secs_f32()
                / motion::tempo(MENU_FADE).as_secs_f32())
            .min(1.0);
            if mt < 1.0 {
                window.request_animation_frame();
            }
            let mut menu = crate::widgets::menu()
                .absolute()
                .left(px(menu_x))
                .top(px(menu_y + 3.0 * (1.0 - motion::ease_out_cubic(mt))))
                .opacity(mt * opacity);
            menu = menu.child(
                crate::widgets::menu_row("toast-markup", "Markup")
                    .on_click(cx.listener(|stage, _, window, cx| {
                        cx.stop_propagation();
                        stage.menu_at = None;
                        stage.hand_off_to_editor(window, cx);
                    })),
            );
            menu = menu.child(
                crate::widgets::menu_row("toast-copy", "Copy")
                    .on_click(cx.listener(|stage, _, _, cx| {
                        cx.stop_propagation();
                        stage.menu_at = None;
                        if let Err(e) = crate::pipeline::copy_image_file(&stage.path) {
                            eprintln!("copy: {e}");
                        }
                        cx.notify();
                    })),
            );
            menu = menu.child(
                crate::widgets::menu_row("toast-delete", "Delete")
                    .on_click(cx.listener(|stage, _, _, cx| {
                        cx.stop_propagation();
                        stage.menu_at = None;
                        match iris_lib::library::delete(&stage.path) {
                            // Only the honest exit flies off: a failed
                            // delete leaves the card, so nothing
                            // pretends to have happened.
                            Ok(()) => stage.begin_close(cx),
                            Err(e) => eprintln!("delete: {e}"),
                        }
                        cx.notify();
                    })),
            );
            menu = menu.child(
                crate::widgets::menu_row("toast-close", "Close")
                    .on_click(cx.listener(|stage, _, _, cx| {
                        cx.stop_propagation();
                        stage.menu_at = None;
                        stage.begin_close(cx);
                    })),
            );
            menu_el = Some(menu);
        }

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
                        let (px_, py_): (f32, f32) =
                            (ev.position.x.into(), ev.position.y.into());
                        let menu_h = 4.0 * crate::widgets::MENU_ROW_H + 10.0;
                        let inside = px_ >= mx && px_ <= mx + crate::widgets::MENU_W
                            && py_ >= my && py_ <= my + menu_h;
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
                let (mx, my): (f32, f32) =
                    (ev.position.x.into(), ev.position.y.into());
                let (dx, dy) = (mx - sx, my - sy);
                if stage.gesture == Gesture::Swipe {
                    let now = Instant::now();
                    if let Some(s) = &mut stage.swipe {
                        let dt =
                            now.duration_since(s.last_at).as_secs_f32().max(0.001);
                        let v = (dx - s.last_dx) / dt;
                        s.vel = 0.65 * s.vel + 0.35 * v;
                        s.last_at = now;
                        s.last_dx = dx;
                        s.dx = dx.max(0.0);
                    }
                    cx.notify();
                    return;
                }
                if dx.hypot(dy) < 8.0 {
                    return;
                }
                if dx > 6.0 && dx.abs() > 2.0 * dy.abs() {
                    // Dominantly rightward: a dismiss swipe.
                    stage.gesture = Gesture::Swipe;
                    stage.swipe = Some(Swipe {
                        dx: dx.max(0.0),
                        vel: 0.0,
                        last_at: Instant::now(),
                        last_dx: dx,
                    });
                } else {
                    // Any other direction: a file drag. The card
                    // itself stays put, like macOS: dropping a copy
                    // does not consume the notification, and a drag
                    // that lands nowhere loses nothing.
                    stage.gesture = Gesture::FileDrag;
                    let icon = iris_lib::dragcopy::DragIcon {
                        width: stage.dims.0 as u32,
                        height: stage.dims.1 as u32,
                        rgba: (*stage.thumb_rgba).clone(),
                    };
                    if let Err(e) = iris_lib::dragcopy::start_file_drag_at_cursor(
                        vec![stage.path.clone()],
                        Some(icon),
                    ) {
                        eprintln!("drag: {e}");
                    }
                }
                cx.notify();
            }))
            .child(
                div()
                    .id("card")
                    .absolute()
                    .bottom(px(MARGIN))
                    .right(px(right))
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(RADIUS))
                    .overflow_hidden()
                    .shadow(card_shadow(shadow_vis))
                    .opacity(opacity)
                    .child(
                        div().absolute().top_0().left_0().size_full().child(
                            img(ImageSource::Render(thumb))
                                .size_full()
                                .object_fit(ObjectFit::Fill)
                                .rounded(px(RADIUS)),
                        ),
                    )
                    .on_hover(cx.listener(|stage, hovering, _window, cx| {
                        // Hover only suspends the dismiss clock. The
                        // thumbnail itself stays visually inert.
                        stage.hover_paused = *hovering;
                        if !*hovering {
                            stage.arm_dismiss(cx);
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|stage, ev: &MouseDownEvent, _, cx| {
                            stage.drag_start =
                                Some((ev.position.x.into(), ev.position.y.into()));
                            stage.gesture = Gesture::Undecided;
                            stage.swipe_return = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|stage, ev: &MouseDownEvent, _, cx| {
                            // Store the clamped origin: the render and
                            // the outside-press hit test must agree on
                            // where the menu actually is.
                            let menu_h = 4.0 * crate::widgets::MENU_ROW_H + 10.0;
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
                    .on_click(cx.listener(|stage, _, window, cx| {
                        match stage.gesture {
                            // A gesture that ended on the card is not a
                            // click. GPUI may dispatch click instead of
                            // mouse-up, so the swipe resolves here too.
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
                                stage.hand_off_to_editor(window, cx);
                            }
                        }
                    })),
            )
            .children(menu_el)
    }
}

/// The toast card's resting rect on a display of `disp_w`x`disp_h`
/// logical px, for a capture of `img_w`x`img_h`. The overlay's
/// capture flight lands exactly here, so the toast must not replay
/// its entrance.
pub fn card_rest_rect(disp_w: f32, disp_h: f32, img_w: u32, img_h: u32) -> (f32, f32, f32, f32) {
    let (iw, ih) = (img_w as f32, img_h as f32);
    let scale = (MAX_W / iw).min(MAX_H / ih).min(1.0);
    let w = (iw * scale).round().max(1.0);
    let h = (ih * scale).round().max(1.0);
    (disp_w - MARGIN - w, disp_h - MARGIN - h, w, h)
}

/// Open the toast stage window in the bottom-right corner of the primary
/// display. The card image paths must already exist on disk.
pub fn show_toast(
    cx: &mut App,
    path: &Path,
    thumb: &Path,
    width: u32,
    height: u32,
) -> Result<(), String> {
    show_toast_kind(cx, path, thumb, width, height, false, None)
}

/// The capture-flight landing: the card appears at rest, no entrance.
/// `anchor` is the committing monitor's rect in screen coordinates, so
/// the card lands where the flight ended, not on another display.
pub fn show_toast_landed(
    cx: &mut App,
    path: &Path,
    thumb: &Path,
    width: u32,
    height: u32,
    anchor: Option<(f32, f32, f32, f32)>,
) -> Result<(), String> {
    show_toast_kind(cx, path, thumb, width, height, true, anchor)
}

fn show_toast_kind(
    cx: &mut App,
    path: &Path,
    thumb: &Path,
    width: u32,
    height: u32,
    landed: bool,
    anchor: Option<(f32, f32, f32, f32)>,
) -> Result<(), String> {
    let _ = (width, height);
    let mut stage = ToastStage::new(path, thumb)?;
    if landed {
        // Pretend the entrance finished long ago.
        stage.opened = Some(Instant::now() - motion::tempo(ENTER));
    }
    let (w, h) = stage.dims;

    let win_size = size(px(w + BLEED + MARGIN), px(h + BLEED + MARGIN));
    // The toast anchors bottom-right of the committing monitor, or of
    // the primary monitor for captures with no overlay. Wayland
    // compositors ignore client-side toplevel positions; the zero
    // origin is a placeholder there.
    let screen = anchor.or_else(|| crate::xwin::primary_monitor_rect(cx));
    let origin = match screen {
        Some((sx, sy, sw, sh)) => point(
            px(sx) + px(sw) - win_size.width,
            px(sy) + px(sh) - win_size.height,
        ),
        None => point(px(0.), px(0.)),
    };
    // The card's rect in screen coordinates, for the editor morph. The
    // card rests MARGIN from the window's bottom-right corner.
    let (ox, oy): (f32, f32) = (origin.x.into(), origin.y.into());
    stage.card_screen = (
        ox + f32::from(win_size.width) - MARGIN - w,
        oy + f32::from(win_size.height) - MARGIN - h,
        w,
        h,
    );

    let win_id = crate::xwin::unique_id("dev.iris.toast");
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin,
                    size: win_size,
                })),
                titlebar: None,
                focus: false,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                display_id: None,
                window_background: WindowBackgroundAppearance::Transparent,
                app_id: Some(win_id.clone()),
                window_min_size: None,
                window_decorations: Some(WindowDecorations::Client),
                tabbing_identifier: None,
            },
            |_, cx| cx.new(|_| stage),
        )
        .map_err(|e| format!("open toast window: {e}"))?;

    if let Some(previous) = TOAST_HANDLE.lock().replace(handle.clone()) {
        let _ = previous.update(cx, |stage, _window, cx| {
            stage.fade_out_quick(cx);
        });
    }

    handle
        .update(cx, |stage: &mut ToastStage, _window, cx| {
            stage.arm_dismiss(cx);
        })
        .map_err(|e| format!("arm dismiss: {e}"))?;

    // openbox-style WMs apply their own placement at map time; put the
    // window back where the corner is. No-op once the WM honors the
    // requested origin (mutter, KWin).
    let (ox, oy): (f32, f32) = (origin.x.into(), origin.y.into());
    crate::xwin::place_after_map_kind(win_id, ox, oy, true);
    Ok(())
}
