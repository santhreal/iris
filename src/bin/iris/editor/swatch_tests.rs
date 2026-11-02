// WHY: the class closed here is "a swatch shrinks into the options
// bar". The black swatch's hairline, white at 9% over #141416, landed on
// the bar's own gray, so black drew 2 device px narrower than every
// other swatch. Every palette color and every gray from black to white
// keeps a rim at least SWATCH_APART from the bar. Not covered: the
// selected swatch's accent ring, which sits outside the rim.

use gpui::Rgba;

use super::action::{hex_rgba, COLORS};
use super::render_chrome::{swatch_rim, SWATCH_APART};
use crate::theme;

#[test]
fn every_swatch_keeps_its_rim_apart_from_the_options_bar() {
    let grays = (0..=255u8).map(|v| {
        let c = f32::from(v) / 255.0;
        Rgba {
            r: c,
            g: c,
            b: c,
            a: 1.0,
        }
    });
    let bar = theme::BG_ELEV;
    let mut wrong = Vec::new();
    for fill in COLORS.iter().map(|(hex, _)| hex_rgba(hex)).chain(grays) {
        let rim = swatch_rim(fill);
        let edge = [
            (rim.r, fill.r, bar.r),
            (rim.g, fill.g, bar.g),
            (rim.b, fill.b, bar.b),
        ]
        .map(|(r, f, b)| (r * rim.a + f * (1.0 - rim.a) - b).abs());
        let apart = edge[0].max(edge[1]).max(edge[2]);
        if apart < SWATCH_APART {
            wrong.push(format!("{fill:?}: rim {apart:.4} from the bar"));
        }
    }
    assert!(
        wrong.is_empty(),
        "swatches whose rim matches the options bar:\n{}",
        wrong.join("\n")
    );
}
