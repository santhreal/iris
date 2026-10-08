//! Design tokens: the single source of truth for iris's visual register.
//! Register: dark macOS chrome. Neutral grays, label tiers as white at
//! falling opacity, one systemBlue accent for state and selection,
//! semantic red and green only where they carry meaning, a 4px grid.

use gpui::{px, App, FontFeatures, FontWeight, Rgba};

const fn rgb(r: f32, g: f32, b: f32) -> Rgba {
    Rgba { r, g, b, a: 1.0 }
}

pub const fn alpha(color: Rgba, a: f32) -> Rgba {
    Rgba { a, ..color }
}

const WHITE: Rgba = rgb(1.0, 1.0, 1.0);
const BLACK: Rgba = rgb(0.0, 0.0, 0.0);

/// Window background, #1E1E1E.
pub const BG: Rgba = rgb(0.118, 0.118, 0.118);
/// Floating panels over the window: menus, popovers, sheets, pills.
pub const BG_ELEV: Rgba = rgb(0.169, 0.169, 0.169);
pub const SURFACE: Rgba = alpha(WHITE, 0.05);
/// Hover wash on ghost controls and rows.
pub const SURFACE_HOVER: Rgba = alpha(WHITE, 0.06);
/// Pressed-state wash: instant feedback that a control took the
/// press, the biggest single cue for perceived responsiveness.
pub const SURFACE_PRESS: Rgba = alpha(WHITE, 0.10);
pub const HAIRLINE: Rgba = alpha(WHITE, 0.09);
/// Grouped-list card fill (macOS System Settings register).
pub const GROUP_BG: Rgba = alpha(WHITE, 0.05);
/// A clickable group card under the pointer and pressed: the card fill
/// with the hover and press washes over it.
pub const CARD_HOVER: Rgba = alpha(WHITE, 0.10);
pub const CARD_PRESS: Rgba = alpha(WHITE, 0.14);
/// Row separator inside a group card, inset to the label's leading edge.
pub const SEPARATOR: Rgba = alpha(WHITE, 0.08);
/// Text field fill and its 1px inner border.
pub const FIELD_BG: Rgba = alpha(BLACK, 0.20);
pub const FIELD_BORDER: Rgba = alpha(WHITE, 0.10);
/// Pop-up button and secondary push button fill.
pub const CONTROL_BG: Rgba = alpha(WHITE, 0.10);

/// Label tiers: primary text, secondary (metadata, captions, resting
/// glyphs), tertiary (placeholders, shortcut columns), disabled.
pub const FG: Rgba = alpha(WHITE, 0.90);
pub const FG_DIM: Rgba = alpha(WHITE, 0.55);
pub const FG_FAINT: Rgba = alpha(WHITE, 0.30);
pub const FG_DISABLED: Rgba = alpha(WHITE, 0.25);

/// systemBlue (dark), #0A84FF: toggle on-track, the primary button,
/// focus rings, selection, links.
pub const ACCENT: Rgba = rgb(0.039, 0.518, 1.0);
/// Text and glyphs drawn on an `ACCENT` fill.
pub const ACCENT_INK: Rgba = WHITE;
/// Keyboard focus ring: 3px of accent at 45%.
pub const FOCUS_RING: Rgba = alpha(ACCENT, 0.45);
/// Destructive text and confirmation fills, #FF453A.
pub const DANGER: Rgba = rgb(1.0, 0.271, 0.227);

/// Window control dots (the macOS traffic-light register).
pub const WIN_CLOSE: Rgba = rgb(1.0, 0.373, 0.341);
pub const WIN_MIN: Rgba = rgb(0.996, 0.741, 0.180);
pub const WIN_ZOOM: Rgba = rgb(0.157, 0.784, 0.251);

/// Pills and small floating chips.
pub const RADIUS_SM: f32 = 8.0;
/// Grouped-list card corners.
pub const RADIUS_GROUP: f32 = 10.0;
/// Menus and popovers.
pub const RADIUS_MENU: f32 = 8.0;
/// Buttons, fields, pop-up buttons, and segmented controls share one
/// corner radius so a row of mixed controls lines up edge to edge.
pub const RADIUS_CONTROL: f32 = 6.0;
/// Control height inside forms and settings rows.
pub const CONTROL_H: f32 = 24.0;
/// Control height in toolbars, and the minimum icon-only hit area.
pub const TOOLBAR_CONTROL_H: f32 = 28.0;
/// Glyph sizes: toolbar icons and row icons.
pub const ICON_TOOLBAR: f32 = 16.0;
pub const ICON_ROW: f32 = 14.0;

/// Spacing, logical px, on the 4px grid.
/// Content inset from the window edges.
pub const INSET: f32 = 20.0;
/// Horizontal padding inside a group row.
pub const ROW_PAD_X: f32 = 12.0;
/// Minimum height of a single-line group row, and of one with a
/// secondary line.
pub const ROW_H: f32 = 36.0;
pub const ROW_H_TALL: f32 = 44.0;
/// Space between groups, and from a section header to its group.
pub const GROUP_GAP: f32 = 20.0;
pub const HEADER_GAP: f32 = 6.0;
/// Unified toolbar (title bar) height.
pub const TOOLBAR_H: f32 = 52.0;

/// Type scale, logical px (weights in the doc of each).
/// Large title, Bold: the Home wordmark.
pub const TEXT_LARGE_TITLE: f32 = 22.0;
/// Title, SemiBold: pane and sheet titles.
pub const TEXT_TITLE: f32 = 15.0;
/// Headline, SemiBold: window titles in the toolbar, row titles.
pub const TEXT_HEADLINE: f32 = 13.0;
/// Body, Regular: labels, menu rows, field text.
pub const TEXT_BODY: f32 = 13.0;
/// Caption, Regular or Medium: metadata, section headers, tooltips.
pub const TEXT_SMALL: f32 = 11.0;

/// UI font family. Every surface sets it on its root; GPUI cascades
/// text styles down the element tree. Bundled (see `load_fonts`) so
/// the family resolves on machines without it installed.
pub const FONT: &str = "Inter";

/// The bold face, also used to bake text annotations into saved
/// images so the output matches the editor's on-canvas text on every
/// platform.
pub const FONT_BOLD_TTF: &[u8] = include_bytes!("../../../assets/fonts/Inter-Bold.ttf");

/// The bundled faces of `FONT`, one per weight the UI uses: Regular,
/// Medium, SemiBold, Bold.
const FONT_FACES: [&[u8]; 4] = [
    include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
    FONT_BOLD_TTF,
];

/// Register the bundled faces with GPUI's text system. Called once at
/// startup, before any window opens.
pub fn load_fonts(cx: &mut gpui::App) {
    let faces = FONT_FACES
        .iter()
        .map(|b| std::borrow::Cow::Borrowed(*b))
        .collect();
    if let Err(e) = cx.text_system().add_fonts(faces) {
        iris_lib::ilog!("iris: fonts: {e}");
    }
}

/// `FONT` at `weight`.
pub fn font(weight: FontWeight) -> gpui::Font {
    gpui::Font {
        weight,
        ..gpui::font(FONT)
    }
}

/// Tabular numerals (`tnum`): every digit has one advance, so a number
/// that updates in place (a timer, a dimension readout) or aligns in a
/// column does not shift. Apply with `.font_features(theme::tabular())`.
pub fn tabular() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

/// Lines `text` wraps to in `FONT` at `size` and `weight` within
/// `width` logical px, as GPUI's text element wraps it: at word
/// boundaries, with a word wider than a line broken across lines. An
/// empty text is one line.
pub fn wrapped_lines(cx: &App, text: &str, size: f32, weight: FontWeight, width: f32) -> usize {
    let mut wrapper = cx.text_system().line_wrapper(font(weight), px(size));
    text.split('\n')
        .map(|line| {
            1 + wrapper
                .wrap_line(&[gpui::LineFragment::text(line)], px(width))
                .count()
        })
        .sum()
}

/// The sum of the Regular face's glyph advances for `text` at `size`,
/// logical px, read from the bundled TTF without a text system: what a
/// unit test without a GPUI app measures a string with. Kerning is not
/// applied; Inter's pairs mostly tighten, so this is close to and
/// usually above the shaped width.
#[cfg(test)]
pub fn advance_sum(text: &str, size: f32) -> f32 {
    use ab_glyph::{Font as _, ScaleFont as _};
    let face = ab_glyph::FontRef::try_from_slice(FONT_FACES[0]).expect("bundled Inter parses");
    let scaled = face.as_scaled(ab_glyph::PxScale::from(
        size * face.height_unscaled() / face.units_per_em().expect("units per em"),
    ));
    text.chars()
        .map(|c| scaled.h_advance(scaled.glyph_id(c)))
        .sum()
}

use gpui::{point, BoxShadow};

/// The soft drop under a toggle knob and similar raised discs.
pub fn shadow_knob() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: gpui::hsla(0.0, 0.0, 0.0, 0.30),
        offset: point(px(0.), px(1.)),
        blur_radius: px(2.5),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// Resting elevation: cards and thumbnails sitting on a surface.
pub fn shadow_rest() -> BoxShadow {
    BoxShadow {
        color: gpui::hsla(0.0, 0.0, 0.0, 0.22),
        offset: point(px(0.), px(2.)),
        blur_radius: px(10.),
        spread_radius: px(0.),
        inset: false,
    }
}

/// Floating chrome: pills, menus, toasts hovering above content.
pub fn shadow_float() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.30),
            offset: point(px(0.), px(4.)),
            blur_radius: px(18.),
            spread_radius: px(0.),
            inset: false,
        },
        BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.18),
            offset: point(px(0.), px(1.)),
            blur_radius: px(4.),
            spread_radius: px(0.),
            inset: false,
        },
    ]
}

/// Floating chrome in a window with little room around it (the
/// recording chip): a contact shadow short enough to end inside a 16px
/// margin, where `shadow_float` would be cut off at the window edge.
pub fn shadow_tight() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.34),
            offset: point(px(0.), px(2.)),
            blur_radius: px(4.5),
            spread_radius: px(0.),
            inset: false,
        },
        BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.20),
            offset: point(px(0.), px(1.)),
            blur_radius: px(1.5),
            spread_radius: px(0.),
            inset: false,
        },
    ]
}

/// The toast and notice card shadow: a soft drop over a contact layer,
/// the only thing separating a card from the desktop. `visibility`
/// scales both: during an entrance the shadow fades in with the card's
/// visible fraction, so the blur never arrives ahead of the pixels
/// casting it. The capture flight raises it with the card's travel to
/// full visibility at rest, so the card it hands to the toast casts the
/// same shadow before and after.
pub fn card_shadow(visibility: f32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.34 * visibility),
            offset: point(px(0.), px(8.)),
            blur_radius: px(12.),
            spread_radius: px(0.),
            inset: false,
        },
        BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.20 * visibility),
            offset: point(px(0.), px(2.)),
            blur_radius: px(5.),
            spread_radius: px(0.),
            inset: false,
        },
    ]
}

/// Room around a card inside its window for `card_shadow`, logical px:
/// its `shadow_reach`. A window edge closer to the card cuts the
/// shadow off in a straight line.
pub const CARD_BLEED: f32 = 44.0;

/// How far past its element's bounds a shadow list paints, logical px.
/// GPUI draws each shadow's Gaussian out to three blur radii around the
/// element's bounds grown by the spread and moved by the offset.
#[cfg(test)]
pub fn shadow_reach(shadows: &[BoxShadow]) -> f32 {
    shadows
        .iter()
        .map(|s| {
            3.0 * f32::from(s.blur_radius)
                + f32::from(s.spread_radius)
                + f32::from(s.offset.x).abs().max(f32::from(s.offset.y).abs())
        })
        .fold(0.0, f32::max)
}

// WHY: the class closed here is "a card window cuts its own shadow": a
// window edge inside the shadow's reach ends the blur in a hard line
// over whatever lies behind, and the toast once cut a 32px blur at 44px.
// Every window that paints `card_shadow` sizes its bleed from
// CARD_BLEED. Not covered: a window that paints `card_shadow` with a
// bleed of its own; the chip's pill shadow has its own test.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_shadow_ends_inside_the_card_bleed() {
        let reach = shadow_reach(&card_shadow(1.0));
        assert!(
            reach <= CARD_BLEED,
            "card_shadow reaches {reach}px past the card; windows leave {CARD_BLEED}px"
        );
    }

    #[test]
    fn reach_counts_three_blurs_the_spread_and_the_larger_offset() {
        let s = |x: f32, y: f32, blur: f32, spread: f32| BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 1.0),
            offset: point(px(x), px(y)),
            blur_radius: px(blur),
            spread_radius: px(spread),
            inset: false,
        };
        assert_eq!(shadow_reach(&[]), 0.0);
        assert_eq!(shadow_reach(&[s(0.0, 8.0, 12.0, 0.0)]), 44.0);
        assert_eq!(shadow_reach(&[s(-6.0, 2.0, 1.0, 3.0)]), 12.0);
        assert_eq!(
            shadow_reach(&[s(0.0, 2.0, 5.0, 0.0), s(0.0, -9.0, 10.0, 1.0)]),
            40.0
        );
    }
}
