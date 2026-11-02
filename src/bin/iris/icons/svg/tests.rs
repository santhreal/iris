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
