//! Home: the launch surface. An iris/aperture mark opens with a
//! spring — the brand moment — then the minimal home settles in:
//! two action tiles (New Screenshot, Library) centered, settings as
//! a small gear in the bottom-right corner. Bare `iris` launches
//! here; the daemon stays headless.

use std::time::{Duration, Instant};

use gpui::*;

use crate::{daemon, icons, motion, theme, widgets};
use icons::Icon;

const WIN: (f32, f32) = (560.0, 420.0);
/// Loading beat: the iris opens, a breath, then the content fades.
const LOADING: Duration = Duration::from_millis(1100);

pub struct Home {
    focus: FocusHandle,
    opened: Option<Instant>,
    hovered: Option<usize>,
    pressed: Option<usize>,
    /// This window's unique WM_CLASS, for the title-bar drag.
    class: String,
    /// Pointer-coupled springs per tile. They reverse mid-flight
    /// when the pointer leaves or the button releases early.
    hover: [motion::Spring; 2],
    press: [motion::Spring; 2],
    last_frame: Option<Instant>,
}

/// Open the home window.
pub fn open(cx: &mut App) -> Result<(), String> {
    let focus = cx.focus_handle();
    let origin = crate::xwin::centered_origin(cx, WIN.0, WIN.1, (200.0, 120.0));
    let win_id = crate::xwin::unique_id("dev.iris.home");
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(origin.0), px(origin.1)),
                size: size(px(WIN.0), px(WIN.1)),
            })),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_movable: true,
            is_resizable: false,
            is_minimizable: true,
            display_id: None,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some(win_id.clone()),
            window_min_size: Some(size(px(WIN.0), px(WIN.1))),
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
        },
        |_, cx| {
            cx.new(|_| Home {
                focus,
                opened: None,
                hovered: None,
                pressed: None,
                hover: [motion::Spring::default(); 2],
                press: [motion::Spring::default(); 2],
                last_frame: None,
                class: win_id.clone(),
            })
        },
    )
    .map_err(|e| format!("open home window: {e}"))?;
    // openbox-class WMs cascade Normal windows off-center and may
    // decorate them; re-place and strip.
    crate::xwin::place_after_map(win_id, origin.0, origin.1);
    Ok(())
}

/// The iris mark: six aperture blades opening around a growing
/// center hole. `t` 0..1 drives rotation and blade spread through
/// the shared spring; `size` is the tile edge in px.
fn iris_mark(t: f32, size: f32) -> Div {
    let e = motion::spring(t);
    div()
        .w(px(size))
        .h(px(size))
        .rounded(px(size * 0.24))
        .bg(theme::BG_ELEV)
        .shadow(theme::shadow_float())
        .child(
            canvas(
                move |_, _, _| (),
                move |bounds, (), window, _| {
                    let (ox, oy): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
                    let u: f32 = f32::from(bounds.size.width) / 64.0;
                    let c = (ox + 32.0 * u, oy + 32.0 * u);
                    let rot = (1.0 - e) * 0.9; // blades unwind ~51 degrees
                    let hole = (3.0 + 13.0 * e) * u; // center aperture opens
                    let r_out = 24.0 * u;
                    // Two passes: alternate blades at 78% give the
                    // pinwheel its definition.
                    for pass in 0..2 {
                        let mut path = Path::new(point(px(c.0), px(c.1)));
                        for i in (pass..6).step_by(2) {
                            let i = i as f32;
                            let a0 = i / 6.0 * std::f32::consts::TAU + rot;
                            let a1 = (i + 1.0) / 6.0 * std::f32::consts::TAU + rot;
                            let o0 = point(px(c.0 + r_out * a0.cos()), px(c.1 + r_out * a0.sin()));
                            let o1 = point(px(c.0 + r_out * a1.cos()), px(c.1 + r_out * a1.sin()));
                            let i0 = point(px(c.0 + hole * a0.cos()), px(c.1 + hole * a0.sin()));
                            let i1 = point(px(c.0 + hole * a1.cos()), px(c.1 + hole * a1.sin()));
                            icons::push_quad(&mut path, o0, o1, i1, i0);
                        }
                        let shade = if pass == 0 { theme::FG } else { theme::alpha(theme::FG, 0.78) };
                        window.paint_path(path, shade);
                    }
                },
            )
            .size_full(),
        )
}

/// One action tile: glyph over a label. `lift` (hover spring) grows
/// the shadow, `squish` (press spring) sinks the card and tightens
/// the shadow.
fn action_tile(
    id: &'static str,
    glyph: Icon,
    label: &'static str,
    enter: f32,
    lift: f32,
    squish: f32,
    hovered: bool,
) -> Stateful<Div> {
    let mut shadow = theme::shadow_rest();
    shadow.color.a *= 0.6 + 0.9 * lift;
    shadow.blur_radius = px((10. + 8.0 * lift) * (1.0 - 0.3 * squish));
    shadow.offset.y = px(2. + 4.0 * lift + 1.0 * squish);
    div()
        .id(ElementId::Name(id.into()))
        .w(px(152.))
        .h(px(104.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .rounded(px(14.))
        .bg(if squish > 0.05 {
            theme::SURFACE_PRESS
        } else if hovered {
            theme::SURFACE_HOVER
        } else {
            theme::GROUP_BG
        })
        .border_1()
        .border_color(theme::SEPARATOR)
        .shadow(vec![shadow])
        .opacity(enter)
        .mt(px((1.0 - motion::ease_out_cubic(enter)) * 12.0 + 1.0 * squish))
        .cursor_pointer()
        .child(icons::icon(glyph, theme::FG, 22.0))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::FG)
                .child(label),
        )
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus.focus(window);
        let opened = *self.opened.get_or_insert_with(Instant::now);
        let elapsed = opened.elapsed();
        let loading_t = (elapsed.as_secs_f32() / motion::tempo(LOADING).as_secs_f32()).min(1.0);

        // Advance the pointer-coupled springs by real frame delta and
        // keep the frame loop alive until everything settles.
        let now = Instant::now();
        let dt = self
            .last_frame
            .map(|t| (now - t).as_secs_f32())
            .unwrap_or(1.0 / 60.0);
        self.last_frame = Some(now);
        let mut live = loading_t < 1.0;
        for ix in 0..2 {
            let h_target = if self.hovered == Some(ix) { 1.0 } else { 0.0 };
            let p_target = if self.pressed == Some(ix) { 1.0 } else { 0.0 };
            self.hover[ix].to(h_target, dt);
            self.press[ix].to(p_target, dt);
            if !self.hover[ix].settled(h_target) || !self.press[ix].settled(p_target) {
                live = true;
            }
        }
        if live {
            window.request_animation_frame();
        }

        let mut content = div().flex_1().flex().items_center().justify_center();

        if loading_t < 0.72 {
            // The brand moment: the iris opens alone in the dark.
            content = content.child(iris_mark((loading_t / 0.72).min(1.0), 88.0));
        } else {
            // Content: mark, wordmark, tiles; each enters staggered.
            let content_t = ((loading_t - 0.72) / 0.28).clamp(0.0, 1.0);
            let stagger = |i: u32| {
                ((content_t - i as f32 * 0.18) / 0.64).clamp(0.0, 1.0)
            };
            if content_t < 1.0 {
                window.request_animation_frame();
            }
            let shot_enter = motion::ease_out_cubic(stagger(0));
            let lib_enter = motion::ease_out_cubic(stagger(1));
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(22.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(12.))
                            .opacity(shot_enter)
                            .child(iris_mark(1.0, 56.0))
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::FG)
                                    .child("Iris"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(14.))
                            .child(
                                action_tile(
                                    "act-shot",
                                    Icon::Viewfinder,
                                    "New Screenshot",
                                    shot_enter,
                                    self.hover[0].value,
                                    self.press[0].value,
                                    self.hovered == Some(0),
                                )
                                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                                    this.hovered = if *h { Some(0) } else { None };
                                    cx.notify();
                                }))
                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                    this.pressed = Some(0);
                                    cx.notify();
                                }))
                                .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                    this.pressed = None;
                                    cx.notify();
                                }))
                                .on_click(cx.listener(|_, _, window, cx| {
                                    window.remove_window();
                                    if let Err(e) =
                                        daemon::dispatch(cx, &daemon::Command::Capture)
                                    {
                                        eprintln!("iris: {e}");
                                    }
                                })),
                            )
                            .child(
                                action_tile(
                                    "act-lib",
                                    Icon::Grid,
                                    "Library",
                                    lib_enter,
                                    self.hover[1].value,
                                    self.press[1].value,
                                    self.hovered == Some(1),
                                )
                                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                                    this.hovered = if *h { Some(1) } else { None };
                                    cx.notify();
                                }))
                                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                    this.pressed = Some(1);
                                    cx.notify();
                                }))
                                .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                                    this.pressed = None;
                                    cx.notify();
                                }))
                                .on_click(cx.listener(|_, _, window, cx| {
                                    window.remove_window();
                                    if let Err(e) = crate::library::open(cx) {
                                        eprintln!("iris: {e}");
                                    }
                                })),
                            ),
                    ),
            );
        }

        // Settings: a small gear resting in the bottom-right corner,
        // appearing with the content.
        let mut frame = widgets::window_frame("Iris", self.class.clone(), vec![], content);
        if loading_t >= 0.72 {
            let gear_enter = (((loading_t - 0.72) / 0.28).clamp(0.0, 1.0) - 0.36).max(0.0) / 0.64;
            frame = frame.child(
                div()
                    .absolute()
                    .bottom(px(14.))
                    .right(px(14.))
                    .opacity(motion::ease_out_cubic(gear_enter.clamp(0.0, 1.0)))
                    .child(
                        widgets::icon_button("home-settings".into(), Icon::Gear, false, 30.0).on_click(
                            cx.listener(|_, _, _, cx| {
                                if let Err(e) = crate::settings::open(cx) {
                                    eprintln!("iris: {e}");
                                }
                            }),
                        ),
                    ),
            );
        }
        frame = frame.track_focus(&self.focus).on_key_down(
            cx.listener(|_, ev: &KeyDownEvent, window, _| {
                if ev.keystroke.key == "escape" {
                    window.remove_window();
                }
            }),
        );
        frame
    }
}
