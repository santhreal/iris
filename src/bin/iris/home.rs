//! Home: the launch surface. An iris/aperture mark opens with a
//! spring (the brand moment), then glides up over the wordmark while
//! the minimal home rises in: three action tiles (New Screenshot,
//! Record Window, Library) centered, settings as a small gear in the
//! bottom-right corner. The brand moment plays on the first home of a
//! daemon's lifetime; later opens start at the content reveal, so the
//! tiles are live on the first frame. Bare `iris` launches here; the
//! daemon stays headless.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui::*;

use crate::{daemon, icons, motion, theme, widgets};
use icons::Icon;

const WIN: (f32, f32) = (560.0, 420.0);
/// The entrance: the iris opens, then the content rises in under it.
const LOADING: Duration = Duration::from_millis(810);
/// Fraction of LOADING the brand mark plays alone. The content starts
/// as the mark's spring comes within 0.2% of rest, so no frame holds a
/// still mark between the two phases.
const REVEAL: f32 = 0.62;
/// Fraction of LOADING at which the mark's spring completes: one pace
/// across both phases.
const MARK_END: f32 = 0.98;
/// Set once the brand moment has played in this process.
static BRAND_PLAYED: AtomicBool = AtomicBool::new(false);

/// The mark's edge at rest over the wordmark, and in the brand moment.
const MARK: f32 = 56.0;
const BRAND_MARK: f32 = 88.0;
/// The content column, top to bottom: the mark, a gap, the wordmark's
/// line, a gap, the tile row. The wordmark's line height is set rather
/// than derived from the font, and the tiles' entrance offsets do not
/// take part in layout, so the column's height is this constant sum.
const HEADER_GAP: f32 = 12.0;
const WORDMARK_LINE: f32 = 28.0;
const COLUMN_GAP: f32 = 22.0;
const TILE: (f32, f32) = (152.0, 104.0);
/// How far the mark's rest center sits above the content center, where
/// the brand moment draws it: the column is centered and the mark tops it.
const MARK_RISE: f32 = (MARK + HEADER_GAP + WORDMARK_LINE + COLUMN_GAP + TILE.1) / 2.0 - MARK / 2.0;

/// Loading progress of a home opened `elapsed` ago at progress `start`
/// (0.0 plays the brand moment, REVEAL skips it). Saturates at exactly
/// 1.0, which is the frame loop's end condition.
fn loading_progress(start: f32, elapsed: Duration, total: Duration) -> f32 {
    (start + elapsed.as_secs_f32() / total.as_secs_f32()).min(1.0)
}

/// Content reveal progress: 0.0 up to REVEAL, exactly 1.0 when loading
/// completes. The span divides by itself at the end, so no rounding
/// can leave the reveal a hair short of done.
fn reveal(loading_t: f32) -> f32 {
    ((loading_t - REVEAL) / (1.0 - REVEAL)).clamp(0.0, 1.0)
}

/// The mark's edge and its drop below its rest slot at glide progress
/// `m`: 0.0 is the brand moment's pose, centered in the content area,
/// and 1.0 rests over the wordmark.
fn mark_pose(m: f32) -> (f32, f32) {
    (BRAND_MARK + (MARK - BRAND_MARK) * m, MARK_RISE * (1.0 - m))
}

/// What a home tile does when clicked.
#[derive(Clone, Copy)]
enum Tile {
    Capture,
    Record,
    Library,
}

/// The home tiles, left to right. The spring arrays on `Home` are
/// sized from this table.
const TILES: [(&str, Icon, &str, Tile); 3] = [
    (
        "act-shot",
        Icon::Viewfinder,
        "New Screenshot",
        Tile::Capture,
    ),
    ("act-rec", Icon::Record, "Record Window", Tile::Record),
    ("act-lib", Icon::Grid, "Library", Tile::Library),
];

pub struct Home {
    focus: FocusHandle,
    opened: Option<Instant>,
    /// Whether this window plays the brand moment (see BRAND_PLAYED).
    brand: bool,
    hovered: Option<usize>,
    pressed: Option<usize>,
    /// Pointer-coupled springs per tile. They reverse mid-flight
    /// when the pointer leaves or the button releases early.
    hover: [motion::Spring; TILES.len()],
    press: [motion::Spring; TILES.len()],
    last_frame: Option<Instant>,
}

/// Open the home window, or raise the one already open.
pub fn open(cx: &mut App) -> Result<(), String> {
    if crate::widgets::raise_open::<Home>(cx, |_| true) {
        return Ok(());
    }
    let focus = cx.focus_handle();
    let origin = crate::sys::window::centered_origin(cx, WIN.0, WIN.1, (200.0, 120.0));
    // Task switchers and taskbars list the window by its title; the
    // client-drawn frame shows its own.
    crate::widgets::open_window(
        cx,
        "iris",
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
            window_min_size: Some(size(px(WIN.0), px(WIN.1))),
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        },
        |_, cx| {
            cx.new(|_| Home {
                focus,
                opened: None,
                brand: !BRAND_PLAYED.swap(true, Ordering::Relaxed),
                hovered: None,
                pressed: None,
                hover: [motion::Spring::default(); TILES.len()],
                press: [motion::Spring::default(); TILES.len()],
                last_frame: None,
            })
        },
    )
    .map_err(|e| format!("open home window: {e}"))?;
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
                        let shade = if pass == 0 {
                            theme::FG
                        } else {
                            theme::alpha(theme::FG, 0.78)
                        };
                        window.paint_path(path, shade);
                    }
                },
            )
            .size_full(),
        )
}

/// One action tile: glyph over a label. `lift` (hover spring) grows
/// the shadow, `squish` (press spring) sinks the card and tightens
/// the shadow. The entrance rise and the press sink offset the drawn
/// tile only: as layout they would resize the row and move the column.
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
        .w(px(TILE.0))
        .h(px(TILE.1))
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
        .relative()
        .top(px(
            (1.0 - motion::ease_out_cubic(enter)) * 12.0 + 1.0 * squish
        ))
        .cursor_pointer()
        .child(icons::icon(glyph, theme::FG, 22.0))
        .child(
            div()
                .text_size(px(theme::TEXT_BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::FG)
                .child(label),
        )
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus.focus(window);
        let opened = *self.opened.get_or_insert_with(Instant::now);
        let start = if self.brand { 0.0 } else { REVEAL };
        let loading_t = loading_progress(start, opened.elapsed(), motion::tempo(LOADING));

        // Advance the pointer-coupled springs by real frame delta and
        // keep the frame loop alive until everything settles.
        let now = Instant::now();
        let dt = self
            .last_frame
            .map(|t| (now - t).as_secs_f32())
            .unwrap_or(1.0 / 60.0);
        self.last_frame = Some(now);
        let mut live = loading_t < 1.0;
        for ix in 0..TILES.len() {
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
        // The mark's spring keeps one pace across both phases; a later
        // open shows the mark at rest.
        let mark_t = if self.brand {
            (loading_t / MARK_END).min(1.0)
        } else {
            1.0
        };

        if loading_t < REVEAL {
            // The brand moment: the iris opens alone in the dark.
            content = content.child(iris_mark(mark_t, BRAND_MARK));
        } else {
            // Content: mark, wordmark, tiles; each enters staggered.
            // The frame loop runs on loading_t alone (`live` above).
            let content_t = reveal(loading_t);
            let stagger = |i: u32| ((content_t - i as f32 * 0.18) / 0.64).clamp(0.0, 1.0);
            let enters: [f32; TILES.len()] =
                std::array::from_fn(|i| motion::ease_out_cubic(stagger(i as u32)));
            // After the brand moment the mark glides from the center up
            // to its rest; a later open fades it in there.
            let (mark_size, mark_drop, mark_alpha) = if self.brand {
                let (size, drop) = mark_pose(enters[0]);
                (size, drop, 1.0)
            } else {
                (MARK, 0.0, enters[0])
            };
            let mut tiles = div().flex().gap(px(14.));
            for (ix, &(id, glyph, label, act)) in TILES.iter().enumerate() {
                tiles = tiles.child(
                    action_tile(
                        id,
                        glyph,
                        label,
                        enters[ix],
                        self.hover[ix].value,
                        self.press[ix].value,
                        self.hovered == Some(ix),
                    )
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                        this.hovered = if *h {
                            Some(ix)
                        } else {
                            this.hovered.filter(|&v| v != ix)
                        };
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.pressed = Some(ix);
                            cx.notify();
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.pressed = None;
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |_, _, window, cx| {
                        window.remove_window();
                        let cmd = match act {
                            Tile::Capture => daemon::Command::Capture,
                            Tile::Record => daemon::Command::RecordToggle,
                            Tile::Library => daemon::Command::Library,
                        };
                        daemon::run(cx, &cmd);
                    })),
                );
            }
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(COLUMN_GAP))
                    .child(
                        // Reversed, so the gliding mark paints over the
                        // wordmark it uncovers.
                        div()
                            .flex()
                            .flex_col_reverse()
                            .items_center()
                            .gap(px(HEADER_GAP))
                            .child(
                                div()
                                    .text_size(px(theme::TEXT_HEADING))
                                    .line_height(px(WORDMARK_LINE))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::FG)
                                    .opacity(enters[0])
                                    .child("iris"),
                            )
                            .child(
                                div().relative().size(px(MARK)).child(
                                    iris_mark(mark_t, mark_size)
                                        .absolute()
                                        .left(px((MARK - mark_size) / 2.0))
                                        .top(px((MARK - mark_size) / 2.0 + mark_drop))
                                        .opacity(mark_alpha),
                                ),
                            ),
                    )
                    .child(tiles),
            );
        }

        // No frame title: the wordmark below the toolbar names the window.
        let mut frame = widgets::window_frame("", false, vec![], content);
        // Settings: a small gear resting in the bottom-right corner,
        // appearing with the content.
        if loading_t >= REVEAL {
            let gear_enter = (reveal(loading_t) - 0.36).max(0.0) / 0.64;
            frame = frame.child(
                div()
                    .absolute()
                    .bottom(px(14.))
                    .right(px(14.))
                    .opacity(motion::ease_out_cubic(gear_enter.clamp(0.0, 1.0)))
                    .child(
                        widgets::icon_button(
                            ElementId::Name("home-settings".into()),
                            Icon::Gear,
                            false,
                            30.0,
                        )
                        .on_click(cx.listener(|_, _, _, cx| {
                            daemon::run(cx, &daemon::Command::Settings);
                        })),
                    ),
            );
        }
        frame = frame.track_focus(&self.focus).on_key_down(cx.listener(
            |_, ev: &KeyDownEvent, window, _| {
                if ev.keystroke.key == "escape" {
                    window.remove_window();
                }
            },
        ));
        frame
    }
}

// WHY: the classes closed here are "a settled surface keeps requesting
// frames" and "the entrance holds a still frame". The loop's end
// condition must be reachable exactly, so the home renders nothing
// once its entrance completes: a derived ratio (the reveal span over a
// separately written constant) once stopped at 0.9999999 and kept the
// window drawing every frame while open. And the content must start as
// the mark comes to rest: a reveal timed apart from the mark's spring
// once held a still mark on screen for 250 ms of a 1.1 s entrance. Not
// covered: the pointer springs, whose convergence motion.rs tests, and
// the glide's start pose matching the brand pose, which depends on
// layout and is checked on rendered frames.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    #[test]
    fn loading_reaches_exactly_one_from_either_start() {
        for start in [0.0, REVEAL] {
            for extra_ms in [0, 1, 16, 5_000] {
                let t = loading_progress(start, LOADING + Duration::from_millis(extra_ms), LOADING);
                assert_eq!(t, 1.0, "start {start}, +{extra_ms}ms");
            }
        }
    }

    #[test]
    fn reveal_is_exactly_done_when_loading_is() {
        assert_eq!(reveal(1.0), 1.0);
        assert_eq!(reveal(REVEAL), 0.0);
        assert_eq!(reveal(0.0), 0.0);
        // Monotonic across the content phase, never past its ends.
        let mut last = 0.0;
        for i in 0..=1000 {
            let r = reveal(REVEAL + (1.0 - REVEAL) * i as f32 / 1000.0);
            assert!((0.0..=1.0).contains(&r) && r >= last, "step {i}: {r}");
            last = r;
        }
    }

    #[test]
    fn warm_open_starts_at_the_reveal() {
        // A later home opens with the tiles already entering: the
        // first frame is past the brand-only phase.
        let t = loading_progress(REVEAL, Duration::ZERO, LOADING);
        assert!(t >= REVEAL && reveal(t) == 0.0);
        let t = loading_progress(REVEAL, Duration::from_millis(16), LOADING);
        assert!(reveal(t) > 0.0);
    }

    #[test]
    fn the_content_starts_as_the_mark_comes_to_rest() {
        // The mark stops moving on screen within 0.1% of rest.
        let off_rest = |loading_t: f32| (1.0 - motion::spring(loading_t / MARK_END)).abs();
        let still = (0..=10_000)
            .map(|i| i as f32 / 10_000.0)
            .rev()
            .take_while(|&t| off_rest(t) < 0.001)
            .last()
            .expect("the mark comes to rest");
        let frame = 1.0 / 60.0 / LOADING.as_secs_f32();
        assert!(
            (still - REVEAL).abs() < frame,
            "the mark rests at {still}, the content starts at {REVEAL}: \
             more than a frame apart"
        );
    }
}
