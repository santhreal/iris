//! The SVG document of each icon: an 18x18 box, white ink. resvg
//! rasterizes a document into one coverage mask, which paints as a
//! single sprite in the icon's color, so parts of a glyph that overlap
//! cover a pixel once at any opacity.

use super::Icon;

/// The opening tag every document shares: strokes are white, 1.6 wide,
/// with round caps and joins, and an element that fills sets its own
/// `fill` and `stroke="none"`.
macro_rules! svg {
    ($body:literal) => {
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 18 18" fill="none" stroke="#fff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"##,
            $body,
            "</svg>"
        )
    };
}

/// `document`: one arm a variant, so an icon without a document does
/// not compile, and `ALL`, every variant from the same arms.
macro_rules! documents {
    ($($name:ident => $body:literal,)*) => {
        /// The sprite atlas key and the SVG document of `kind`.
        pub(super) fn document(kind: Icon) -> (&'static str, &'static str) {
            match kind {
                $(Icon::$name => (concat!("iris-icon/", stringify!($name)), svg!($body)),)*
            }
        }

        #[cfg(test)]
        const ALL: &[Icon] = &[$(Icon::$name,)*];
    };
}

documents! {
    Pen => r##"<path d="M6.47 11.53L13.83 4.17" stroke-width="3"/><path d="M2.56 15.44L3.71 11.07L6.93 14.29Z" fill="#fff" stroke="none"/>"##,
    Line => r##"<path d="M5 13L13 5"/><circle cx="4.6" cy="13.4" r="1.9" fill="#fff" stroke="none"/><circle cx="13.4" cy="4.6" r="1.9" fill="#fff" stroke="none"/>"##,
    Arrow => r##"<path d="M4.2 13.8L11.43 6.57"/><path d="M14.4 3.6L12.92 9.19L8.81 5.08Z" fill="#fff" stroke="none"/>"##,
    Ellipse => r##"<ellipse cx="9" cy="9" rx="6" ry="4.6"/>"##,
    Rect => r##"<rect x="3.6" y="5" width="10.8" height="8" rx="0.6"/>"##,
    Text => r##"<path d="M3.8 4.4L14.2 4.4"/><path d="M9 4.4L9 14.4"/>"##,
    Highlight => r##"<path d="M4.6 10.4L11.6 3.4" stroke-width="3.4"/><path d="M3 14.6L15 14.6"/>"##,
    Blur => r##"<path d="M4.2 4.2L13.8 4.2L13.8 13.8L4.2 13.8Z"/><rect x="6.4" y="6.4" width="2" height="2" fill="#fff" stroke="none"/><rect x="9.6" y="6.4" width="2" height="2" fill="#fff" stroke="none"/><rect x="6.4" y="9.6" width="2" height="2" fill="#fff" stroke="none"/><rect x="9.6" y="9.6" width="2" height="2" fill="#fff" stroke="none"/>"##,
    Undo => r##"<path d="M7.4 3.6L4 7L7.4 10.4"/><path d="M4.4 7H10.6A3.8 3.8 0 0 1 10.6 14.6H8"/>"##,
    Redo => r##"<g transform="matrix(-1 0 0 1 18 0)"><path d="M7.4 3.6L4 7L7.4 10.4"/><path d="M4.4 7H10.6A3.8 3.8 0 0 1 10.6 14.6H8"/></g>"##,
    Trash => r##"<path d="M4.4 5.6L13.6 5.6"/><path d="M7.4 4L10.6 4"/><path d="M5.4 5.6L6.2 14.2L11.8 14.2L12.6 5.6"/><path d="M8 7.6L8.3 12.2" stroke-width="1.36"/><path d="M10 7.6L9.7 12.2" stroke-width="1.36"/>"##,
    ChevronDown => r##"<path d="M5.4 7L9 10.6L12.6 7"/>"##,
    Check => r##"<path d="M4.4 9.6L7.8 13L13.6 5.2"/>"##,
    Copy => r##"<rect x="6.8" y="3.6" width="7.6" height="7.6" rx="1.6"/><path d="M11.2 14.4L5.2 14.4A1.6 1.6 0 0 1 3.6 12.8L3.6 6.8"/>"##,
    Close => r##"<path d="M4.6 4.6L13.4 13.4"/><path d="M13.4 4.6L4.6 13.4"/>"##,
    Mic => r##"<g transform="matrix(0.9 0 0 0.9 0.9 0.9)" stroke-width="1.78"><rect x="6.4" y="2.4" width="5.2" height="8.6" rx="2.6" fill="#fff" stroke="none"/><path d="M4.4 9A4.6 4.6 0 0 0 13.6 9"/><path d="M9 13.6L9 15.6"/></g>"##,
    Stroke1 => r##"<path d="M3.8 9L14.2 9" stroke-width="1.4"/>"##,
    Stroke2 => r##"<path d="M4.4 9L13.6 9" stroke-width="2.6"/>"##,
    Stroke3 => r##"<path d="M5.2 9L12.8 9" stroke-width="4.2"/>"##,
    Cursor => r##"<path d="M4.4 2.6L4.4 14.6L7.8 11.2L10 15.4L11.8 14.4L9.6 10.2L13.6 9.8Z" fill="#fff" stroke="none"/>"##,
    Crop => r##"<path d="M5.6 3.2L5.6 12.4L14.8 12.4"/><path d="M3.2 5.6L12.4 5.6L12.4 14.8"/>"##,
    Viewfinder => r##"<path d="M2.8 7L2.8 4.8A2 2 0 0 1 4.8 2.8L7 2.8"/><path d="M11 2.8L13.2 2.8A2 2 0 0 1 15.2 4.8L15.2 7"/><path d="M2.8 11L2.8 13.2A2 2 0 0 0 4.8 15.2L7 15.2"/><path d="M15.2 11L15.2 13.2A2 2 0 0 1 13.2 15.2L11 15.2"/><circle cx="9" cy="9" r="2.4" fill="#fff" stroke="none"/>"##,
    Gear => r##"<path d="M7.28 4.73L7.67 2.74A6.4 6.4 0 0 1 10.33 2.74L10.72 4.73A4.6 4.6 0 0 1 11.83 5.38L13.76 4.72A6.4 6.4 0 0 1 15.09 7.02L13.56 8.36A4.6 4.6 0 0 1 13.56 9.64L15.09 10.98A6.4 6.4 0 0 1 13.76 13.28L11.83 12.62A4.6 4.6 0 0 1 10.72 13.27L10.33 15.26A6.4 6.4 0 0 1 7.67 15.26L7.28 13.27A4.6 4.6 0 0 1 6.17 12.62L4.24 13.28A6.4 6.4 0 0 1 2.91 10.98L4.44 9.64A4.6 4.6 0 0 1 4.44 8.36L2.91 7.02A6.4 6.4 0 0 1 4.24 4.72L6.17 5.38A4.6 4.6 0 0 1 7.28 4.73Z"/><circle cx="9" cy="9" r="2"/>"##,
    Folder => r##"<path d="M3.2 4.6L7.2 4.6L8.8 6.6L14.8 6.6L14.8 13.4L3.2 13.4Z"/>"##,
    Pin => r##"<path d="M8 10L3 15"/><path d="M5.5 7.5L10.5 12.5" stroke-width="1.8"/><path d="M8 10L12.5 5.5" stroke-width="3"/><path d="M11 3L15 7"/>"##,
    Pause => r##"<path d="M6.2 4.9L6.2 13.1" stroke-width="3"/><path d="M11.8 4.9L11.8 13.1" stroke-width="3"/>"##,
    MicOff => r##"<g transform="matrix(0.9 0 0 0.9 0.9 0.9)" stroke-width="1.78"><mask id="m"><rect width="18" height="18" fill="#fff" stroke="none"/><path d="M3.4 3.4L14.6 14.6" stroke="#000" stroke-width="4.9"/></mask><g mask="url(#m)"><rect x="6.4" y="2.4" width="5.2" height="8.6" rx="2.6" fill="#fff" stroke="none"/><path d="M4.4 9A4.6 4.6 0 0 0 13.6 9"/><path d="M9 13.6L9 15.6"/></g><path d="M3.4 3.4L14.6 14.6"/></g>"##,
    Counter => r##"<circle cx="9" cy="9" r="6.4"/><path d="M7.6 7.2L9.2 5.9V12.2" stroke-width="1.6"/>"##,
    Record => r##"<rect x="2.2" y="4.6" width="9.4" height="8.8" rx="2"/><path d="M11.6 7.9L15.4 5.7L15.4 12.3L11.6 10.1"/>"##,
    Play => r##"<path d="M6.2 4.3L6.2 13.7L14.3 9Z" fill="#fff" stroke-width="1.4"/>"##,
    Stop => r##"<rect x="4.5" y="4.5" width="9" height="9" rx="2" fill="#fff" stroke="none"/>"##,
    Alert => r##"<circle cx="9" cy="9" r="6.6"/><path d="M9 5.2L9 10" stroke-width="1.8"/><circle cx="9" cy="12.9" r="1.15" fill="#fff" stroke="none"/>"##,
    ChevronUpDown => r##"<path d="M6.2 7L9 4.2L11.8 7"/><path d="M6.2 11L9 13.8L11.8 11"/>"##,
    Ellipsis => r##"<circle cx="4" cy="9" r="1.45" fill="#fff" stroke="none"/><circle cx="9" cy="9" r="1.45" fill="#fff" stroke="none"/><circle cx="14" cy="9" r="1.45" fill="#fff" stroke="none"/>"##,
    Fill => r##"<rect x="3.6" y="5" width="10.8" height="8" rx="1.4"/><path d="M9 5H5A1.4 1.4 0 0 0 3.6 6.4V11.6A1.4 1.4 0 0 0 5 13H9Z" fill="#fff" stroke="none"/>"##,
    Keyboard => r##"<rect x="2.2" y="4.6" width="13.6" height="8.8" rx="2"/><circle cx="5.4" cy="7.6" r="0.85" fill="#fff" stroke="none"/><circle cx="8" cy="7.6" r="0.85" fill="#fff" stroke="none"/><circle cx="10.6" cy="7.6" r="0.85" fill="#fff" stroke="none"/><circle cx="13.2" cy="7.6" r="0.85" fill="#fff" stroke="none"/><path d="M6.4 10.6L11.6 10.6"/>"##,
    Download => r##"<circle cx="9" cy="9" r="6.4"/><path d="M9 5.4L9 12.2"/><path d="M6.4 9.8L9 12.4L11.6 9.8"/>"##,
    Library => r##"<rect x="3" y="3" width="4.8" height="4.8" rx="1.3"/><rect x="10.2" y="3" width="4.8" height="4.8" rx="1.3"/><rect x="3" y="10.2" width="4.8" height="4.8" rx="1.3"/><rect x="10.2" y="10.2" width="4.8" height="4.8" rx="1.3"/>"##,
    Timer => r##"<circle cx="9" cy="10" r="5.6"/><path d="M7.2 2.6L10.8 2.6"/><path d="M9 2.6L9 4.4"/><path d="M9 10L9 7.2"/><path d="M13.2 4.8L14.2 5.8"/>"##,
    Minimize => r##"<path d="M5.4 9L12.6 9"/>"##,
    Maximize => r##"<rect x="5.4" y="5.4" width="7.2" height="7.2" rx="1"/>"##,
    Restore => r##"<rect x="5.4" y="7.4" width="5.2" height="5.2" rx="0.8"/><path d="M7.4 5.4L12.6 5.4L12.6 10.6"/>"##,
    CaptionMinimize => r##"<path d="M4 9.5L14 9.5" stroke-width="1" stroke-linecap="butt"/>"##,
    CaptionMaximize => r##"<rect x="4.5" y="4.5" width="9" height="9" rx="1" stroke-width="1"/>"##,
    CaptionRestore => r##"<rect x="4.5" y="6.5" width="7" height="7" rx="1" stroke-width="1"/><path d="M6.5 4.5L12.5 4.5A1 1 0 0 1 13.5 5.5L13.5 11.5" stroke-width="1"/>"##,
    CaptionClose => r##"<path d="M4.5 4.5L13.5 13.5" stroke-width="1"/><path d="M13.5 4.5L4.5 13.5" stroke-width="1"/>"##,
    Screen => r##"<rect x="2.6" y="3.6" width="12.8" height="10.8" rx="1.8"/><rect x="6" y="7" width="6" height="4" rx="0.7" fill="#fff" stroke="none"/>"##,
    Selection => r##"<rect x="2.6" y="3.6" width="12.8" height="10.8" rx="1.2" stroke-dasharray="2.2 2.1"/>"##,
    RecordScreen => r##"<rect x="2.6" y="3.6" width="12.8" height="10.8" rx="1.8"/><circle cx="9" cy="9" r="2.7" fill="#fff" stroke="none"/>"##,
    RecordSelection => r##"<rect x="2.6" y="3.6" width="12.8" height="10.8" rx="1.2" stroke-dasharray="2.2 2.1"/><circle cx="9" cy="9" r="2.7" fill="#fff" stroke="none"/>"##,
}

// WHY: the classes closed here are "an icon draws outside its box or
// not at all": every variant's document parses, draws ink, and leaves
// the box's edge clear, so no stroke is cut off; and "a glyph sits off
// its box's centre", which a button centres by its box. Not covered:
// how a glyph reads, which the screenshots show (`tests/glyphs.rs`
// reads overlaps on screen).
#[cfg(test)]
mod tests;
