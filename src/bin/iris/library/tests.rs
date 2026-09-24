use std::path::PathBuf;
use std::prelude::v1::test;

use iris_lib::library::CaptureEntry;

use super::entries::{fit_name, name_cols};
use super::{visible_rows, Library, CARD_W, GAP, LABEL_GAP, LABEL_PAD, THUMB_H};
use crate::theme::SMALL_ADVANCE;

// The virtualized grid must place every rendered card at the same
// document offset an un-virtualized wrap would, and preserve the
// total content height so the scroll range is unchanged. The
// spacer heights are the load-bearing part: a wrong one shifts
// every card below it and inflates or shrinks the scrollable area.
#[test]
fn visible_rows_preserves_card_offsets_and_scroll_height() {
    let card_h = THUMB_H + 8.0 + 18.0;
    let row_pitch = card_h + GAP;
    // 80 entries, 4 columns -> 20 rows. Row r's un-virtualized top
    // is GAP + r*row_pitch; content bottom is rows*row_pitch.
    let (n, cols, rows) = (80usize, 4usize, 20usize);

    // No viewport yet (first frame): every row renders, no spacers.
    assert_eq!(visible_rows(n, cols, 0.0, 0.0), (0, 19, 0.0, 0.0));

    // Scrolled to the top, two rows tall: rows 0..=1 visible plus
    // one buffer row -> 0..=2, no top spacer.
    let (first, last, top, bottom) = visible_rows(n, cols, 0.0, 2.0 * row_pitch);
    assert_eq!((first, last), (0, 2));
    assert_eq!(top, 0.0);
    // Content bottom = spacer_top + bottom_h + GAP(bottom pad).
    let spacer_top = GAP + (last + 1) as f32 * row_pitch;
    assert_eq!(spacer_top + bottom + GAP, rows as f32 * row_pitch + GAP);

    // Scrolled to the middle: a strict subset renders with both
    // spacers, and every rendered row lands at its un-virtualized
    // offset.
    let (first, last, top, bottom) = visible_rows(n, cols, 8.0 * row_pitch, 3.0 * row_pitch);
    assert!(first > 0 && last < rows - 1 && first <= last);
    // The first rendered card row lands at GAP + first*row_pitch:
    // top spacer occupies [GAP, GAP+top_h], then a GAP line break.
    assert_eq!(GAP + top + GAP, GAP + first as f32 * row_pitch);
    // Total content height is preserved end to end. The rendered
    // rows occupy count*row_pitch - GAP (internal gaps, no
    // trailing), so the sum is top + rendered + bottom + 4*GAP.
    let rendered = (last - first + 1) as f32 * row_pitch - GAP;
    let total = GAP + top + GAP + rendered + GAP + bottom + GAP;
    assert_eq!(total, rows as f32 * row_pitch + GAP);

    // Scrolled past the end: clamps to the last row, never inverts.
    let (first, last, _, _) = visible_rows(n, cols, 100000.0, 500.0);
    assert!(first <= last && last == rows - 1);

    // Empty library: no rows, no spacers.
    assert_eq!(visible_rows(0, cols, 0.0, 500.0), (0, 0, 0.0, 0.0));
}

// WHY: a card's label is fitted to its row before layout, because GPUI
// clips nowrap text rather than ending it in an ellipsis. Two classes
// are closed here. "The label overruns its row": the name and the
// dimensions together pass the card edge or run into each other.
// "The label hides what tells two captures apart": a shortened name
// drops the tail that holds the seconds and the collision suffix, so
// two cards read the same. Not covered: glyphs outside JetBrains Mono
// (a CJK fallback face is wider than 0.6 em), which the row clips.

fn entry(file: &str, width: u32, height: u32) -> CaptureEntry {
    CaptureEntry {
        path: PathBuf::from("shots").join(file),
        thumb: PathBuf::new(),
        width,
        height,
        created_ms: 0,
    }
}

#[test]
fn every_label_fits_its_row() {
    let sides = [1, 12, 123, 1234, 12345, u32::MAX];
    let names = [
        "a",
        "2026-09-23_10-51-52",
        "2026-09-23_10-51-52-2",
        "Screenshot 2026-09-23 at 10-51-52 of the release notes",
        "äöüßäöüßäöüßäöüßäöüßäöüßäöüßäöüß",
    ];
    for w in sides {
        for h in sides {
            for name in names {
                let (label, dims) = Library::entry_name(&entry(&format!("{name}.png"), w, h));
                let cols = label.chars().count() + dims.chars().count();
                let width = cols as f32 * SMALL_ADVANCE + LABEL_GAP + 2.0 * LABEL_PAD;
                assert!(
                    width <= CARD_W,
                    "{label:?} beside {dims:?} is {width}px on a {CARD_W}px card"
                );
                // Shortened only when the whole stem does not fit.
                if name.chars().count() <= name_cols(&dims) {
                    assert_eq!(label, name, "beside {dims:?}");
                }
            }
        }
    }
}

#[test]
fn a_default_name_shows_whole_and_its_suffix_survives_shortening() {
    for (w, h) in [(1280, 720), (3840, 2160), (7680, 4320)] {
        // {date}_{time}, the default template, drops only the extension.
        let (label, _) = Library::entry_name(&entry("2026-09-23_10-51-52.png", w, h));
        assert_eq!(label, "2026-09-23_10-51-52");
        let (label, _) = Library::entry_name(&entry("2026-09-23_10-51-52-2.png", w, h));
        assert!(label.ends_with("-51-52-2"), "{label:?} beside {w}x{h}");
    }
}

#[test]
fn a_shortened_name_keeps_its_head_and_tail() {
    assert_eq!(fit_name("abcdefghij", 10), "abcdefghij");
    assert_eq!(fit_name("abcdefghij", 9), "abcd\u{2026}ghij");
    assert_eq!(fit_name("abcdefghij", 6), "abc\u{2026}ij");
    assert_eq!(fit_name("abcdefghij", 5), "ab\u{2026}ij");
    assert_eq!(fit_name("abcdefghij", 1), "\u{2026}");
    // Characters, not bytes: a cut inside a multi-byte one panics.
    assert_eq!(fit_name("äöüßäöüß", 5), "äö\u{2026}üß");
    assert_eq!(fit_name("截图截图截图", 4), "截图\u{2026}图");
}

#[test]
fn names_that_differ_only_at_the_end_stay_apart() {
    let (a, b) = ("2026-09-23_10-51-52-2", "2026-09-23_10-51-52-3");
    for cols in 3..=a.len() + 2 {
        assert_ne!(fit_name(a, cols), fit_name(b, cols), "{cols} columns");
    }
}
