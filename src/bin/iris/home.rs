//! Home: the launch surface, a welcome window. The app icon, the
//! "iris" title, and a tagline sit over three action tiles (New
//! Screenshot, Record Window, Library), each with a colored badge and
//! the keys that run it drawn as key caps; Settings is a gear in the
//! toolbar and the version sits in the footer. The window draws its
//! final layout on its first frame and requests no further frames:
//! hover and press are style states, not animations. Bare `iris`
//! launches here.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::*;

use crate::sys::hotkeys::{self, Binding, Chord};
use crate::{daemon, icons, theme, widgets};
use icons::Icon;
use iris_lib::config::Config;

const WIN: (f32, f32) = (600.0, 392.0);
/// The app icon's edge.
const ICON: f32 = 64.0;
const TILE: (f32, f32) = (168.0, 124.0);
const TILE_GAP: f32 = 12.0;
/// A tile badge's edge, corner radius, and glyph size.
const BADGE: f32 = 36.0;
const BADGE_RADIUS: f32 = 9.0;
const BADGE_GLYPH: f32 = 20.0;
/// The keyboard focus ring: its width (`border_2`) and its gap outside
/// the tile.
const RING: f32 = 2.0;
const RING_GAP: f32 = 2.0;

/// After a tile hides home, how long the capture waits for the
/// compositor to drop home's last pixels from the screen it grabs.
const VANISH: Duration = Duration::from_millis(34);
/// The longest a tile waits for home's empty frame: a window that gets
/// no frames (minimized mid-press) still runs the tile.
const VANISH_FALLBACK: Duration = Duration::from_millis(250);

/// What a home tile runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tile {
    Capture,
    Record,
    Library,
}

/// The home tiles, left to right: element id, glyph, badge color,
/// title, action.
const TILES: [(&str, Icon, Rgba, &str, Tile); 3] = [
    (
        "act-shot",
        Icon::Viewfinder,
        theme::ACCENT,
        "New Screenshot",
        Tile::Capture,
    ),
    (
        "act-rec",
        Icon::Record,
        theme::DANGER,
        "Record Window",
        Tile::Record,
    ),
    (
        "act-lib",
        Icon::Grid,
        theme::INDIGO,
        "Library",
        Tile::Library,
    ),
];

/// The library has no global hotkey; this in-window key opens it from
/// home (with Ctrl or Cmd), and its tile shows it.
const LIBRARY_KEY: &str = "l";
const LIBRARY_SHORTCUT: &str = crate::widgets::meta_shortcut!("L");
const SETTINGS_SHORTCUT: &str = crate::widgets::meta_shortcut!(",");

/// The line under a tile's title.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Caption {
    /// Keys that run the tile, drawn as key caps.
    Keys(SharedString),
    /// No chord is configured.
    NoShortcut,
    /// A chord is configured and does nothing: the keys and the reason.
    Unavailable {
        keys: SharedString,
        why: SharedString,
    },
}

impl Tile {
    fn command(self) -> daemon::Command {
        match self {
            Tile::Capture => daemon::Command::Capture,
            Tile::Record => daemon::Command::RecordToggle,
            Tile::Library => daemon::Command::Library,
        }
    }

    /// The global chord that runs the tile, if it has one.
    fn chord(self) -> Option<Chord> {
        match self {
            Tile::Capture => Some(Chord::Capture),
            Tile::Record => Some(Chord::Record),
            Tile::Library => None,
        }
    }

    /// Whether the tile reads the screen: home must be off it first.
    fn grabs(self) -> bool {
        matches!(self, Tile::Capture | Tile::Record)
    }

    /// The caption for the tile under `cfg`, given `binding`, the
    /// registration state of its configured chord (`None` while it is
    /// pending). A pending chord shows its keys: the regrab after a
    /// save lands within milliseconds.
    fn caption(self, cfg: &Config, binding: impl Fn(Chord, &str) -> Option<Binding>) -> Caption {
        let Some(chord) = self.chord() else {
            return Caption::Keys(LIBRARY_SHORTCUT.into());
        };
        let keys = chord.keys(cfg);
        if keys.is_empty() {
            return Caption::NoShortcut;
        }
        match binding(chord, keys) {
            Some(Binding::Unset) => Caption::NoShortcut,
            Some(Binding::Refused(why)) => Caption::Unavailable {
                keys: SharedString::from(keys.to_owned()),
                why: why.into(),
            },
            Some(Binding::Live) | None => Caption::Keys(SharedString::from(keys.to_owned())),
        }
    }
}

/// The tile keyboard selection lands on after one Tab or arrow step
/// from `from`, wrapping at both ends. With nothing selected, a forward
/// step selects the first tile and a backward step the last.
fn step(from: Option<usize>, forward: bool) -> usize {
    let n = TILES.len();
    match (from, forward) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
    }
}

pub struct Home {
    focus: FocusHandle,
    /// The tile Tab and the arrow keys selected; Return or Space runs
    /// it. None until a key moves it, so pointer use draws no ring.
    selected: Option<usize>,
    /// A tile that reads the screen ran: home draws nothing until it
    /// closes, so the capture never contains it.
    leaving: bool,
}

/// Open the home window, or raise the one already open.
pub fn open(cx: &mut App) -> Result<(), String> {
    if crate::widgets::raise_open::<Home>(cx, |h| !h.leaving) {
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
        |window, cx| {
            focus.focus(window, cx);
            cx.new(|_| Home {
                focus,
                selected: None,
                leaving: false,
            })
        },
    )
    .map_err(|e| format!("open home window: {e}"))?;
    Ok(())
}

/// Run `cmd` once the compositor has had `VANISH` to drop home.
fn run_after_vanish(cx: &mut App, cmd: daemon::Command) {
    cx.spawn(async move |cx| {
        cx.background_executor().timer(VANISH).await;
        cx.update(|cx| daemon::run(cx, &cmd));
    })
    .detach();
}

impl Home {
    /// Run `tile`. A tile that reads the screen first draws home empty
    /// and closes it, then starts once the compositor has dropped it, so
    /// the capture never contains home: a window closed in the same
    /// turn as the grab is still on the screen the grab reads, and a
    /// window manager's close animation keeps it there longer. The
    /// library opens at once.
    fn activate(&mut self, tile: Tile, window: &mut Window, cx: &mut Context<Self>) {
        if self.leaving {
            return;
        }
        if !tile.grabs() {
            window.remove_window();
            daemon::run(cx, &tile.command());
            return;
        }
        self.leaving = true;
        cx.notify();
        let pending = Rc::new(Cell::new(Some(tile.command())));
        // Next-frame callbacks run before that frame draws: the first
        // runs before the empty frame, the second after it presented.
        let on_frame = pending.clone();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                if let Some(cmd) = on_frame.take() {
                    window.remove_window();
                    run_after_vanish(cx, cmd);
                }
            });
        });
        let handle = window.window_handle();
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(VANISH_FALLBACK).await;
            cx.update(|cx| {
                if let Some(cmd) = pending.take() {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                    run_after_vanish(cx, cmd);
                }
            });
        })
        .detach();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.leaving {
            return;
        }
        let ks = &ev.keystroke;
        let meta = ks.modifiers.control || ks.modifiers.platform;
        match ks.key.as_str() {
            "escape" => window.remove_window(),
            "w" if meta => window.remove_window(),
            "," if meta => daemon::run(cx, &daemon::Command::Settings),
            k if meta && k == LIBRARY_KEY => self.activate(Tile::Library, window, cx),
            "tab" => {
                self.selected = Some(step(self.selected, !ks.modifiers.shift));
                cx.notify();
            }
            "right" | "left" => {
                self.selected = Some(step(self.selected, ks.key == "right"));
                cx.notify();
            }
            "enter" | "space" => {
                if let Some(ix) = self.selected {
                    self.activate(TILES[ix].4, window, cx);
                }
            }
            _ => {}
        }
    }
}

/// The app icon, drawn as vectors so it is sharp at every scale: the
/// white hexagonal aperture ring on a dark rounded square, as in the
/// packaged icon. Alternate blades at 78% give the ring its facets.
fn app_icon(edge: f32) -> Div {
    div()
        .w(px(edge))
        .h(px(edge))
        .rounded(px(edge * 0.24))
        .bg(linear_gradient(
            180.,
            linear_color_stop(theme::lighten(theme::BG_ELEV, 0.06), 0.),
            linear_color_stop(theme::BG_ELEV, 1.),
        ))
        .border_1()
        .border_color(theme::HAIRLINE)
        .shadow(theme::shadow_float())
        .child(
            canvas(
                move |_, _, _| (),
                move |bounds, (), window, _| {
                    let (ox, oy): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
                    let u: f32 = f32::from(bounds.size.width) / 64.0;
                    let c = (ox + 32.0 * u, oy + 32.0 * u);
                    let (outer, inner) = (24.0 * u, 16.0 * u);
                    let at = |r: f32, a: f32| point(px(c.0 + r * a.cos()), px(c.1 + r * a.sin()));
                    for pass in 0..2 {
                        let mut path = Path::new(point(px(c.0), px(c.1)));
                        for i in (pass..6).step_by(2) {
                            let a0 = i as f32 / 6.0 * std::f32::consts::TAU;
                            let a1 = (i + 1) as f32 / 6.0 * std::f32::consts::TAU;
                            icons::push_quad(
                                &mut path,
                                at(outer, a0),
                                at(outer, a1),
                                at(inner, a1),
                                at(inner, a0),
                            );
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

/// A tile's badge: the glyph in white on a rounded square of `color`,
/// lit slightly from the top.
fn badge(glyph: Icon, color: Rgba) -> Div {
    div()
        .w(px(BADGE))
        .h(px(BADGE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(BADGE_RADIUS))
        .bg(linear_gradient(
            180.,
            linear_color_stop(theme::lighten(color, 0.14), 0.),
            linear_color_stop(color, 1.),
        ))
        .shadow(vec![BoxShadow {
            color: theme::alpha(color, 0.30).into(),
            offset: point(px(0.), px(2.)),
            blur_radius: px(6.),
            spread_radius: px(0.),
            inset: false,
        }])
        .child(icons::icon(glyph, theme::ACCENT_INK, BADGE_GLYPH))
}

/// The caption line: key caps, or a note when the tile has no working
/// chord.
fn caption_line(caption: &Caption) -> AnyElement {
    let note = |text: &'static str| {
        div()
            .h(px(18.))
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(theme::TEXT_SMALL))
            .line_height(px(14.))
            .text_color(theme::FG_FAINT)
            .child(text)
    };
    match caption {
        Caption::Keys(keys) => widgets::keycaps(keys).into_any_element(),
        Caption::NoShortcut => note("No shortcut").into_any_element(),
        Caption::Unavailable { .. } => {
            let line = note("Shortcut unavailable");
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(icons::icon(Icon::Alert, theme::WIN_MIN, 12.))
                .child(line)
                .into_any_element()
        }
    }
}

/// One action tile: badge, title, and caption on a group card with
/// hover and press washes. `selected` draws the keyboard focus ring
/// outside the card, so the card's layout and fill never change with
/// it.
fn action_tile(
    id: &'static str,
    glyph: Icon,
    color: Rgba,
    title: &'static str,
    caption: &Caption,
    selected: bool,
) -> Stateful<Div> {
    let tile = div()
        .id(ElementId::Name(id.into()))
        .relative()
        .w(px(TILE.0))
        .h(px(TILE.1))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_GROUP))
        .bg(theme::GROUP_BG)
        .border_1()
        .border_color(theme::alpha(theme::FG, 0.06))
        .hover(|s| s.bg(theme::CARD_HOVER))
        .active(|s| s.bg(theme::CARD_PRESS))
        .cursor_pointer()
        .child(badge(glyph, color))
        .child(
            div()
                .mt(px(12.))
                .text_size(px(theme::TEXT_HEADLINE))
                .line_height(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::FG)
                .child(title),
        )
        .child(div().mt(px(6.)).child(caption_line(caption)))
        .children(selected.then(|| {
            let out = RING + RING_GAP;
            div()
                .absolute()
                .top(px(-out))
                .left(px(-out))
                .w(px(TILE.0 + 2.0 * out))
                .h(px(TILE.1 + 2.0 * out))
                .rounded(px(theme::RADIUS_GROUP + out))
                .border_2()
                .border_color(theme::FOCUS_RING)
        }));
    match caption {
        Caption::Unavailable { keys, why } => {
            tile.tooltip(widgets::tip(why.clone(), Some(keys.clone())))
        }
        _ => tile,
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.leaving {
            // The transparent window shows nothing while it closes.
            return div().size_full().into_any_element();
        }
        // Config::load serves a cached parse; the captions follow a
        // hotkey changed in Settings while home is open, and the daemon
        // redraws home when a regrab records a new state.
        let cfg = Config::load();
        let mut tiles = div().flex().gap(px(TILE_GAP));
        for (ix, &(id, glyph, color, title, act)) in TILES.iter().enumerate() {
            let caption = act.caption(&cfg, hotkeys::binding);
            tiles = tiles.child(
                action_tile(id, glyph, color, title, &caption, self.selected == Some(ix)).on_click(
                    cx.listener(move |this, _, window, cx| this.activate(act, window, cx)),
                ),
            );
        }
        let header = div()
            .flex()
            .flex_col()
            .items_center()
            .child(app_icon(ICON))
            .child(
                div()
                    .mt(px(14.))
                    .text_size(px(theme::TEXT_LARGE_TITLE))
                    .line_height(px(28.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme::FG)
                    .child("iris"),
            )
            .child(
                div()
                    .mt(px(2.))
                    .text_size(px(theme::TEXT_BODY))
                    .line_height(px(18.))
                    .text_color(theme::FG_DIM)
                    .child("Screenshots and screen recordings"),
            );
        let footer = div()
            .h(px(14.))
            .flex()
            .justify_center()
            .text_size(px(theme::TEXT_SMALL))
            .line_height(px(14.))
            .text_color(theme::FG_FAINT)
            .font_features(theme::tabular())
            .child(concat!("Version ", env!("CARGO_PKG_VERSION")));
        let content = div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(8.))
            .pb(px(16.))
            .child(header)
            .child(div().mt(px(28.)).child(tiles))
            .child(div().flex_1())
            .child(footer);
        let gear = widgets::icon_button(
            ElementId::Name("home-settings".into()),
            Icon::Gear,
            false,
            theme::TOOLBAR_CONTROL_H,
        )
        .tooltip(widgets::tip("Settings", Some(SETTINGS_SHORTCUT.into())))
        .on_click(cx.listener(|_, _, _, cx| daemon::run(cx, &daemon::Command::Settings)));
        widgets::toolbar_frame(
            window,
            widgets::Toolbar::default().trailing(gear),
            false,
            content,
        )
        .track_focus(&self.focus)
        .on_key_down(cx.listener(Self::on_key))
        .into_any_element()
    }
}

// WHY: the classes closed here are "a tile shows a shortcut other than
// the one that runs it, or one that does nothing" and "keyboard
// selection escapes the tiles". A hardcoded caption once read a default
// hotkey after the config changed it, and a chord another app held was
// captioned as if it worked; every tile's caption is derived from the
// config it runs under and the chord's registration state, and an
// emptied hotkey reads as no shortcut instead of a blank line. The
// selection steps through every tile in both directions and always
// lands on a tile Return can run. Only the tiles that read the screen
// hide home before they run. Not covered: the rendered layout and the
// frame timing of the hide, checked on docshots and the surfaces suite.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    fn keys(k: &str) -> Caption {
        Caption::Keys(SharedString::from(k.to_owned()))
    }

    #[test]
    fn captions_follow_the_configured_hotkeys_and_their_state() {
        let cfg = Config {
            capture_hotkey: "Super+P".into(),
            record_hotkey: " Ctrl+Alt+V ".into(),
            ..Config::default()
        };
        let live = |_: Chord, _: &str| Some(Binding::Live);
        let caps: Vec<Caption> = TILES.iter().map(|t| t.4.caption(&cfg, live)).collect();
        assert_eq!(
            caps,
            [keys("Super+P"), keys("Ctrl+Alt+V"), keys(LIBRARY_SHORTCUT)]
        );
        // A pending regrab still shows the keys.
        assert_eq!(Tile::Capture.caption(&cfg, |_, _| None), keys("Super+P"));
        // The state is looked up for the tile's own chord and keys.
        let taken = |c: Chord, k: &str| {
            (c == Chord::Record && k == "Ctrl+Alt+V")
                .then(|| Binding::Refused("In use by another app".into()))
                .or(Some(Binding::Live))
        };
        assert_eq!(Tile::Capture.caption(&cfg, taken), keys("Super+P"));
        assert_eq!(
            Tile::Record.caption(&cfg, taken),
            Caption::Unavailable {
                keys: "Ctrl+Alt+V".into(),
                why: "In use by another app".into(),
            }
        );
        assert_eq!(Tile::Library.caption(&cfg, taken), keys(LIBRARY_SHORTCUT));
        let cleared = Config {
            capture_hotkey: String::new(),
            record_hotkey: "  ".into(),
            ..Config::default()
        };
        assert_eq!(Tile::Capture.caption(&cleared, live), Caption::NoShortcut);
        assert_eq!(Tile::Record.caption(&cleared, live), Caption::NoShortcut);
    }

    #[test]
    fn every_tile_runs_its_own_command_and_only_screen_tiles_hide_home() {
        let got: Vec<String> = TILES
            .iter()
            .map(|t| format!("{:?}:{}", t.4.command(), t.4.grabs()))
            .collect();
        assert_eq!(got, ["Capture:true", "RecordToggle:true", "Library:false"]);
        let chords: Vec<Option<Chord>> = TILES.iter().map(|t| t.4.chord()).collect();
        assert_eq!(chords, [Some(Chord::Capture), Some(Chord::Record), None]);
    }

    #[test]
    fn selection_cycles_every_tile_in_both_directions() {
        let n = TILES.len();
        assert_eq!(step(None, true), 0);
        assert_eq!(step(None, false), n - 1);
        for forward in [true, false] {
            let mut seen = vec![false; n];
            let mut at = step(None, forward);
            for _ in 0..n {
                assert!(at < n, "selection {at} is past the last tile");
                seen[at] = true;
                at = step(Some(at), forward);
            }
            assert!(seen.iter().all(|&s| s), "forward={forward}: {seen:?}");
            // n steps return to the start.
            assert_eq!(at, step(None, forward));
        }
    }

    #[test]
    fn the_tiles_fit_the_window() {
        let n = TILES.len() as f32;
        let row = n * TILE.0 + (n - 1.0) * TILE_GAP;
        let margin = (WIN.0 - row) / 2.0;
        assert!(margin >= 32.0, "tile row leaves {margin}px side margins");
    }
}
