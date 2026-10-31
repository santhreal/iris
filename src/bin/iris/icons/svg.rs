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
    Pen => r##"<path d="M6.8 11.2L13.2 4.8" stroke-width="2.8"/><path d="M3.4 14.6L4.4 10.8L7.2 13.6Z" fill="#fff" stroke="none"/>"##,
    Line => r##"<path d="M4 14L14 4"/><circle cx="4" cy="14" r="2.2" fill="#fff" stroke="none"/><circle cx="14" cy="4" r="2.2" fill="#fff" stroke="none"/>"##,
    Arrow => r##"<path d="M4 14L11.08 6.92"/><path d="M13.2 4.8L11.51 10.56L7.44 6.49Z" fill="#fff" stroke="none"/>"##,
    Ellipse => r##"<ellipse cx="9" cy="9" rx="5.6" ry="4.2"/>"##,
    Rect => r##"<path d="M4.2 5.4L13.8 5.4L13.8 12.6L4.2 12.6Z"/>"##,
    Text => r##"<path d="M4.4 5.2L13.6 5.2"/><path d="M9 5.2L9 14.2"/>"##,
    Highlight => r##"<path d="M4.6 10.4L11.6 3.4" stroke-width="3.4"/><path d="M3 14.6L15 14.6"/>"##,
    Blur => r##"<path d="M4.2 4.2L13.8 4.2L13.8 13.8L4.2 13.8Z"/><rect x="6.4" y="6.4" width="2" height="2" fill="#fff" stroke="none"/><rect x="9.6" y="6.4" width="2" height="2" fill="#fff" stroke="none"/><rect x="6.4" y="9.6" width="2" height="2" fill="#fff" stroke="none"/><rect x="9.6" y="9.6" width="2" height="2" fill="#fff" stroke="none"/>"##,
    Undo => r##"<path d="M7.4 3.6L4 7L7.4 10.4"/><path d="M4.4 7H10.6A3.8 3.8 0 0 1 10.6 14.6H8"/>"##,
    Redo => r##"<g transform="matrix(-1 0 0 1 18 0)"><path d="M7.4 3.6L4 7L7.4 10.4"/><path d="M4.4 7H10.6A3.8 3.8 0 0 1 10.6 14.6H8"/></g>"##,
    Trash => r##"<path d="M4.4 5.6L13.6 5.6"/><path d="M7.4 4L10.6 4"/><path d="M5.4 5.6L6.2 14.2L11.8 14.2L12.6 5.6"/><path d="M8 7.6L8.3 12.2" stroke-width="1.36"/><path d="M10 7.6L9.7 12.2" stroke-width="1.36"/>"##,
    ChevronDown => r##"<path d="M5.4 7L9 10.6L12.6 7"/>"##,
    Check => r##"<path d="M4.4 9.6L7.8 13L13.6 5.2"/>"##,
    Copy => r##"<path d="M7 4L14 4L14 11L7 11Z"/><path d="M4 7L11 7L11 14L4 14Z"/>"##,
    Close => r##"<path d="M5.2 5.2L12.8 12.8"/><path d="M12.8 5.2L5.2 12.8"/>"##,
    Mic => r##"<ellipse cx="9" cy="6.6" rx="2.6" ry="3.8"/><path d="M13.55 9.29A4.6 4.6 0 0 1 4.45 9.29"/><path d="M9 13.2L9 15.4"/><path d="M6.4 15.4L11.6 15.4"/>"##,
    Stroke1 => r##"<path d="M4 9L14 9" stroke-width="1.4"/>"##,
    Stroke2 => r##"<path d="M4 9L14 9" stroke-width="2.6"/>"##,
    Stroke3 => r##"<path d="M4 9L14 9" stroke-width="4.2"/>"##,
    Cursor => r##"<path d="M5 3L5 15L8.4 11.6L10.6 15.8L12.4 14.8L10.2 10.6L14.2 10.2Z" fill="#fff" stroke="none"/>"##,
    Crop => r##"<path d="M4 8L4 4L8 4"/><path d="M14 8L14 4L10 4"/><path d="M4 10L4 14L8 14"/><path d="M14 10L14 14L10 14"/>"##,
    Viewfinder => r##"<path d="M3.2 6.6L3.2 3.2L6.6 3.2" stroke-width="1.8"/><path d="M11.4 3.2L14.8 3.2L14.8 6.6" stroke-width="1.8"/><path d="M3.2 11.4L3.2 14.8L6.6 14.8" stroke-width="1.8"/><path d="M11.4 14.8L14.8 14.8L14.8 11.4" stroke-width="1.8"/>"##,
    Gear => r##"<circle cx="9" cy="9" r="4.1" stroke-width="2.2"/><path d="M13.8 10.2L16.2 9.9L16.2 8.1L13.8 7.8Z" fill="#fff" stroke="none"/><path d="M11.55 13.24L13.45 14.73L14.73 13.45L13.24 11.55Z" fill="#fff" stroke="none"/><path d="M7.8 13.8L8.1 16.2L9.9 16.2L10.2 13.8Z" fill="#fff" stroke="none"/><path d="M4.76 11.55L3.27 13.45L4.55 14.73L6.45 13.24Z" fill="#fff" stroke="none"/><path d="M4.2 7.8L1.8 8.1L1.8 9.9L4.2 10.2Z" fill="#fff" stroke="none"/><path d="M6.45 4.76L4.55 3.27L3.27 4.55L4.76 6.45Z" fill="#fff" stroke="none"/><path d="M10.2 4.2L9.9 1.8L8.1 1.8L7.8 4.2Z" fill="#fff" stroke="none"/><path d="M13.24 6.45L14.73 4.55L13.45 3.27L11.55 4.76Z" fill="#fff" stroke="none"/>"##,
    Folder => r##"<path d="M3.2 5.2L7.2 5.2L8.8 7.2L14.8 7.2L14.8 14L3.2 14Z"/>"##,
    Pin => r##"<path d="M8.5 9.5L3.5 14.5"/><path d="M6 7L11 12" stroke-width="1.8"/><path d="M8.5 9.5L13 5" stroke-width="3"/><path d="M11.5 2.5L15.5 6.5"/>"##,
    Pause => r##"<path d="M6.6 4.5L6.6 13.5" stroke-width="2.4"/><path d="M11.4 4.5L11.4 13.5" stroke-width="2.4"/>"##,
    MicOff => r##"<ellipse cx="9" cy="6.6" rx="2.6" ry="3.8"/><path d="M4 4L14 14"/><path d="M9 13.2L9 15.4"/>"##,
    Counter => r##"<mask id="m"><rect width="18" height="18" fill="#fff" stroke="none"/><path d="M7.4 7.2L9 5.6V12.4" stroke="#000" stroke-width="1.8"/></mask><circle cx="9" cy="9" r="6.4" fill="#fff" stroke="none" mask="url(#m)"/>"##,
    Record => r##"<path d="M3 5.5L11 5.5L11 12.5L3 12.5Z"/><path d="M11 7.5L15 5L15 13L11 10.5"/>"##,
    Play => r##"<path d="M6 4L6 14L14.5 9Z" fill="#fff" stroke="none"/>"##,
    Stop => r##"<rect x="5" y="5" width="8" height="8" fill="#fff" stroke="none"/>"##,
    Alert => r##"<circle cx="9" cy="9" r="6.6"/><path d="M9 5.2L9 10" stroke-width="1.8"/><circle cx="9" cy="12.9" r="1.15" fill="#fff" stroke="none"/>"##,
    ChevronUpDown => r##"<path d="M6.2 7L9 4.2L11.8 7"/><path d="M6.2 11L9 13.8L11.8 11"/>"##,
    Ellipsis => r##"<circle cx="4" cy="9" r="1.45" fill="#fff" stroke="none"/><circle cx="9" cy="9" r="1.45" fill="#fff" stroke="none"/><circle cx="14" cy="9" r="1.45" fill="#fff" stroke="none"/>"##,
    Rotate => r##"<path d="M4.3 12.22A5.2 5.2 0 1 1 14.11 9.05"/><path d="M14.37 10.43L10.94 6.18L16.05 5.24Z" fill="#fff" stroke="none"/>"##,
    FlipH => r##"<path d="M9 3L9 15" stroke-width="1.28"/><path d="M7.4 5L7.4 13L2.6 13Z" fill="#fff" stroke="none"/><path d="M10.6 5L10.6 13L15.4 13Z"/>"##,
    FlipV => r##"<path d="M3 9L15 9" stroke-width="1.28"/><path d="M5 7.4L13 7.4L13 2.6Z" fill="#fff" stroke="none"/><path d="M5 10.6L13 10.6L13 15.4Z"/>"##,
    File => r##"<path d="M5 2.8L10.4 2.8L13.4 5.8L13.4 15.2L5 15.2Z"/><path d="M10.4 2.8L10.4 5.8L13.4 5.8"/>"##,
    Fill => r##"<rect x="4.2" y="5.4" width="9.6" height="7.2" fill="#fff" stroke="none"/>"##,
    Keyboard => r##"<path d="M2.4 4.8L15.6 4.8L15.6 13.2L2.4 13.2Z"/><circle cx="5.4" cy="7.6" r="0.85" fill="#fff" stroke="none"/><circle cx="8" cy="7.6" r="0.85" fill="#fff" stroke="none"/><circle cx="10.6" cy="7.6" r="0.85" fill="#fff" stroke="none"/><circle cx="13.2" cy="7.6" r="0.85" fill="#fff" stroke="none"/><path d="M6.2 10.6L11.8 10.6"/>"##,
    Download => r##"<circle cx="9" cy="9" r="6.6"/><path d="M9 5L9 12.4"/><path d="M6.2 9.8L9 12.6L11.8 9.8"/>"##,
    Sliders => r##"<path d="M3 6L15 6"/><path d="M3 12L15 12"/><circle cx="6.6" cy="6" r="2.3" fill="#fff" stroke="none"/><circle cx="11.4" cy="12" r="2.3" fill="#fff" stroke="none"/>"##,
    Timer => r##"<circle cx="9" cy="10" r="5.6"/><path d="M7.2 2.6L10.8 2.6"/><path d="M9 2.6L9 4.4"/><path d="M9 10L9 7.2"/><path d="M13.2 4.8L14.2 5.8"/>"##,
    Minimize => r##"<path d="M5.4 11.6L12.6 11.6"/>"##,
    Maximize => r##"<rect x="5.4" y="5.4" width="7.2" height="7.2" rx="1"/>"##,
    Restore => r##"<rect x="5.4" y="7.4" width="5.2" height="5.2" rx="0.8"/><path d="M7.4 5.4L12.6 5.4L12.6 10.6"/>"##,
    CaptionMinimize => r##"<path d="M4 9.5L14 9.5" stroke-width="1" stroke-linecap="butt"/>"##,
    CaptionMaximize => r##"<rect x="4.5" y="4.5" width="9" height="9" rx="1" stroke-width="1"/>"##,
    CaptionRestore => r##"<rect x="4.5" y="6.5" width="7" height="7" rx="1" stroke-width="1"/><path d="M6.5 4.5L12.5 4.5A1 1 0 0 1 13.5 5.5L13.5 11.5" stroke-width="1"/>"##,
    CaptionClose => r##"<path d="M4.5 4.5L13.5 13.5" stroke-width="1"/><path d="M13.5 4.5L4.5 13.5" stroke-width="1"/>"##,
    Screen => r##"<rect x="2.6" y="3.6" width="12.8" height="10.8" rx="1.8"/><rect x="5" y="6" width="8" height="6" rx="0.6" fill="#fff" stroke="none"/>"##,
    Selection => r##"<rect x="2.6" y="3.6" width="12.8" height="10.8" rx="1.2" stroke-dasharray="2.2 2.1"/>"##,
    RecordScreen => r##"<rect x="2.6" y="3.6" width="12.8" height="10.8" rx="1.8"/><circle cx="9" cy="9" r="2.7" fill="#fff" stroke="none"/>"##,
    RecordSelection => r##"<rect x="2.6" y="3.6" width="12.8" height="10.8" rx="1.2" stroke-dasharray="2.2 2.1"/><circle cx="9" cy="9" r="2.7" fill="#fff" stroke="none"/>"##,
}

// WHY: the class closed here is "an icon draws outside its box or not
// at all": every variant's document parses, draws ink, and leaves the
// box's edge clear, so no stroke is cut off. Not covered: how a glyph
// reads, which the screenshots show (`tests/glyphs.rs` reads overlaps
// on screen).
#[cfg(test)]
mod tests;
