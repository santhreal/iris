use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use pipewire::spa::buffer::DataType;
use pipewire::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pipewire::spa::param::video::{VideoFormat, VideoInfoRaw};
use pipewire::spa::param::ParamType;
use pipewire::spa::pod;
use pipewire::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, SpaTypes};
use pipewire::stream::StreamRef;

use super::gl::{GlContext, DRM_FORMAT_MOD_INVALID, DRM_FORMAT_MOD_LINEAR};
use crate::record::encoder::{Encoder, EncoderConfig, PixFmt};
use crate::record::{ext_of, unique_recording_path};

const fn fourcc_code(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

pub(crate) fn drm_fourcc_for_format(format: VideoFormat) -> Result<u32, String> {
    match format {
        VideoFormat::BGRx => Ok(fourcc_code(b'X', b'R', b'2', b'4')),
        VideoFormat::BGRA => Ok(fourcc_code(b'A', b'R', b'2', b'4')),
        VideoFormat::RGBx => Ok(fourcc_code(b'X', b'B', b'2', b'4')),
        VideoFormat::RGBA => Ok(fourcc_code(b'A', b'B', b'2', b'4')),
        other => Err(format!("no DRM fourcc mapping for format {other:?}")),
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PlaneInfo {
    pub(crate) fd: i32,
    pub(crate) offset: u32,
    pub(crate) stride: u32,
}

pub(crate) struct Shared {
    pub(crate) encoder: Option<Encoder>,
    /// The (size, pixel format) the live encoder was opened with; a
    /// renegotiated stream splits the file rather than dying on a
    /// frame-size mismatch or swapping channels mid-stream.
    pub(crate) enc_fmt: (u32, u32, PixFmt),
    /// The path of the segment currently being written: splits rename
    /// the output, and the caller reports the last one.
    pub(crate) output: PathBuf,
    pub(crate) error: Option<String>,
    pub(crate) quit: Option<pipewire::main_loop::WeakMainLoop>,
    /// Set once the encoder is taken for finish(): on_process can fire
    /// once more as the loop drains, and must not respawn ffmpeg.
    pub(crate) done: bool,
}

pub(crate) struct StreamData {
    pub(crate) fps: u32,
    pub(crate) mic: bool,
    pub(crate) rec_format: crate::config::RecordingFormat,
    pub(crate) rec_encoder: crate::config::RecordingEncoder,
    pub(crate) format: Option<(u32, u32, VideoFormat, u64)>,
    pub(crate) shared: Rc<RefCell<Shared>>,
    pub(crate) unsupported_reported: bool,
    pub(crate) frames: u64,
    pub(crate) gl_context: Option<GlContext>,
    pub(crate) scratch: Vec<u8>,
    /// Plane descriptors for the DMA-buf import, rebuilt per frame
    /// into a reused Vec: an allocation per frame on the RT thread
    /// is a needless syscall in the capture path.
    pub(crate) planes: Vec<PlaneInfo>,
}

impl StreamData {
    pub(crate) fn quit(&self) {
        let mainloop = self.shared.borrow().quit.as_ref().and_then(|w| w.upgrade());
        if let Some(mainloop) = mainloop {
            mainloop.quit();
        }
    }

    pub(crate) fn fail(&mut self, error: String) {
        self.shared.borrow_mut().error = Some(error);
        self.quit();
    }
}

pub(crate) fn on_process(stream: &StreamRef, data: &mut StreamData) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let datas = buffer.datas_mut();
    let Some(first) = datas.first_mut() else {
        return;
    };

    // The frame's pixel format, decided by which path produced it:
    // GL readPixels is always rgba; a CPU-mappable buffer keeps the
    // negotiated format.
    let mut pix_fmt = PixFmt::Rgba;
    match first.type_() {
        DataType::MemPtr | DataType::MemFd => {
            let Some((width, height, format, _modifier)) = data.format else {
                return;
            };
            let (offset, stride, size) = {
                let chunk = first.chunk();
                (
                    chunk.offset() as usize,
                    chunk.stride() as usize,
                    chunk.size() as usize,
                )
            };
            let Some(buf) = first.data() else {
                return;
            };
            let end = offset.saturating_add(size).min(buf.len());
            let src = &buf[offset.min(end)..end];

            match copy_frame(src, stride, width, height, format, &mut data.scratch) {
                Ok(f) => pix_fmt = f,
                Err(e) => {
                    data.fail(e);
                    return;
                }
            }
        }
        DataType::DmaBuf => {
            let Some((width, height, format, modifier)) = data.format else {
                return;
            };
            if let Some(gl) = &mut data.gl_context {
                data.planes.clear();
                for d in datas.iter() {
                    let raw = d.as_raw();
                    let chunk = d.chunk();
                    data.planes.push(PlaneInfo {
                        fd: raw.fd as i32,
                        offset: chunk.offset() + raw.mapoffset,
                        stride: chunk.stride() as u32,
                    });
                }
                match gl.read_dma_buf(
                    width,
                    height,
                    format,
                    modifier,
                    &data.planes,
                    &mut data.scratch,
                ) {
                    Ok(true) => {}
                    // The PBO pipeline primed but produced no pixels
                    // this call; skip the encode for it.
                    Ok(false) => return,
                    Err(e) => {
                        data.fail(format!("DMA-buf EGL import failed: {e}"));
                        return;
                    }
                }
            } else {
                let (offset, stride, size) = {
                    let chunk = first.chunk();
                    (
                        chunk.offset() as usize,
                        chunk.stride() as usize,
                        chunk.size() as usize,
                    )
                };
                if let Some(buf) = first.data() {
                    let end = offset.saturating_add(size).min(buf.len());
                    let src = &buf[offset.min(end)..end];

                    match copy_frame(src, stride, width, height, format, &mut data.scratch) {
                        Ok(f) => pix_fmt = f,
                        Err(e) => {
                            data.fail(e);
                            return;
                        }
                    }
                } else {
                    if !data.unsupported_reported {
                        data.unsupported_reported = true;
                        data.fail(format!(
                            "compositor offers only DMA-buf buffers (modifier {modifier:#x}); EGL is unavailable and buffer is not CPU-mappable"
                        ));
                    }
                    return;
                }
            }
        }
        other => {
            if !data.unsupported_reported {
                data.unsupported_reported = true;
                data.fail(format!(
                    "compositor offers only {other:?} buffers; iris's Wayland path needs MemPtr, MemFd, or DMA-buf"
                ));
            }
            return;
        }
    }

    let Some((width, height, _format, _modifier)) = data.format else {
        return;
    };

    // try_borrow_mut: teardown disconnects the stream first, but a
    // frame already queued on the data thread can still arrive while
    // the main thread holds the borrow. Dropping it beats aborting
    // the process on a RefCell panic in a non-unwinding callback.
    let Ok(mut shared) = data.shared.try_borrow_mut() else {
        return;
    };
    if shared.done {
        return;
    }
    if shared.encoder.is_none() {
        match Encoder::start(&EncoderConfig {
            output: shared.output.clone(),
            width,
            height,
            fps: data.fps,
            mic: data.mic,
            format: data.rec_format,
            encoder: data.rec_encoder,
            pix_fmt,
        }) {
            Ok(encoder) => {
                shared.enc_fmt = (width, height, pix_fmt);
                shared.encoder = Some(encoder);
            }
            Err(e) => {
                drop(shared);
                data.fail(e);
                return;
            }
        }
    } else if shared.enc_fmt != (width, height, pix_fmt) {
        // The compositor renegotiated (window resized): finish this
        // segment and open the next at the new size, the same split
        // the X11 loop performs on a resize.
        let old = shared.encoder.take().unwrap();
        drop(shared);
        if let Err(e) = old.finish() {
            data.fail(format!("encoder split on resize: {e}"));
            return;
        }
        let next = {
            let mut shared = data.shared.borrow_mut();
            let dir = shared
                .output
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            shared.output = unique_recording_path(&dir, ext_of(&shared.output));
            shared.output.clone()
        };
        match Encoder::start(&EncoderConfig {
            output: next,
            width,
            height,
            fps: data.fps,
            mic: data.mic,
            format: data.rec_format,
            encoder: data.rec_encoder,
            pix_fmt,
        }) {
            Ok(encoder) => {
                let mut shared = data.shared.borrow_mut();
                shared.enc_fmt = (width, height, pix_fmt);
                shared.encoder = Some(encoder);
            }
            Err(e) => {
                data.fail(e);
                return;
            }
        }
        return;
    }
    let frame = std::mem::replace(
        &mut data.scratch,
        shared.encoder.as_mut().unwrap().take_buf(),
    );
    let write_result = shared.encoder.as_mut().unwrap().write_frame(frame);
    drop(shared);
    data.frames += 1;
    if data.frames == 1 {
        crate::ilog!("iris: record: first frame written");
    }
    if let Err(e) = write_result {
        data.fail(e);
    }
}

/// Copy one PipeWire frame into `out` (resized to w*h*4), returning
/// the pixel format the encoder must declare. `out` is reused across
/// frames so recording does not allocate a multi-MB buffer per frame.
/// No swizzle: ffmpeg accepts the native layout through -pix_fmt.
pub(crate) fn copy_frame(
    src: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    format: VideoFormat,
    out: &mut Vec<u8>,
) -> Result<PixFmt, String> {
    let (w, h) = (width as usize, height as usize);
    let stride = if stride == 0 { w * 4 } else { stride };
    if src.len() < stride * h {
        return Err(format!(
            "short frame buffer: {} bytes for {w}x{h} at stride {stride}",
            src.len()
        ));
    }
    let pix_fmt = match format {
        VideoFormat::BGRx | VideoFormat::BGRA => PixFmt::Bgra,
        VideoFormat::RGBx | VideoFormat::RGBA => PixFmt::Rgba,
        other => {
            return Err(format!(
                "unsupported negotiated pixel format {other:?}; expected BGRx/BGRA/RGBx/RGBA"
            ))
        }
    };
    out.clear();
    out.reserve(w * h * 4);
    // Uninit capacity, not a zeroed vec: the banded row copy writes
    // every byte, and a resize's memset before the copy is a wasted
    // pass per frame.
    #[allow(clippy::uninit_vec)]
    unsafe {
        out.set_len(w * h * 4)
    };
    // Strided rows into a packed buffer, banded across threads once
    // the frame is large enough to pay for the spawn.
    crate::par::par_bands_mut(out, w * 4, |o_chunk, start| {
        let row0 = start / (w * 4);
        for (r, row_out) in o_chunk.chunks_exact_mut(w * 4).enumerate() {
            let row_in = &src[(row0 + r) * stride..(row0 + r) * stride + w * 4];
            row_out.copy_from_slice(row_in);
        }
    });
    Ok(pix_fmt)
}

pub(crate) fn on_param_changed(
    _stream: &StreamRef,
    data: &mut StreamData,
    id: u32,
    param: Option<&pod::Pod>,
) {
    if id != ParamType::Format.as_raw() {
        return;
    }
    let Some(param) = param else { return };
    let mut info = VideoInfoRaw::new();
    if info.parse(param).is_err() {
        return;
    }
    let size = info.size();
    let format = info.format();
    let modifier = info.modifier();
    data.format = Some((size.width, size.height, format, modifier));
    crate::ilog!(
        "iris: record: negotiated format {}x{} {:?} modifier {:#x}",
        size.width,
        size.height,
        format,
        modifier
    );
}

pub(crate) fn build_param_pods() -> Result<Vec<Vec<u8>>, String> {
    let obj_raw = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
    );

    let obj_dma = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        pod::Property::new(
            FormatProperties::VideoModifier.as_raw(),
            pod::Value::Choice(pod::ChoiceValue::Long(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Enum {
                    default: DRM_FORMAT_MOD_INVALID as i64,
                    alternatives: vec![DRM_FORMAT_MOD_INVALID as i64, DRM_FORMAT_MOD_LINEAR as i64,],
                },
            ))),
        )
    );

    let buffers_obj = pod::object!(
        SpaTypes::ObjectParamBuffers,
        ParamType::Buffers,
        pod::Property::new(
            pipewire::spa::sys::SPA_PARAM_BUFFERS_dataType,
            pod::Value::Int(
                (1 << DataType::MemPtr.as_raw())
                    | (1 << DataType::MemFd.as_raw())
                    | (1 << DataType::DmaBuf.as_raw())
            ),
        ),
    );

    let mut params = Vec::new();
    for obj in [obj_raw, obj_dma, buffers_obj] {
        let values: Vec<u8> = pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &pod::Value::Object(obj),
        )
        .map_err(|e| format!("serialize format params: {e}"))?
        .0
        .into_inner();
        params.push(values);
    }
    Ok(params)
}
