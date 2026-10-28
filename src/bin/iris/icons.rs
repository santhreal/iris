//! Vector icons: one stroke-weighted set for every chrome glyph.
//!
//! Text labels and emoji do not ship on chrome. Every icon is an SVG
//! document in an 18x18 box (`svg`), scaled to the element bounds and
//! painted as one sprite in one color. The editor's vector markup
//! tessellates through the helpers in `tessellate`.

use gpui::*;

mod svg;
mod tessellate;

pub(crate) use tessellate::{
    push_disc, push_ellipse_fill, push_filled_triangle, push_quad, push_rect_fill, push_ring,
    push_segment, TriRecorder,
};

// Some variants are only drawn by the Linux recording chip; the enum is
// a shared vocabulary, so the unused-on-Windows variants stay.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Pen,
    Line,
    Arrow,
    Ellipse,
    Rect,
    Text,
    Highlight,
    Blur,
    Undo,
    Redo,
    Trash,
    ChevronDown,
    Check,
    Copy,
    Close,
    Mic,
    Stroke1,
    Stroke2,
    Stroke3,
    Cursor,
    Crop,
    Viewfinder,
    Grid,
    Gear,
    Folder,
    Pin,
    Pause,
    MicOff,
    Counter,
    Record,
    Play,
    Stop,
    Alert,
    ChevronUpDown,
    Ellipsis,
    Rotate,
    FlipH,
    FlipV,
    Share,
    Fill,
    Keyboard,
    Download,
    Sliders,
}

/// A `size`px square canvas that paints `kind` in `color`. It neither
/// grows nor shrinks: in a row whose text overflows, a shrinkable
/// canvas collapsed toward zero width and painted a sliver.
pub fn icon(kind: Icon, color: Rgba, size: f32) -> Canvas<()> {
    canvas(
        move |_, _, _| (),
        move |bounds, (), window, cx| paint_icon(kind, bounds, color, window, cx),
    )
    .flex_none()
    .w(px(size))
    .h(px(size))
}

/// Paint `kind` into `bounds` in `color`. The atlas holds one coverage
/// mask an icon and size; the color applies once to the whole mask.
fn paint_icon(kind: Icon, bounds: Bounds<Pixels>, color: Rgba, window: &mut Window, cx: &App) {
    let (key, document) = svg::document(kind);
    if let Err(e) = window.paint_svg(
        bounds,
        SharedString::new_static(key),
        Some(document.as_bytes()),
        TransformationMatrix::unit(),
        color.into(),
        cx,
    ) {
        iris_lib::ilog!("iris: paint icon {kind:?}: {e}");
    }
}

// WHY: the class closed here is "a glyph shrinks in a row with no room
// left". A canvas in a flex row shrank like text, so a folder glyph
// beside an overflowing path in the settings window painted a sliver.
// `icon` is the one constructor every glyph goes through, and its box
// neither grows nor shrinks. Not covered: a caller that wraps the glyph
// in a box of its own that shrinks.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    #[test]
    fn an_icon_keeps_its_size_in_a_full_row() {
        let mut glyph = icon(Icon::Folder, crate::theme::FG, 16.);
        let style = glyph.style();
        assert_eq!(style.flex_shrink, Some(0.));
        assert_eq!(style.flex_grow, Some(0.));
        assert_eq!(style.size.width, Some(px(16.).into()));
        assert_eq!(style.size.height, Some(px(16.).into()));
    }
}
