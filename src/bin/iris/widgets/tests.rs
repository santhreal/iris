// WHY: the class closed here is "the swizzle scrambles channels":
// every capture, thumbnail, and RenderImage passes through it, so a
// wrong lane silently recolors or drops alpha across the whole app.
// The involution case matters because the same function is applied
// symmetrically (RGBA->BGRA on the way in, BGRA->RGBA on the way out).
// Not covered: the banding that calls it, which par.rs tests own.

use super::swizzle_rgba_bgra;

#[test]
fn swizzle_swaps_red_and_blue_keeps_green_alpha() {
    let mut px = vec![0x11, 0x22, 0x33, 0x44];
    swizzle_rgba_bgra(&mut px);
    assert_eq!(px, vec![0x33, 0x22, 0x11, 0x44]);
}

#[test]
fn swizzle_is_an_involution() {
    let original: Vec<u8> = (0..64).map(|i| (i * 37 + 11) as u8).collect();
    let mut buf = original.clone();
    swizzle_rgba_bgra(&mut buf);
    swizzle_rgba_bgra(&mut buf);
    assert_eq!(buf, original);
}

#[test]
fn swizzle_handles_many_pixels() {
    // A multi-pixel buffer: each pixel swizzles independently, no
    // cross-lane bleed.
    let mut buf = vec![
        0xAA, 0xBB, 0xCC, 0xDD, // px0
        0x01, 0x02, 0x03, 0x04, // px1
        0xFF, 0x00, 0x80, 0x7F, // px2
    ];
    swizzle_rgba_bgra(&mut buf);
    assert_eq!(
        buf,
        vec![
            0xCC, 0xBB, 0xAA, 0xDD, // px0
            0x03, 0x02, 0x01, 0x04, // px1
            0x80, 0x00, 0xFF, 0x7F, // px2
        ]
    );
}
