// WHY: the class closed here is "the compositor and iris disagree on
// the frame": a format offered to the compositor that one read path
// cannot take, a DRM fourcc that swaps channels on the EGL import, an
// offer without the buffer types or the rate cap the stream relies on,
// and a row copy that keeps stride padding, reads past the buffer, or
// splits rows across threads wrongly. The offered formats are read back
// from the serialized offer, so a format added to it without a decision
// here fails. Not covered: a live negotiation and the EGL readback
// (the Wayland recording smoke test covers both).

use pipewire::spa::buffer::DataType;
use pipewire::spa::param::format::FormatProperties;
use pipewire::spa::param::video::VideoFormat;
use pipewire::spa::param::ParamType;
use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::{ChoiceValue, Object, Value};
use pipewire::spa::utils::{Choice, ChoiceEnum, SpaTypes};

use super::gl::{DRM_FORMAT_MOD_INVALID, DRM_FORMAT_MOD_LINEAR};
use super::stream::{build_param_pods, copy_frame, drm_fourcc_for_format};
use crate::record::mkv::PixFmt;

/// The DRM fourcc (the format's bytes as a little-endian word) and the
/// byte order each accepted format records as.
fn expected(format: VideoFormat) -> ([u8; 4], PixFmt) {
    match format {
        VideoFormat::BGRx => (*b"XR24", PixFmt::Bgrx),
        VideoFormat::BGRA => (*b"AR24", PixFmt::Bgrx),
        VideoFormat::RGBx => (*b"XB24", PixFmt::Rgbx),
        VideoFormat::RGBA => (*b"AB24", PixFmt::Rgbx),
        other => panic!("{other:?} is offered without an expected fourcc and layout"),
    }
}

/// The offer's objects, in order: CPU format, DMA-buf format, buffers.
fn offer(fps: u32) -> Vec<Object> {
    let pods = build_param_pods(fps).expect("serialize the offer");
    pods.iter()
        .enumerate()
        .map(
            |(i, bytes)| match PodDeserializer::deserialize_any_from(bytes) {
                Ok((_, Value::Object(obj))) => obj,
                _ => panic!("offer pod {i} is not an object"),
            },
        )
        .collect()
}

fn prop(obj: &Object, key: FormatProperties) -> Option<&Value> {
    let key = key.as_raw();
    obj.properties
        .iter()
        .find(|p| p.key == key)
        .map(|p| &p.value)
}

/// The video formats one format object offers.
fn formats(obj: &Object) -> Vec<VideoFormat> {
    match prop(obj, FormatProperties::VideoFormat) {
        Some(Value::Choice(ChoiceValue::Id(Choice(_, ChoiceEnum::Enum { alternatives, .. })))) => {
            alternatives.iter().map(|id| VideoFormat(id.0)).collect()
        }
        other => panic!("format object without a format choice: {other:?}"),
    }
}

#[test]
fn every_offered_format_reads_on_both_paths() {
    let objs = offer(30);
    let offered: Vec<VideoFormat> = objs[..2].iter().flat_map(formats).collect();
    assert!(!offered.is_empty());
    for format in offered {
        let (fourcc, pix) = expected(format);
        assert_eq!(
            drm_fourcc_for_format(format),
            Ok(u32::from_le_bytes(fourcc)),
            "{format:?} imports as the wrong DRM format"
        );
        let src = [1, 2, 3, 4];
        let mut out = Vec::new();
        assert_eq!(copy_frame(&src, 4, 1, 1, format, &mut out), Ok(pix));
        assert_eq!(out, src, "{format:?} bytes changed in the copy");
    }
}

#[test]
fn formats_outside_the_offer_are_rejected() {
    for format in [VideoFormat::xRGB, VideoFormat::YUY2, VideoFormat::I420] {
        assert!(drm_fourcc_for_format(format).is_err(), "{format:?}");
        let mut out = Vec::new();
        assert!(copy_frame(&[0; 4], 4, 1, 1, format, &mut out).is_err());
    }
}

#[test]
fn the_offer_caps_the_rate_and_takes_every_buffer_type() {
    for (fps, cap) in [(0, 1), (30, 30), (60, 60)] {
        let objs = offer(fps);
        assert_eq!(objs.len(), 3);
        for obj in &objs[..2] {
            assert_eq!(obj.type_, SpaTypes::ObjectParamFormat.as_raw());
            assert_eq!(obj.id, ParamType::EnumFormat.as_raw());
            match prop(obj, FormatProperties::VideoMaxFramerate) {
                Some(Value::Choice(ChoiceValue::Fraction(Choice(
                    _,
                    ChoiceEnum::Range { min, max, .. },
                )))) => {
                    assert_eq!((min.num, min.denom), (1, 1));
                    assert_eq!((max.num, max.denom), (cap, 1), "fps {fps}");
                }
                other => panic!("no max framerate range: {other:?}"),
            }
        }
        // CPU memory: no modifier. DMA-buf: implicit or linear.
        assert!(prop(&objs[0], FormatProperties::VideoModifier).is_none());
        match prop(&objs[1], FormatProperties::VideoModifier) {
            Some(Value::Choice(ChoiceValue::Long(Choice(
                _,
                ChoiceEnum::Enum { alternatives, .. },
            )))) => {
                let mut mods = alternatives.clone();
                mods.sort_unstable();
                let mut want = [DRM_FORMAT_MOD_LINEAR as i64, DRM_FORMAT_MOD_INVALID as i64];
                want.sort_unstable();
                assert_eq!(mods, want);
            }
            other => panic!("no modifier choice on the DMA-buf format: {other:?}"),
        }
        let buffers = &objs[2];
        assert_eq!(buffers.type_, SpaTypes::ObjectParamBuffers.as_raw());
        let data_type = buffers
            .properties
            .iter()
            .find(|p| p.key == pipewire::spa::sys::SPA_PARAM_BUFFERS_dataType)
            .map(|p| &p.value);
        let Some(Value::Int(mask)) = data_type else {
            panic!("no buffer data type mask: {data_type:?}");
        };
        for kind in [DataType::MemPtr, DataType::MemFd, DataType::DmaBuf] {
            assert_ne!(mask & (1 << kind.as_raw()), 0, "{kind:?} not accepted");
        }
    }
}

/// A `width` x `height` BGRx frame at `stride`: every pixel byte
/// distinct per position, the padding 0xEE. Returns it and its rows
/// packed.
fn padded(width: usize, height: usize, stride: usize) -> (Vec<u8>, Vec<u8>) {
    let row = width * 4;
    let mut src = vec![0xEE; stride * height];
    let mut packed = Vec::with_capacity(row * height);
    for r in 0..height {
        for c in 0..row {
            let byte = ((r * 31 + c * 7) % 251) as u8;
            src[r * stride + c] = byte;
            packed.push(byte);
        }
    }
    (src, packed)
}

#[test]
fn stride_padding_is_dropped() {
    let (src, packed) = padded(3, 2, 16);
    // A reused buffer: stale bytes and length from an earlier frame.
    let mut out = vec![0x55; 100];
    let pix = copy_frame(&src, 16, 3, 2, VideoFormat::BGRx, &mut out);
    assert_eq!(pix, Ok(PixFmt::Bgrx));
    assert_eq!(out, packed);
    // The last row needs no padding after it.
    let tight = &src[..16 + 12];
    assert_eq!(
        copy_frame(tight, 16, 3, 2, VideoFormat::BGRx, &mut out),
        Ok(PixFmt::Bgrx)
    );
    assert_eq!(out, packed);
    // Stride 0: the rows are packed.
    assert_eq!(
        copy_frame(&packed, 0, 3, 2, VideoFormat::BGRx, &mut out),
        Ok(PixFmt::Bgrx)
    );
    assert_eq!(out, packed);
}

#[test]
fn short_or_empty_frames_are_rejected() {
    let (src, _) = padded(3, 2, 16);
    let mut out = Vec::new();
    let cases = [
        (&src[..16 + 11], 16, 3, 2, "one byte short"),
        (&src[..], 8, 3, 2, "stride under a row"),
        (&src[..], 16, 0, 2, "no width"),
        (&src[..], 16, 3, 0, "no height"),
        (&src[..0], 0, 3, 2, "no bytes"),
    ];
    for (bytes, stride, w, h, case) in cases {
        let got = copy_frame(bytes, stride, w, h, VideoFormat::BGRx, &mut out);
        assert!(got.is_err(), "{case}: {got:?}");
    }
}

#[test]
fn a_large_frame_copies_every_row_across_threads() {
    // Over the 1 MiB mark where the copy splits rows across threads.
    let (w, h, stride) = (1000, 300, 4096);
    let (src, packed) = padded(w, h, stride);
    let mut out = Vec::new();
    let pix = copy_frame(
        &src,
        stride,
        w as u32,
        h as u32,
        VideoFormat::RGBA,
        &mut out,
    );
    assert_eq!(pix, Ok(PixFmt::Rgbx));
    assert_eq!(out.len(), packed.len());
    if let Some(at) = out.iter().zip(&packed).position(|(a, b)| a != b) {
        panic!("row {} differs at byte {}", at / (w * 4), at % (w * 4));
    }
}
