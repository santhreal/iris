use std::prelude::v1::test;

use super::{visible_rows, GAP, THUMB_H};

#[test]
fn containing_folder_path_resolution() {
    let path = std::path::Path::new("/tmp/iris/captures/screenshot_01.png");
    let parent = path.parent().unwrap_or(path);
    assert_eq!(parent, std::path::Path::new("/tmp/iris/captures"));

    let root_file = std::path::Path::new("/file.png");
    let parent = root_file.parent().unwrap_or(root_file);
    assert_eq!(parent, std::path::Path::new("/"));
}

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
