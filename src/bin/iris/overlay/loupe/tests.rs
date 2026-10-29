use std::sync::Arc;

use gpui::RenderImage;

use super::{loupe_image, LOUPE_PX, LOUPE_ZOOM};

/// A `side` x `side` frame of one BGRA color.
fn frame(side: u32, bgra: [u8; 4]) -> Arc<RenderImage> {
    let bytes = bgra.repeat((side * side) as usize);
    let buf = image::RgbaImage::from_raw(side, side, bytes).unwrap();
    Arc::new(RenderImage::new([image::Frame::new(buf)]))
}

/// The BGRA pixel at (x, y) of a loupe image.
fn at(img: &RenderImage, x: u32, y: u32) -> [u8; 4] {
    let d = ((y * LOUPE_PX + x) * 4) as usize;
    img.as_bytes(0).unwrap()[d..d + 4].try_into().unwrap()
}

/// Relative luminance of a BGRA pixel, 0 to 255.
fn luma([b, g, r, _]: [u8; 4]) -> f32 {
    0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)
}

#[test]
fn the_center_box_stands_off_the_pixel_it_marks() {
    let edge = LOUPE_PX / 2 - LOUPE_ZOOM / 2;
    for page in [
        [0xfc, 0xfa, 0xfa, 255],
        [255, 255, 255, 255],
        [0, 0xd7, 0xff, 255],
        [0x14, 0x14, 0x16, 255],
        [0, 0, 0, 255],
        [0xe6, 0x5c, 0x1e, 255],
    ] {
        let mut scratch = Vec::new();
        let (img, info) = loupe_image(&frame(40, page), 40, 40, (20, 20), (0, 0), &mut scratch);
        assert!(info.ends_with(&format!("#{:02x}{:02x}{:02x}", page[2], page[1], page[0])));
        let mark = at(&img, edge, edge);
        assert!(
            (luma(mark) - luma(page)).abs() > 100.0,
            "box {mark:?} over {page:?}"
        );
        // The box is on the center cell's edge; the cell's inside and
        // its neighbors show the pixel.
        assert_eq!(at(&img, edge + LOUPE_ZOOM / 2, edge + LOUPE_ZOOM / 2), page);
        assert_eq!(at(&img, edge - 1, edge - 1), page);
    }
}

// WHY: the frozen frame starts at the virtual screen's origin, which is
// negative with a monitor left of or above the primary on Windows and
// macOS. The loupe magnifies the frame pixel under the cursor and its
// info line states that pixel in root coordinates, as the crosshair
// readout does. Not covered: the cursor-to-frame mapping, which
// `overlay/tests.rs` pins.
#[test]
fn the_loupe_reads_the_frame_pixel_and_states_it_in_root_coordinates() {
    let mut bytes = [0x20, 0x20, 0x20, 255].repeat(40 * 40);
    let marked = (7 * 40 + 5) * 4;
    bytes[marked..marked + 4].copy_from_slice(&[0x30, 0x60, 0x90, 255]);
    let buf = image::RgbaImage::from_raw(40, 40, bytes).unwrap();
    let frame = Arc::new(RenderImage::new([image::Frame::new(buf)]));
    let mut scratch = Vec::new();
    let (img, info) = loupe_image(&frame, 40, 40, (5, 7), (-1920, -40), &mut scratch);
    assert_eq!(info.as_ref(), "-1915, -33  #906030");
    let inside = LOUPE_PX / 2;
    assert_eq!(at(&img, inside, inside), [0x30, 0x60, 0x90, 255]);
}
