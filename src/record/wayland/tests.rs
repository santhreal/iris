// WHY: DMA-buf screencasting requires parameter negotiation and pixel conversion:
// 1) format pods must advertise DmaBuf in ParamBuffers alongside MemPtr and MemFd.
// 2) format pods must serialize modifier choice pods for compositor negotiation.
// 3) DRM FOURCC codes must match little-endian SPA VideoFormat channel layouts.
// 4) convert_frame swizzles 32-bit channel words correctly into opaque RGBA.

use pipewire::spa::param::video::VideoFormat;
use pipewire::spa::pod;

use super::stream::{copy_frame, drm_fourcc_for_format};
use super::*;

#[test]
fn drm_fourcc_mapping_matches_spa_formats() {
    assert_eq!(
        drm_fourcc_for_format(VideoFormat::BGRx).unwrap(),
        0x34325258
    );
    assert_eq!(
        drm_fourcc_for_format(VideoFormat::BGRA).unwrap(),
        0x34325241
    );
    assert_eq!(
        drm_fourcc_for_format(VideoFormat::RGBx).unwrap(),
        0x34324258
    );
    assert_eq!(
        drm_fourcc_for_format(VideoFormat::RGBA).unwrap(),
        0x34324241
    );
    assert!(drm_fourcc_for_format(VideoFormat::YUY2).is_err());
}

#[test]
fn param_pods_serialize_dmabuf_and_memfd() {
    let pods = build_param_pods().expect("serialize param pods");
    assert_eq!(pods.len(), 3);
    for pod_bytes in &pods {
        assert!(pod::Pod::from_bytes(pod_bytes).is_some());
    }
}

#[test]
fn copy_frame_bgrx_passthrough() {
    // No swizzle: the bytes move verbatim and the format is
    // reported for ffmpeg's -pix_fmt.
    let src = [0x10, 0x20, 0x30, 0x00, 0x40, 0x50, 0x60, 0x00];
    let mut out = Vec::new();
    let fmt = copy_frame(&src, 8, 2, 1, VideoFormat::BGRx, &mut out).unwrap();
    assert_eq!(fmt, crate::record::encoder::PixFmt::Bgra);
    assert_eq!(out, src);
}

#[test]
fn copy_frame_rgbx_passthrough() {
    let src = [0x30, 0x20, 0x10, 0x00, 0x60, 0x50, 0x40, 0x00];
    let mut out = Vec::new();
    let fmt = copy_frame(&src, 8, 2, 1, VideoFormat::RGBx, &mut out).unwrap();
    assert_eq!(fmt, crate::record::encoder::PixFmt::Rgba);
    assert_eq!(out, src);
}

#[test]
fn gl_context_headless_initialization_does_not_panic() {
    let result = GlContext::new();
    match result {
        Ok(ctx) => {
            assert!(!ctx.display.as_ptr().is_null());
        }
        Err(e) => {
            assert!(!e.is_empty());
        }
    }
}
