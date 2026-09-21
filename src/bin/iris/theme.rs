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

/// UI font family, bundled in main.rs. Set on each surface's root;
/// GPUI cascades text styles down the element tree.
pub const FONT: &str = "Inter";

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
