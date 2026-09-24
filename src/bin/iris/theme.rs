//! Design tokens: the single source of truth for iris's visual register.
//! Register (near-black monochrome, semantic
//! color only where it carries meaning, 9/14 radius scale).

use gpui::Rgba;

const fn rgb(r: f32, g: f32, b: f32) -> Rgba {
    Rgba { r, g, b, a: 1.0 }
}

pub const fn alpha(color: Rgba, a: f32) -> Rgba {
    Rgba { a, ..color }
}

const WHITE: Rgba = rgb(1.0, 1.0, 1.0);

pub const BG: Rgba = rgb(0.106, 0.106, 0.118);
pub const BG_ELEV: Rgba = rgb(0.149, 0.149, 0.169);
pub const SURFACE: Rgba = alpha(WHITE, 0.055);
pub const SURFACE_HOVER: Rgba = alpha(WHITE, 0.095);
pub const HAIRLINE: Rgba = alpha(WHITE, 0.09);
/// Grouped-list card fill (macOS System Settings register).
pub const GROUP_BG: Rgba = alpha(WHITE, 0.045);
/// Row separator inside a group card, inset from the card's left edge.
pub const SEPARATOR: Rgba = alpha(WHITE, 0.07);
/// Text field fill inside a group row.
pub const FIELD_BG: Rgba = alpha(WHITE, 0.07);
pub const FG: Rgba = rgb(0.949, 0.949, 0.957);
pub const FG_DIM: Rgba = rgb(0.635, 0.635, 0.659);
pub const FG_FAINT: Rgba = rgb(0.443, 0.443, 0.478);
pub const ACCENT: Rgba = rgb(0.961, 0.961, 0.969);
pub const ACCENT_INK: Rgba = rgb(0.106, 0.106, 0.118);
// Only the Linux recording chip's pause state uses DANGER today; the
// palette is a shared vocabulary, so it stays defined everywhere.
#[allow(dead_code)]
pub const DANGER: Rgba = rgb(1.0, 0.271, 0.227);

/// Pressed-state wash: instant feedback that a control took the
/// press, the biggest single cue for perceived responsiveness.
pub const SURFACE_PRESS: Rgba = alpha(WHITE, 0.16);

/// Window control dots (the macOS traffic-light register).
pub const WIN_CLOSE: Rgba = rgb(1.0, 0.373, 0.341);
pub const WIN_MIN: Rgba = rgb(0.996, 0.741, 0.180);
pub const WIN_ZOOM: Rgba = rgb(0.157, 0.784, 0.251);

pub const RADIUS_SM: f32 = 9.0;
/// Grouped-list card corners.
pub const RADIUS_GROUP: f32 = 12.0;
/// Buttons, fields, and dropdowns share one corner radius and height
/// so a row of mixed controls lines up edge to edge.
pub const RADIUS_CONTROL: f32 = 7.0;
pub const CONTROL_H: f32 = 28.0;

/// Type scale, px. JetBrains Mono has a 0.6 em advance, wider than a
/// proportional sans, so body text is 13px rather than GPUI's 14px
/// `text_sm` to keep labels inside fixed-width controls.
pub const TEXT_SMALL: f32 = 11.5;
pub const TEXT_BODY: f32 = 13.0;
pub const TEXT_TITLE: f32 = 13.5;
pub const TEXT_HEADING: f32 = 17.0;
/// Width of one glyph at `TEXT_SMALL`. JetBrains Mono is monospace at a
/// 0.6 em advance, so a small label's width is known before layout.
pub const SMALL_ADVANCE: f32 = 0.6 * TEXT_SMALL;

/// UI font family. Every surface sets it on its root; GPUI cascades
/// text styles down the element tree. Bundled (see `load_fonts`) so
/// the family resolves on machines without it installed.
pub const FONT: &str = "JetBrains Mono";

/// The bold face, also used to bake text annotations into saved
/// images so the output matches on every platform.
pub const FONT_BOLD_TTF: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Bold.ttf");

/// The bundled faces of `FONT`, one per weight the UI uses.
const FONT_FACES: [&[u8]; 4] = [
    include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-Medium.ttf"),
    include_bytes!("../../../assets/fonts/JetBrainsMono-SemiBold.ttf"),
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

use gpui::{point, px, BoxShadow};

/// Resting elevation: cards and thumbnails sitting on a surface.
pub fn shadow_rest() -> BoxShadow {
    BoxShadow {
        color: gpui::hsla(0.0, 0.0, 0.0, 0.22),
        offset: point(px(0.), px(2.)),
        blur_radius: px(10.),
        spread_radius: px(0.),
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
        },
        BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.18),
            offset: point(px(0.), px(1.)),
            blur_radius: px(4.),
            spread_radius: px(0.),
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
        },
        BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.20),
            offset: point(px(0.), px(1.)),
            blur_radius: px(1.5),
            spread_radius: px(0.),
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
        },
        BoxShadow {
            color: gpui::hsla(0.0, 0.0, 0.0, 0.20 * visibility),
            offset: point(px(0.), px(2.)),
            blur_radius: px(5.),
            spread_radius: px(0.),
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
