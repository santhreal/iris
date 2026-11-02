//! A PNG's pixel density: its `pHYs` chunk, the number of pixels per
//! metre on each axis. A capture states the scale of the display it
//! came from, so a document or an image editor places it at the size it
//! showed on screen: a 2x capture at 192 pixels per inch, or 144 on
//! macOS, where a logical pixel is a 1/72 inch point.

use std::io::{Read, Seek, SeekFrom};

/// The PNG signature every file starts with.
const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Logical pixels per inch: a macOS point is 1/72 inch, a Windows DIP
/// and a Linux logical pixel 1/96.
const LOGICAL_PER_INCH: f32 = if cfg!(target_os = "macos") {
    72.0
} else {
    96.0
};

/// A `pHYs` payload: pixels per unit on x, then on y, as big-endian
/// u32s, then the unit, 1 for the metre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Density([u8; 9]);

impl Density {
    /// The density of a capture at `scale` root pixels per logical
    /// pixel, in pixels per metre. None for a scale that is not finite
    /// and positive.
    pub fn of_scale(scale: f32) -> Option<Density> {
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let per_metre = (LOGICAL_PER_INCH * scale / 0.0254).round() as u32;
        Some(Self::per_metre(per_metre, per_metre))
    }

    /// `x` and `y` pixels per metre.
    pub fn per_metre(x: u32, y: u32) -> Density {
        let mut payload = [1u8; 9];
        payload[0..4].copy_from_slice(&x.to_be_bytes());
        payload[4..8].copy_from_slice(&y.to_be_bytes());
        Density(payload)
    }

    /// The density the PNG in `r` states: its `pHYs` chunk, read from
    /// the chunks before the image data. None for a file that is not a
    /// PNG, states no density, or holds a `pHYs` chunk whose CRC does
    /// not match it. Reads the chunk headers and seeks past their data.
    pub fn read(r: &mut (impl Read + Seek)) -> Option<Density> {
        let mut signature = [0u8; 8];
        r.read_exact(&mut signature).ok()?;
        if &signature != SIGNATURE {
            return None;
        }
        loop {
            // Length, type, and for a pHYs chunk its payload and CRC.
            let mut head = [0u8; 8];
            r.read_exact(&mut head).ok()?;
            let [l0, l1, l2, l3, kind @ ..] = head;
            let len = u32::from_be_bytes([l0, l1, l2, l3]);
            match &kind {
                b"pHYs" if len == 9 => {
                    let mut chunk = [0u8; 4 + 9 + 4];
                    chunk[..4].copy_from_slice(&kind);
                    r.read_exact(&mut chunk[4..]).ok()?;
                    let (body, crc) = chunk.split_at(13);
                    if crc32fast::hash(body).to_be_bytes() != crc {
                        return None;
                    }
                    let mut payload = [0u8; 9];
                    payload.copy_from_slice(&body[4..]);
                    return Some(Density(payload));
                }
                // PNG places pHYs before the image data.
                b"IDAT" | b"IEND" => return None,
                _ => {
                    r.seek(SeekFrom::Current(i64::from(len) + 4)).ok()?;
                }
            }
        }
    }

    /// Insert this density into `png`, which an encoder wrote with IHDR
    /// first and no `pHYs` chunk: a `pHYs` chunk right after the IHDR.
    pub fn insert(self, png: &mut Vec<u8>) -> Result<(), String> {
        let ihdr_len = png
            .get(8..16)
            .filter(|h| &png[..8] == SIGNATURE && &h[4..] == b"IHDR")
            .map(|h| u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize)
            .ok_or("encoded PNG does not start with its IHDR chunk")?;
        // The signature, then IHDR's length, type, data, and CRC.
        let at = 8 + 4 + 4 + ihdr_len + 4;
        if png.len() < at {
            return Err("encoded PNG ends inside its IHDR chunk".to_string());
        }
        let mut chunk = [0u8; 4 + 4 + 9 + 4];
        chunk[..4].copy_from_slice(&9u32.to_be_bytes());
        chunk[4..8].copy_from_slice(b"pHYs");
        chunk[8..17].copy_from_slice(&self.0);
        let crc = crc32fast::hash(&chunk[4..17]);
        chunk[17..].copy_from_slice(&crc.to_be_bytes());
        png.splice(at..at, chunk);
        Ok(())
    }

    /// Pixels per metre on x and y, or None when the unit is not the
    /// metre.
    pub fn pixels_per_metre(self) -> Option<(u32, u32)> {
        let [x0, x1, x2, x3, y0, y1, y2, y3, unit] = self.0;
        (unit == 1).then(|| {
            (
                u32::from_be_bytes([x0, x1, x2, x3]),
                u32::from_be_bytes([y0, y1, y2, y3]),
            )
        })
    }
}

// WHY: the class closed here is "a capture states a density other than
// its display's, or a PNG reader takes it for damage": a scale that
// rounds to the wrong count or picks the wrong inch, a chunk whose CRC
// or place breaks the file for a decoder, and a reader that misses the
// chunk behind another or launders a damaged one into a new file.
// Not covered: which scale a capture path passes in; `tests/density.rs`
// reads that from the files the running daemon saves.
#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::prelude::v1::test;

    use super::*;

    /// A 3x2 PNG as the image encoder writes it.
    fn encoded() -> (image::RgbaImage, Vec<u8>) {
        let img = image::RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([x as u8 * 80, y as u8 * 90, 7, 255])
        });
        let mut png = Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        (img, png.into_inner())
    }

    /// The chunk types of `png`, in order.
    fn kinds(png: &[u8]) -> Vec<String> {
        let mut at = 8;
        let mut out = Vec::new();
        while at + 8 <= png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            out.push(String::from_utf8_lossy(&png[at + 4..at + 8]).into_owned());
            at += 12 + len;
        }
        out
    }

    #[test]
    fn a_scale_states_its_logical_pixels_per_inch_in_pixels_per_metre() {
        let inch = if cfg!(target_os = "macos") {
            72.0
        } else {
            96.0
        };
        for (scale, dpi) in [
            (1.0, inch),
            (1.25, inch * 1.25),
            (1.5, inch * 1.5),
            (2.0, inch * 2.0),
            (3.0, inch * 3.0),
        ] {
            let per_metre = (dpi / 0.0254_f64).round() as u32;
            assert_eq!(
                Density::of_scale(scale).unwrap().pixels_per_metre(),
                Some((per_metre, per_metre)),
                "scale {scale}"
            );
        }
        // The counts other software writes for 96 and 192 per inch, and
        // for 72 and 144 on macOS.
        let (one, two) = if cfg!(target_os = "macos") {
            (2835, 5669)
        } else {
            (3780, 7559)
        };
        assert_eq!(
            Density::of_scale(1.0).unwrap().pixels_per_metre(),
            Some((one, one))
        );
        assert_eq!(
            Density::of_scale(2.0).unwrap().pixels_per_metre(),
            Some((two, two))
        );
    }

    #[test]
    fn a_scale_that_is_not_finite_and_positive_states_no_density() {
        for scale in [0.0, -1.0, -0.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(Density::of_scale(scale), None, "scale {scale}");
        }
    }

    #[test]
    fn an_inserted_density_follows_the_ihdr_decodes_and_reads_back() {
        let (img, mut png) = encoded();
        let plain = png.clone();
        let density = Density::of_scale(2.0).unwrap();
        density.insert(&mut png).unwrap();
        let mut want = kinds(&plain);
        want.insert(1, "pHYs".to_string());
        assert_eq!(kinds(&png), want);
        assert_eq!(png.len(), plain.len() + 21);
        // The decoder checks every chunk's CRC: a wrong one fails here.
        let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
        assert_eq!(decoded, img);
        assert_eq!(Density::read(&mut Cursor::new(&png)), Some(density));
        assert_eq!(Density::read(&mut Cursor::new(&plain)), None);
    }

    #[test]
    fn a_density_behind_other_chunks_reads_back() {
        let (_, plain) = encoded();
        // A 300-byte private ancillary chunk right after the IHDR, then
        // the density after it.
        let mut png = plain.clone();
        let mut extra = Vec::new();
        extra.extend_from_slice(&300u32.to_be_bytes());
        extra.extend_from_slice(b"prVt");
        extra.extend_from_slice(&[0x55; 300]);
        let crc = crc32fast::hash(&extra[4..]);
        extra.extend_from_slice(&crc.to_be_bytes());
        png.splice(33..33, extra);
        let density = Density::per_metre(5000, 4000);
        density.insert(&mut png).unwrap();
        // insert puts it first; move it behind the private chunk.
        let phys: Vec<u8> = png.drain(33..54).collect();
        png.splice(33 + 312..33 + 312, phys);
        assert_eq!(kinds(&png)[..3], ["IHDR", "prVt", "pHYs"]);
        image::load_from_memory(&png).unwrap();
        assert_eq!(Density::read(&mut Cursor::new(&png)), Some(density));
        assert_eq!(density.pixels_per_metre(), Some((5000, 4000)));
    }

    #[test]
    fn a_damaged_or_missing_density_reads_as_none() {
        let (_, mut png) = encoded();
        Density::of_scale(2.0).unwrap().insert(&mut png).unwrap();
        // A flipped payload byte breaks the chunk's CRC.
        let mut damaged = png.clone();
        damaged[33 + 8] ^= 1;
        assert_eq!(Density::read(&mut Cursor::new(&damaged)), None);
        // Cut inside the chunk, before it, and a file that is no PNG.
        for cut in [33 + 10, 20, 8, 3, 0] {
            assert_eq!(
                Density::read(&mut Cursor::new(&png[..cut])),
                None,
                "cut at {cut}"
            );
        }
        let mut not_png = png.clone();
        not_png[1] = b'J';
        assert_eq!(Density::read(&mut Cursor::new(&not_png)), None);
    }

    #[test]
    fn a_density_after_the_image_data_is_not_read() {
        let (_, plain) = encoded();
        let mut png = plain.clone();
        Density::of_scale(2.0).unwrap().insert(&mut png).unwrap();
        let phys: Vec<u8> = png.drain(33..54).collect();
        // Just before IEND, after the image data.
        let iend = png.len() - 12;
        png.splice(iend..iend, phys);
        assert_eq!(kinds(&png).last().map(String::as_str), Some("IEND"));
        assert_eq!(Density::read(&mut Cursor::new(&png)), None);
    }

    #[test]
    fn insert_rejects_bytes_that_do_not_start_with_an_ihdr() {
        let (_, png) = encoded();
        for (what, mut bytes) in [
            ("empty", Vec::new()),
            ("signature only", png[..8].to_vec()),
            ("cut inside the IHDR", png[..20].to_vec()),
            ("no signature", png[8..].to_vec()),
        ] {
            assert!(
                Density::of_scale(1.0).unwrap().insert(&mut bytes).is_err(),
                "{what}"
            );
        }
    }

    /// A chunk that claims more bytes than the file holds ends the
    /// read: the seek past it lands beyond the end, where a read fails.
    #[test]
    fn a_reader_ends_on_a_chunk_that_claims_more_than_the_file() {
        let (_, plain) = encoded();
        let mut png = plain[..33].to_vec();
        png.extend_from_slice(&u32::MAX.to_be_bytes());
        png.extend_from_slice(b"prVt");
        assert_eq!(Density::read(&mut Cursor::new(&png)), None);
    }
}
