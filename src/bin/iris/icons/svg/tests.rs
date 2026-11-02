use std::sync::Arc;

use gpui::SvgRenderer;

use super::{document, Icon, ALL};

/// The alpha of each pixel of `kind` rasterized at `scale`, in rows of
/// `18 * scale`.
fn coverage(kind: Icon, scale: f32) -> (usize, Vec<u8>) {
    let (_, svg) = document(kind);
    let image = SvgRenderer::new(Arc::new(()))
        .render_single_frame(svg.as_bytes(), scale)
        .unwrap_or_else(|e| panic!("{kind:?} does not parse: {e}"));
    let side = image.size(0).width.0 as usize;
    let bgra = image.as_bytes(0).unwrap();
    (side, bgra.iter().skip(3).step_by(4).copied().collect())
}

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
        let (side, alpha) = coverage(kind, 8.0);
        // Raster pixels a unit of the 18-unit box.
        let unit = side as f32 / 18.0;
        let (mut lo, mut hi) = ((side, side), (0, 0));
        for (i, _) in alpha.iter().enumerate().filter(|&(_, &a)| a > 127) {
            let (x, y) = (i % side, i / side);
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        let centre = |a: usize, b: usize| (a + b + 1) as f32 / 2.0 / unit - 9.0;
        let off = (centre(lo.0, hi.0), centre(lo.1, hi.1));
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
