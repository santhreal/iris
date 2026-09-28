//! WHY: a pop-up on an X11 screen with no compositing manager shows
//! exactly the pixels its bounding shape covers. The class closed here
//! is a shape that differs from the painted rounded rect: a corner row
//! with the wrong inset, a bottom corner that does not mirror the top, a
//! middle band that is missing or overlaps the corners, a radius past
//! half the shorter side, and areas whose device edges drift apart when
//! the scale is fractional; and a shape sent as a rectangle per corner
//! row where rows share an inset. Not covered: the Shape request itself
//! and what the X server shows, which `tests/popups.rs` checks on Xvfb.

use super::{rounded, spans, Span};
use crate::sys::window::Rounded;

/// How many rectangles of `rects` cover each pixel of a `w`x`h` grid.
fn coverage(rects: &[Span], w: i32, h: i32) -> Vec<u32> {
    let mut grid = vec![0u32; (w * h) as usize];
    for &[x, y, rw, rh] in rects {
        assert!(rw > 0 && rh > 0, "an empty rectangle {:?}", [x, y, rw, rh]);
        for py in y..y + rh {
            for px in x..x + rw {
                assert!(
                    (0..w).contains(&px) && (0..h).contains(&py),
                    "a rectangle past the rect: {:?}",
                    [x, y, rw, rh]
                );
                grid[(py * w + px) as usize] += 1;
            }
        }
    }
    grid
}

/// Whether the centre of pixel (`px`, `py`) lies inside a `w`x`h` rect
/// whose corners have radius `r`: within `r` of the nearest point of the
/// rect inset by `r`.
fn inside(px: i32, py: i32, w: i32, h: i32, r: i32) -> bool {
    let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
    let r = r.clamp(0, w.min(h) / 2) as f32;
    let nx = cx.clamp(r, w as f32 - r);
    let ny = cy.clamp(r, h as f32 - r);
    (cx - nx).powi(2) + (cy - ny).powi(2) <= r * r
}

#[test]
fn the_rectangles_cover_each_pixel_inside_the_rounded_rect_once_and_no_other() {
    let sizes = [
        (1, 1),
        (7, 3),
        (24, 24),
        (36, 36),
        (200, 140),
        (208, 36),
        (57, 91),
    ];
    let radii = [0, 1, 2, 5, 12, 18, 30, 1000];
    for (w, h) in sizes {
        for r in radii {
            let mut rects = Vec::new();
            rounded(&mut rects, 0, 0, w, h, r);
            let grid = coverage(&rects, w, h);
            for py in 0..h {
                for px in 0..w {
                    let want = u32::from(inside(px, py, w, h, r));
                    assert_eq!(
                        grid[(py * w + px) as usize],
                        want,
                        "pixel ({px}, {py}) of {w}x{h} radius {r}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_rect_moves_with_its_origin() {
    let (mut at_zero, mut moved) = (Vec::new(), Vec::new());
    rounded(&mut at_zero, 0, 0, 40, 30, 9);
    rounded(&mut moved, -15, 22, 40, 30, 9);
    let shifted: Vec<Span> = at_zero
        .iter()
        .map(|&[x, y, w, h]| [x - 15, y + 22, w, h])
        .collect();
    assert_eq!(moved, shifted);
}

#[test]
fn corner_rows_with_one_inset_share_a_rectangle() {
    let mut square = Vec::new();
    rounded(&mut square, 0, 0, 50, 30, 0);
    assert_eq!(square, vec![[0, 0, 50, 30]]);
    // Each run of corner rows with one inset is a rectangle at the top
    // and one at the bottom; a pill (radius half its height) has no
    // middle band.
    for (w, h, r) in [(208, 36, 18), (200, 140, 12), (380, 90, 14)] {
        let inset = |j: i32| (0..w).find(|&x| inside(x, j, w, h, r)).unwrap();
        let runs = (0..r)
            .filter(|&j| j == 0 || inset(j) != inset(j - 1))
            .count();
        assert!(
            runs < r as usize,
            "{w}x{h} radius {r}: no two rows share an inset"
        );
        let mut rects = Vec::new();
        rounded(&mut rects, 0, 0, w, h, r);
        let band = usize::from(h > 2 * r);
        assert_eq!(
            rects.len(),
            2 * runs + band,
            "{w}x{h} radius {r}: {rects:?}"
        );
    }
}

#[test]
fn an_empty_rect_has_no_rectangles() {
    for (w, h) in [(0, 10), (10, 0), (-4, 10), (10, -4)] {
        let mut rects = Vec::new();
        rounded(&mut rects, 3, 3, w, h, 4);
        assert!(rects.is_empty(), "{w}x{h}: {rects:?}");
    }
}

#[test]
fn areas_that_meet_in_logical_pixels_meet_in_device_pixels() {
    // Two square areas side by side at a fractional scale: the first
    // ends in device pixels where the second begins.
    let left = Rounded {
        x: 10.3,
        y: 0.0,
        w: 33.3,
        h: 20.0,
        r: 0.0,
    };
    let right = Rounded { x: 43.6, ..left };
    for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
        let mut out = Vec::new();
        spans(&mut out, scale, &[left, right]);
        let [[x0, _, w0, _], [x1, _, _, _]] = out[..] else {
            panic!("two squares made {out:?}");
        };
        assert_eq!(x0 + w0, x1, "scale {scale}");
        assert_eq!(x0, (10.3 * scale).round() as i32, "scale {scale}");
    }
}

#[test]
fn a_radius_scales_with_its_rect() {
    let pill = Rounded {
        x: 16.0,
        y: 16.0,
        w: 208.0,
        h: 36.0,
        r: 18.0,
    };
    let mut out = Vec::new();
    spans(&mut out, 2.0, &[pill]);
    let mut want = Vec::new();
    rounded(&mut want, 32, 32, 416, 72, 36);
    assert_eq!(out, want);
}
