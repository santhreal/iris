//! Home: the launch surface, a welcome window. The app icon, the
//! "iris" title, and the running version sit over three action tiles
//! (New Screenshot, Record Window, Library), each captioned with the
//! shortcut that runs it; Settings is a gear in the toolbar. The window
//! draws its final layout on its first frame and requests no further
//! frames: hover and press are style states, not animations. Bare
//! `iris` launches here; the daemon stays headless.

use gpui::*;

use crate::{daemon, icons, theme, widgets};
use icons::Icon;
use iris_lib::config::Config;

const WIN: (f32, f32) = (560.0, 380.0);
/// The app icon's edge.
const ICON: f32 = 64.0;
const TILE: (f32, f32) = (152.0, 112.0);
const TILE_GAP: f32 = 12.0;

/// What a home tile runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tile {
    Capture,
    Record,
    Library,
}

/// The home tiles, left to right: element id, glyph, title, action.
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

/// The library has no global hotkey; this in-window key opens it from
/// home (with Ctrl or Cmd), and its tile shows it.
const LIBRARY_KEY: &str = "l";
const LIBRARY_SHORTCUT: &str = "Ctrl+L";
const SETTINGS_SHORTCUT: &str = "Ctrl+,";

impl Tile {
    fn command(self) -> daemon::Command {
        match self {
            Tile::Capture => daemon::Command::Capture,
            Tile::Record => daemon::Command::RecordToggle,
            Tile::Library => daemon::Command::Library,
        }
    }

    /// The caption under the tile's title: the configured global hotkey
    /// for a capture or a recording, the in-window key for the library.
    fn shortcut(self, cfg: &Config) -> SharedString {
        let keys = match self {
            Tile::Capture => cfg.capture_hotkey.trim(),
            Tile::Record => cfg.record_hotkey.trim(),
            Tile::Library => LIBRARY_SHORTCUT,
        };
        if keys.is_empty() {
            "No shortcut".into()
        } else {
            SharedString::from(keys.to_owned())
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
        |window, cx| {
            focus.focus(window, cx);
            cx.new(|_| Home {
                focus,
                selected: None,
            })
        },
    )
    .map_err(|e| format!("open home window: {e}"))?;
    Ok(())
}

/// Close home and run `tile`. The window closes first, so a capture
/// never contains it.
fn activate(tile: Tile, window: &mut Window, cx: &mut App) {
    window.remove_window();
    daemon::run(cx, &tile.command());
}

/// The app icon, drawn as vectors so it is sharp at every scale: the
/// white hexagonal aperture ring on a dark rounded square, as in the
/// packaged icon. Alternate blades at 78% give the ring its facets.
fn app_icon(edge: f32) -> Div {
    div()
        .w(px(edge))
        .h(px(edge))
        .rounded(px(edge * 0.24))
        .bg(theme::BG_ELEV)
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

/// One action tile: glyph, title, and the shortcut caption on a group
/// card with hover and press washes. `selected` draws the focus ring
/// as the border: the card fill is translucent, so a ring drawn as a
/// shadow under it would tint the whole card.
fn action_tile(
    id: &'static str,
    glyph: Icon,
    title: &'static str,
    shortcut: SharedString,
    selected: bool,
) -> Stateful<Div> {
    div()
        .id(ElementId::Name(id.into()))
        .w(px(TILE.0))
        .h(px(TILE.1))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .rounded(px(theme::RADIUS_GROUP))
        .border_2()
        .border_color(if selected {
            theme::ACCENT
        } else {
            theme::alpha(theme::ACCENT, 0.0)
        })
        .bg(theme::GROUP_BG)
        .hover(|s| s.bg(theme::CARD_HOVER))
        .active(|s| s.bg(theme::CARD_PRESS))
        .cursor_pointer()
        .child(icons::icon(glyph, theme::FG, 24.0))
        .child(
            div()
                .mt(px(12.))
                .text_size(px(theme::TEXT_BODY))
                .line_height(px(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::FG)
                .child(title),
        )
        .child(
            div()
                .mt(px(2.))
                .text_size(px(theme::TEXT_SMALL))
                .line_height(px(14.))
                .text_color(theme::FG_DIM)
                .font_features(theme::tabular())
                .child(shortcut),
        )
}

impl Home {
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let meta = ks.modifiers.control || ks.modifiers.platform;
        match ks.key.as_str() {
            "escape" => window.remove_window(),
            "w" if meta => window.remove_window(),
            "," if meta => daemon::run(cx, &daemon::Command::Settings),
            k if meta && k == LIBRARY_KEY => activate(Tile::Library, window, cx),
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
                    activate(TILES[ix].3, window, cx);
                }
            }
            _ => {}
        }
    }
}

impl Render for Home {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Config::load serves a cached parse; the captions follow a
        // hotkey changed in Settings while home is open.
        let cfg = Config::load();
        let mut tiles = div().flex().gap(px(TILE_GAP));
        for (ix, &(id, glyph, title, act)) in TILES.iter().enumerate() {
            tiles = tiles.child(
                action_tile(
                    id,
                    glyph,
                    title,
                    act.shortcut(&cfg),
                    self.selected == Some(ix),
                )
                .on_click(cx.listener(move |_, _, window, cx| activate(act, window, cx))),
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
                    .text_size(px(theme::TEXT_SMALL))
                    .line_height(px(14.))
                    .text_color(theme::FG_DIM)
                    .font_features(theme::tabular())
                    .child(concat!("Version ", env!("CARGO_PKG_VERSION"))),
            );
        // The column centers in the band under the toolbar, with equal
        // space above the icon and below the tiles.
        let content = div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(28.))
            .child(header)
            .child(tiles);
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
    }
}

// WHY: the classes closed here are "a tile shows a shortcut other than
// the one that runs it" and "keyboard selection escapes the tiles". A
// hardcoded caption once read a default hotkey after the config changed
// it; every tile's caption is derived from the config it runs under, and
// an emptied hotkey reads as no shortcut instead of a blank line. The
// selection steps through every tile in both directions and always lands
// on a tile Return can run. Not covered: the rendered layout, which is
// checked on docshots.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    #[test]
    fn captions_follow_the_configured_hotkeys() {
        let cfg = Config {
            capture_hotkey: "Super+P".into(),
            record_hotkey: " Ctrl+Alt+V ".into(),
            ..Config::default()
        };
        let caps: Vec<SharedString> = TILES.iter().map(|t| t.3.shortcut(&cfg)).collect();
        assert_eq!(caps, ["Super+P", "Ctrl+Alt+V", LIBRARY_SHORTCUT]);
        let cleared = Config {
            capture_hotkey: String::new(),
            record_hotkey: "  ".into(),
            ..Config::default()
        };
        assert_eq!(Tile::Capture.shortcut(&cleared), "No shortcut");
        assert_eq!(Tile::Record.shortcut(&cleared), "No shortcut");
    }

    #[test]
    fn every_tile_runs_its_own_command() {
        let commands: Vec<String> = TILES
            .iter()
            .map(|t| format!("{:?}", t.3.command()))
            .collect();
        assert_eq!(commands, ["Capture", "RecordToggle", "Library"]);
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
}
