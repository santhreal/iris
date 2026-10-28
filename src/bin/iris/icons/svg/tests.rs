use std::sync::Arc;

use gpui::SvgRenderer;

use super::{document, ALL};

/// The alpha of each pixel of `kind` rasterized at `scale`, in rows of
/// `18 * scale`.
fn coverage(kind: super::Icon, scale: f32) -> (usize, Vec<u8>) {
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
