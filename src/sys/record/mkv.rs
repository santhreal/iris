//! Matroska framing for raw frames on ffmpeg's stdin.
//!
//! A segment's video reaches ffmpeg as uncompressed Matroska: one
//! header with the frame size and pixel layout, then one cluster per
//! frame whose timestamp is the frame's capture time in milliseconds.
//! ffmpeg reads the stream parameters from the header without probing
//! a frame, and every frame keeps its own timestamp: an unchanged
//! source writes no frame, and a dropped frame shifts no later one.

/// The memory layout of a captured frame. The capture path copies
/// server pixels as they are; the header declares the layout, and
/// ffmpeg converts once, inside the encode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PixFmt {
    /// B,G,R,X: 4 bytes a pixel (X11 TrueColor, PipeWire BGRx/BGRA).
    Bgrx,
    /// R,G,B,X: 4 bytes a pixel (GL readPixels, PipeWire RGBx/RGBA).
    Rgbx,
    /// B,G,R: 3 bytes a pixel (packed 24-bit X11 ZPixmap).
    Bgr24,
    /// R,G,B: 3 bytes a pixel.
    Rgb24,
}

impl PixFmt {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgrx | Self::Rgbx => 4,
            Self::Bgr24 | Self::Rgb24 => 3,
        }
    }

    /// The Matroska ColourSpace of this layout. ffmpeg reads it as
    /// bgr0, rgb0, bgr24, or rgb24: the fourth byte of a 4-byte pixel
    /// is padding, never alpha.
    fn fourcc(self) -> [u8; 4] {
        match self {
            Self::Bgrx => *b"BGR\0",
            Self::Rgbx => *b"RGB\0",
            Self::Bgr24 => [b'B', b'G', b'R', 24],
            Self::Rgb24 => [b'R', b'G', b'B', 24],
        }
    }
}

/// Stream header for `width`x`height` frames in `pix`, nominally
/// `fps` a second. Timestamps are milliseconds. The nominal rate sets
/// the last frame's duration and the rate a player reports; frame
/// times still come from the clusters.
pub fn header(width: u32, height: u32, pix: PixFmt, fps: u32) -> Vec<u8> {
    let ebml = [
        el(&[0x42, 0x86], &uint(1)),    // EBMLVersion
        el(&[0x42, 0xF7], &uint(1)),    // EBMLReadVersion
        el(&[0x42, 0xF2], &uint(4)),    // EBMLMaxIDLength
        el(&[0x42, 0xF3], &uint(8)),    // EBMLMaxSizeLength
        el(&[0x42, 0x82], b"matroska"), // DocType
        el(&[0x42, 0x87], &uint(4)),    // DocTypeVersion
        el(&[0x42, 0x85], &uint(2)),    // DocTypeReadVersion
    ]
    .concat();
    let info = [
        el(&[0x2A, 0xD7, 0xB1], &uint(1_000_000)), // TimestampScale: 1 ms
        el(&[0x4D, 0x80], b"iris"),                // MuxingApp
        el(&[0x57, 0x41], b"iris"),                // WritingApp
    ]
    .concat();
    let video = [
        el(&[0xB0], &uint(u64::from(width))),   // PixelWidth
        el(&[0xBA], &uint(u64::from(height))),  // PixelHeight
        el(&[0x2E, 0xB5, 0x24], &pix.fourcc()), // ColourSpace
    ]
    .concat();
    let track = [
        el(&[0xD7], &uint(1)),          // TrackNumber
        el(&[0x73, 0xC5], &uint(1)),    // TrackUID
        el(&[0x83], &uint(1)),          // TrackType: video
        el(&[0x9C], &uint(0)),          // FlagLacing
        el(&[0x86], b"V_UNCOMPRESSED"), // CodecID
        el(
            &[0x23, 0xE3, 0x83],
            &uint(1_000_000_000 / u64::from(fps.max(1))),
        ), // DefaultDuration, ns
        el(&[0xE0], &video),            // Video
    ]
    .concat();
    let mut out = el(&[0x1A, 0x45, 0xDF, 0xA3], &ebml); // EBML
                                                        // Segment of unknown size: the stream ends when the pipe closes.
    out.extend_from_slice(&[
        0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ]);
    out.extend(el(&[0x15, 0x49, 0xA9, 0x66], &info)); // Info
    out.extend(el(&[0x16, 0x54, 0xAE, 0x6B], &el(&[0xAE], &track))); // Tracks > TrackEntry
    out
}

/// The bytes that precede a `frame_len`-byte frame captured `ts_ms`
/// after the segment began: a Cluster holding that timestamp and one
/// keyframe SimpleBlock. Written into `out`, which is cleared first,
/// so the writer reuses one buffer for every frame.
pub fn cluster_head(ts_ms: u64, frame_len: usize, out: &mut Vec<u8>) {
    let ts = uint(ts_ms);
    // Timestamp element (id, size, value) + SimpleBlock id and size +
    // block header (track, relative time, flags), then the frame.
    let block_len = 4 + frame_len as u64;
    let cluster_len = 2 + ts.len() as u64 + 1 + 8 + block_len;
    out.clear();
    out.extend_from_slice(&[0x1F, 0x43, 0xB6, 0x75]); // Cluster
    out.extend_from_slice(&size8(cluster_len));
    out.push(0xE7); // Timestamp
    out.push(0x80 | ts.len() as u8);
    out.extend_from_slice(&ts);
    out.push(0xA3); // SimpleBlock
    out.extend_from_slice(&size8(block_len));
    // Track 1, relative timestamp 0, keyframe.
    out.extend_from_slice(&[0x81, 0x00, 0x00, 0x80]);
}

/// One element: id, the shortest size that holds `payload`, payload.
fn el(id: &[u8], payload: &[u8]) -> Vec<u8> {
    let n = payload.len();
    let mut out = Vec::with_capacity(id.len() + 2 + n);
    out.extend_from_slice(id);
    // All-ones sizes mean "unknown", so one byte holds up to 126.
    if n < 0x7F {
        out.push(0x80 | n as u8);
    } else {
        assert!(n < 0x3FFF, "header element of {n} bytes");
        out.extend_from_slice(&[0x40 | (n >> 8) as u8, n as u8]);
    }
    out.extend_from_slice(payload);
    out
}

/// An unsigned integer payload: big-endian, without leading zero bytes,
/// at least one byte.
fn uint(v: u64) -> Vec<u8> {
    let bytes = v.to_be_bytes();
    let skip = (v.leading_zeros() / 8).min(7) as usize;
    bytes[skip..].to_vec()
}

/// An 8-byte element size: fixed width, so a frame's size never
/// changes the head's length.
fn size8(n: u64) -> [u8; 8] {
    let mut b = n.to_be_bytes();
    b[0] = 0x01;
    b
}

// WHY: the class closed here is "ffmpeg reads a different stream than
// the capture wrote": a size field off by one byte, a timestamp in the
// wrong unit, or a layout declared as the wrong ffmpeg pixel format
// shifts every later frame or swaps channels. The expected bytes are
// the stream ffmpeg 6.1 decoded into exact frame times and sizes. Not
// covered: ffmpeg's reading of them (the recorder smoke test encodes).
#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter()
            .map(|x| format!("{x:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn header_matches_the_decoded_reference() {
        let want = "1a 45 df a3 a3 42 86 81 01 42 f7 81 01 42 f2 81 04 42 f3 81 08 \
            42 82 88 6d 61 74 72 6f 73 6b 61 42 87 81 04 42 85 81 02 \
            18 53 80 67 01 ff ff ff ff ff ff ff \
            15 49 a9 66 95 2a d7 b1 83 0f 42 40 4d 80 84 69 72 69 73 57 41 84 69 72 69 73 \
            16 54 ae 6b b8 ae b6 d7 81 01 73 c5 81 01 83 81 01 9c 81 00 \
            86 8e 56 5f 55 4e 43 4f 4d 50 52 45 53 53 45 44 \
            23 e3 83 84 01 fc a0 55 \
            e0 8f b0 82 01 40 ba 81 f0 2e b5 24 84 42 47 52 00";
        assert_eq!(hex(&header(320, 240, PixFmt::Bgrx, 30)), want);
    }

    #[test]
    fn every_layout_declares_its_ffmpeg_pixel_format() {
        // (layout, ColourSpace ffmpeg maps to bgr0/rgb0/bgr24/rgb24, bpp)
        let cases = [
            (PixFmt::Bgrx, *b"BGR\0", 4),
            (PixFmt::Rgbx, *b"RGB\0", 4),
            (PixFmt::Bgr24, *b"BGR\x18", 3),
            (PixFmt::Rgb24, *b"RGB\x18", 3),
        ];
        for (pix, cs, bpp) in cases {
            assert_eq!((pix.fourcc(), pix.bytes_per_pixel()), (cs, bpp), "{pix:?}");
            let h = header(2, 2, pix, 30);
            assert!(
                h.ends_with(&[&[0x2E, 0xB5, 0x24, 0x84][..], &cs].concat()),
                "{pix:?}"
            );
        }
    }

    #[test]
    fn cluster_heads_match_the_decoded_reference() {
        let mut out = Vec::new();
        let frame = 320 * 240 * 4;
        let cases = [
            (0, frame, "1f 43 b6 75 01 00 00 00 00 04 b0 10 e7 81 00 a3 01 00 00 00 00 04 b0 04 81 00 00 80"),
            (1234, frame, "1f 43 b6 75 01 00 00 00 00 04 b0 11 e7 82 04 d2 a3 01 00 00 00 00 04 b0 04 81 00 00 80"),
            (70_000, 1920 * 1080 * 4, "1f 43 b6 75 01 00 00 00 00 7e 90 12 e7 83 01 11 70 a3 01 00 00 00 00 7e 90 04 81 00 00 80"),
        ];
        for (ts, len, want) in cases {
            cluster_head(ts, len, &mut out);
            assert_eq!(hex(&out), want, "ts {ts}");
        }
    }

    #[test]
    fn cluster_size_counts_every_byte_after_its_size_field() {
        // Cluster content = everything after the 4-byte id and 8-byte
        // size, frame included, at every timestamp width.
        let mut out = Vec::new();
        for ts in [0, 255, 256, 65_535, 65_536, 1 << 40, u64::MAX] {
            cluster_head(ts, 1000, &mut out);
            let size =
                u64::from_be_bytes([0, out[5], out[6], out[7], out[8], out[9], out[10], out[11]]);
            assert_eq!(size, (out.len() - 12 + 1000) as u64, "ts {ts}");
        }
    }

    #[test]
    fn nominal_rate_sets_default_duration() {
        for (fps, ns) in [
            (30, 33_333_333u64),
            (60, 16_666_666),
            (20, 50_000_000),
            (0, 1_000_000_000),
        ] {
            let h = header(2, 2, PixFmt::Rgb24, fps);
            let at = h.windows(3).position(|w| w == [0x23, 0xE3, 0x83]).unwrap();
            let n = usize::from(h[at + 3] & 0x7F);
            let v = h[at + 4..at + 4 + n]
                .iter()
                .fold(0u64, |a, &b| a << 8 | u64::from(b));
            assert_eq!(v, ns, "{fps} fps");
        }
    }
}
