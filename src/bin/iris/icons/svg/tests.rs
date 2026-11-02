use super::{coverage, document, ink_box, Icon, ALL};

#[test]
fn every_icon_draws_inside_its_box() {
    let mut keys = Vec::new();
    for &kind in ALL {
        let (side, alpha) = coverage(kind, 2.0);
        assert_eq!(alpha.len(), side * side, "{kind:?} is not square");
        assert!(
            alpha.iter().filter(|&&a| a == 255).count() >= side,
            "{kind:?} draws no solid stroke"
        );
        let edge =
            (0..side).flat_map(|i| [i, (side - 1) * side + i, i * side, i * side + side - 1]);
        let clipped: Vec<usize> = edge.filter(|&i| alpha[i] != 0).collect();
        assert!(
            clipped.is_empty(),
            "{kind:?} draws on its box's edge at {clipped:?}"
        );
        keys.push(document(kind).0);
    }
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), ALL.len(), "two icons share an atlas key");
}

/// The icons whose ink box sits off their box's centre on purpose: each
/// balances its ink, not its outline.
const OPTICAL: [Icon; 6] = [
    // A triangle centred on its centroid.
    Icon::Play,
    // A 1 px line on a pixel row's centre at scale 1.
    Icon::CaptionMinimize,
    // Top-heavy glyphs moved down, and a bottom-heavy one up.
    Icon::Text,
    Icon::Mic,
    Icon::MicOff,
    Icon::Highlight,
];

/// The folder sat 0.6 units low, the pin 0.5 up and right, and the
/// cursor 0.6 right and 0.4 down. Every icon's ink box is centred within
/// a quarter unit, but for the OPTICAL ones, which stay off it, so a
/// stale entry fails too.
#[test]
fn every_icon_centres_its_ink_box_in_its_box() {
    const TOLERANCE: f32 = 0.25;
    let mut wrong = Vec::new();
    for &kind in ALL {
        let [left, top, right, bottom] = ink_box(kind);
        let off = ((left + right) / 2.0 - 9.0, (top + bottom) / 2.0 - 9.0);
        let centred = off.0.abs() <= TOLERANCE && off.1.abs() <= TOLERANCE;
        if centred == OPTICAL.contains(&kind) {
            wrong.push(format!("{kind:?} {off:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "ink box centre off its box's by more than {TOLERANCE} units, or OPTICAL and centred: {wrong:?}"
    );
}

/// The icons drawn off their vertical axis on purpose: a slant, a
/// direction, a figure, or a part on one side.
const OFF_VERTICAL_AXIS: [Icon; 20] = [
    Icon::Pen,
    Icon::Line,
    Icon::Arrow,
    Icon::Highlight,
    Icon::Undo,
    Icon::Redo,
    Icon::Check,
    Icon::Copy,
    Icon::Cursor,
    Icon::Crop,
    Icon::Folder,
    Icon::Pin,
    Icon::MicOff,
    Icon::Counter,
    Icon::Record,
    Icon::Play,
    Icon::Fill,
    Icon::Timer,
    Icon::Restore,
    Icon::CaptionRestore,
];

/// The icons drawn off their horizontal axis on purpose: those above,
/// but for the ones that point or split sideways, and those with a top
/// unlike their bottom.
const OFF_HORIZONTAL_AXIS: [Icon; 25] = [
    Icon::Pen,
    Icon::Line,
    Icon::Arrow,
    Icon::Text,
    Icon::Highlight,
    Icon::Undo,
    Icon::Redo,
    Icon::Trash,
    Icon::ChevronDown,
    Icon::Check,
    Icon::Copy,
    Icon::Mic,
    Icon::Cursor,
    Icon::Crop,
    Icon::Folder,
    Icon::Pin,
    Icon::MicOff,
    Icon::Counter,
    Icon::Alert,
    Icon::Download,
    Icon::Timer,
    Icon::Keyboard,
    Icon::Restore,
    Icon::CaptionRestore,
    Icon::CaptionMinimize,
];

/// How much `kind` differs from its mirror image across its vertical
/// axis and across its horizontal one, each as a fraction of its ink.
fn mirror_mismatch(kind: Icon) -> [f32; 2] {
    let (side, alpha) = coverage(kind, 8.0);
    let rows: Vec<&[u8]> = alpha.chunks_exact(side).collect();
    let differ = |(a, b): (&u8, &u8)| f32::from(a.abs_diff(*b));
    let across_vertical: f32 = rows
        .iter()
        .flat_map(|row| row.iter().zip(row.iter().rev()))
        .map(differ)
        .sum();
    let across_horizontal: f32 = rows
        .iter()
        .zip(rows.iter().rev())
        .flat_map(|(top, bottom)| top.iter().zip(bottom.iter()))
        .map(differ)
        .sum();
    let ink: f32 = alpha.iter().map(|&a| f32::from(a)).sum();
    [across_vertical / ink, across_horizontal / ink]
}

/// The keyboard's row of keys sat 0.3 units right of its outline's
/// centre, and the dashed selection's dashes ran on from one corner, so
/// no two corners matched. Every icon matches its mirror image across
/// both axes within a few percent of its ink, but across the axes it is
/// listed off, where it differs by more, so a stale entry fails too.
#[test]
fn every_icon_matches_its_mirror_images_but_across_its_listed_axes() {
    const TOLERANCE: f32 = 0.03;
    let mut wrong = Vec::new();
    for &kind in ALL {
        let [vertical, horizontal] = mirror_mismatch(kind);
        for (axis, off, listed) in [
            ("vertical", vertical, OFF_VERTICAL_AXIS.contains(&kind)),
            (
                "horizontal",
                horizontal,
                OFF_HORIZONTAL_AXIS.contains(&kind),
            ),
        ] {
            if (off > TOLERANCE) != listed {
                wrong.push(format!("{kind:?} across {axis} {off:.3}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "differs from its mirror image by more than {TOLERANCE} of its ink and is not listed off that axis, or is listed and does not: {wrong:?}"
    );
}
