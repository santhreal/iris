//! The PipeWire side of a Wayland recording: format negotiation, each
//! buffer read into packed rows for the recorder, and the controls read
//! on each ring. The stream callbacks and the ring run on the main
//! loop's thread, one at a time.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

use pipewire::main_loop::WeakMainLoop;
use pipewire::spa::buffer::{ChunkFlags, Data, DataType};
use pipewire::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pipewire::spa::param::video::{VideoFormat, VideoInfoRaw};
use pipewire::spa::param::ParamType;
use pipewire::spa::pod;
use pipewire::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, SpaTypes};
use pipewire::stream::{StreamRef, StreamState};

use super::gl::{GlContext, DRM_FORMAT_MOD_INVALID, DRM_FORMAT_MOD_LINEAR};
use crate::record::mkv::PixFmt;
use crate::record::recorder::{Recorder, Shape};
use crate::record::wake::Wake;
use crate::record::RecControl;

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

/// The video format the compositor negotiated.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Negotiated {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: VideoFormat,
    pub(crate) modifier: u64,
}

/// State the stream callbacks and the control timer share.
pub(crate) struct Shared {
    /// Taken at teardown to finish the recording.
    pub(crate) recorder: Option<Recorder>,
    pub(crate) error: Option<String>,
    pub(crate) quit: WeakMainLoop,
}

impl Shared {
    /// Return from the main loop; teardown then finishes the recording,
    /// failed with `error` when one is given. The first error stays.
    pub(crate) fn end(&mut self, error: Option<String>) {
        if self.error.is_none() {
            self.error = error;
        }
        if let Some(mainloop) = self.quit.upgrade() {
            mainloop.quit();
        }
    }
}

pub(crate) struct StreamData {
    pub(crate) format: Option<Negotiated>,
    pub(crate) shared: Rc<RefCell<Shared>>,
    pub(crate) reader: Reader,
    pub(crate) frames: u64,
}

/// Reads PipeWire buffers into packed rows: a DMA-buf through the EGL
/// import when a context exists, a CPU-mapped buffer by copy.
pub(crate) struct Reader {
    gl: Option<GlContext>,
    /// Plane descriptors of the DMA-buf import, refilled per frame.
    planes: Vec<PlaneInfo>,
    /// The first buffer's type is logged, so a run records which path
    /// it exercised.
    logged: bool,
}

impl Reader {
    pub(crate) fn new(gl: Option<GlContext>) -> Self {
        Self {
            gl,
            planes: Vec::with_capacity(4),
            logged: false,
        }
    }

    /// Read the frame in `datas` into `out`; the layout of the result,
    /// or `None` when the buffer holds no picture: the compositor
    /// queued it without drawing into it.
    fn read(
        &mut self,
        datas: &mut [Data],
        f: Negotiated,
        out: &mut Vec<u8>,
    ) -> Result<Option<PixFmt>, String> {
        let kind = datas
            .first()
            .map(Data::type_)
            .ok_or("PipeWire buffer without data")?;
        if !self.logged {
            self.logged = true;
            crate::ilog!("iris: record: first buffer type {kind:?}");
        }
        if datas
            .iter()
            .any(|d| d.chunk().flags().contains(ChunkFlags::CORRUPTED))
        {
            return Ok(None);
        }
        match (kind, &mut self.gl) {
            (DataType::DmaBuf, Some(gl)) => {
                self.planes.clear();
                self.planes.extend(datas.iter().map(|d| {
                    let (raw, chunk) = (d.as_raw(), d.chunk());
                    PlaneInfo {
                        fd: raw.fd as i32,
                        offset: chunk.offset() + raw.mapoffset,
                        stride: chunk.stride() as u32,
                    }
                }));
                gl.read_dma_buf(f.width, f.height, f.format, f.modifier, &self.planes, out)
                    .map_err(|e| format!("DMA-buf EGL import failed: {e}"))?;
                // glReadPixels with GL_RGBA: R,G,B,A in memory.
                Ok(Some(PixFmt::Rgbx))
            }
            (DataType::MemPtr | DataType::MemFd | DataType::DmaBuf, _) => {
                let d = &mut datas[0];
                let (offset, size, stride) = {
                    let c = d.chunk();
                    (c.offset() as usize, c.size() as usize, c.stride())
                };
                // An empty memory chunk is a buffer queued without a
                // picture. A DMA-buf chunk may leave its size unset,
                // and then the mapping bounds the frame.
                if size == 0 && kind != DataType::DmaBuf {
                    return Ok(None);
                }
                let stride =
                    usize::try_from(stride).map_err(|_| format!("negative row stride {stride}"))?;
                let Some(buf) = d.data() else {
                    return Err(if kind == DataType::DmaBuf {
                        format!(
                            "compositor offers only DMA-buf buffers (modifier {:#x}); \
                             EGL is unavailable and the buffer is not CPU-mappable",
                            f.modifier
                        )
                    } else {
                        "the compositor's buffer is not CPU-mappable".to_string()
                    });
                };
                let end = match size {
                    0 => buf.len(),
                    size => offset.saturating_add(size).min(buf.len()),
                };
                let src = &buf[offset.min(end)..end];
                copy_frame(src, stride, f.width, f.height, f.format, out).map(Some)
            }
            (other, _) => Err(format!(
                "compositor offers only {other:?} buffers; iris's Wayland path needs \
                 MemPtr, MemFd, or DMA-buf"
            )),
        }
    }
}

pub(crate) fn on_process(stream: &StreamRef, data: &mut StreamData) {
    // The newest queued buffer: older ones go back unread, so a slow
    // read never falls further behind the window.
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    while let Some(newer) = stream.dequeue_buffer() {
        buffer = newer;
    }
    let at = Instant::now();
    // An empty size (a compositor may send one while the window is
    // hidden) has no pixels to record: the last frame stays on screen.
    let Some(format) = data.format.filter(|f| f.width > 0 && f.height > 0) else {
        return;
    };
    // try_borrow_mut: a RefCell panic aborts inside this non-unwinding
    // callback, and dropping a frame on an unexpected nested borrow
    // costs nothing.
    let Ok(mut guard) = data.shared.try_borrow_mut() else {
        return;
    };
    let shared = &mut *guard;
    let Some(rec) = shared.recorder.as_mut() else {
        return;
    };
    // Paused or failed: the buffer goes back unread.
    if rec.paused() || shared.error.is_some() {
        return;
    }
    let mut out = rec.take_buf();
    let result = match data.reader.read(buffer.datas_mut(), format, &mut out) {
        Ok(Some(pix)) => {
            let shape = Shape {
                width: format.width,
                height: format.height,
                pix,
            };
            rec.frame(out, shape, at)
        }
        Ok(None) => {
            rec.give_back(out);
            return;
        }
        Err(e) => {
            rec.give_back(out);
            Err(e)
        }
    };
    match result {
        Ok(()) => {
            data.frames += 1;
            if data.frames == 1 {
                crate::ilog!("iris: record: first frame written");
            }
        }
        Err(e) => shared.end(Some(e)),
    }
}

pub(crate) fn on_state_changed(
    _stream: &StreamRef,
    data: &mut StreamData,
    _old: StreamState,
    new: StreamState,
) {
    crate::ilog!("iris: record: stream {new:?}");
    let end = match new {
        StreamState::Error(e) => Some(format!("PipeWire stream failed: {e}")),
        // The connection to PipeWire is gone: no frame follows.
        StreamState::Unconnected => None,
        StreamState::Connecting | StreamState::Paused | StreamState::Streaming => return,
    };
    if let Ok(mut shared) = data.shared.try_borrow_mut() {
        shared.end(end);
    }
}

/// What steers a recording from outside the stream.
pub(crate) struct Controls {
    pub(crate) stop: Receiver<()>,
    pub(crate) control: Receiver<RecControl>,
    /// Set when the compositor closed the portal session.
    pub(crate) closed: Arc<AtomicBool>,
}

/// Read the controls after a ring: the stop signal, chip controls, and
/// a session the compositor closed.
pub(crate) fn on_ring(shared: &RefCell<Shared>, ctl: &Controls, wake: &Wake) {
    // Undrained while the state is borrowed: the loop's level-triggered
    // watch runs this again on its next pass.
    let Ok(mut guard) = shared.try_borrow_mut() else {
        return;
    };
    // Drained before the channels are read: a send after the read
    // rings again.
    wake.drain();
    let shared = &mut *guard;
    let running = match shared.recorder.as_mut() {
        Some(rec) => apply_controls(rec, ctl),
        None => Ok(false),
    };
    match running {
        Ok(true) => {}
        Ok(false) => shared.end(None),
        Err(e) => shared.end(Some(e)),
    }
}

/// Apply what arrived since the last ring; false once the recording
/// stops.
fn apply_controls(rec: &mut Recorder, ctl: &Controls) -> Result<bool, String> {
    // A stop is a send OR a disconnect: a dropped ActiveRecording must
    // still end the stream, not record forever detached. A closed
    // session delivers no frame again.
    if !matches!(ctl.stop.try_recv(), Err(TryRecvError::Empty))
        || ctl.closed.load(Ordering::Acquire)
    {
        return Ok(false);
    }
    loop {
        match ctl.control.try_recv() {
            Ok(c) => rec.apply(c, Instant::now())?,
            Err(TryRecvError::Empty) => return Ok(true),
            Err(TryRecvError::Disconnected) => return Ok(false),
        }
    }
}

/// Copy one frame of `stride`-byte rows into `out` as packed rows,
/// returning its layout. `out` is reused across frames, so recording
/// allocates no frame-sized buffer per frame. No swizzle: the segment
/// header declares the layout, and ffmpeg converts once.
pub(crate) fn copy_frame(
    src: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    format: VideoFormat,
    out: &mut Vec<u8>,
) -> Result<PixFmt, String> {
    let pix = match format {
        VideoFormat::BGRx | VideoFormat::BGRA => PixFmt::Bgrx,
        VideoFormat::RGBx | VideoFormat::RGBA => PixFmt::Rgbx,
        other => {
            return Err(format!(
                "unsupported negotiated pixel format {other:?}; expected BGRx/BGRA/RGBx/RGBA"
            ))
        }
    };
    let (row, h) = (width as usize * 4, height as usize);
    if row == 0 || h == 0 {
        return Err(format!("empty frame {width}x{height}"));
    }
    let stride = if stride == 0 { row } else { stride };
    let need = stride * (h - 1) + row;
    if stride < row || src.len() < need {
        return Err(format!(
            "short frame buffer: {} bytes for {width}x{height} at stride {stride}",
            src.len()
        ));
    }
    let len = row * h;
    out.clear();
    out.reserve(len);
    // Rows go into the spare capacity, banded across threads once the
    // frame is large enough to pay for the spawn: no zero fill first.
    crate::par::par_bands_mut(&mut out.spare_capacity_mut()[..len], row, |band, start| {
        let first = start / row;
        for (r, dst) in band.chunks_exact_mut(row).enumerate() {
            let at = (first + r) * stride;
            let line = &src[at..at + row];
            // SAFETY: `dst` and `line` are both `row` bytes and do not
            // overlap: one is `out`'s spare capacity, the other `src`.
            unsafe { std::ptr::copy_nonoverlapping(line.as_ptr(), dst.as_mut_ptr().cast(), row) };
        }
    });
    // SAFETY: the bands cover all `len` bytes, and each was written.
    unsafe { out.set_len(len) };
    Ok(pix)
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
    let max = info.max_framerate();
    data.format = Some(Negotiated {
        width: size.width,
        height: size.height,
        format: info.format(),
        modifier: info.modifier(),
    });
    crate::ilog!(
        "iris: record: negotiated format {}x{} {:?} modifier {:#x}, at most {}/{} fps",
        size.width,
        size.height,
        info.format(),
        info.modifier(),
        max.num,
        max.denom
    );
}

/// The stream's format offers: packed 32-bit RGB in CPU memory, the
/// same as DMA-buf, and the buffer types iris reads. Both formats ask
/// for at most `fps` frames a second, so a compositor that honors
/// maxFramerate sends no frame the recording would not keep.
pub(crate) fn build_param_pods(fps: u32) -> Result<Vec<Vec<u8>>, String> {
    let max = Fraction {
        num: fps.max(1),
        denom: 1,
    };
    let max_rate = || {
        pod::property!(
            FormatProperties::VideoMaxFramerate,
            Choice,
            Range,
            Fraction,
            max,
            Fraction { num: 1, denom: 1 },
            max
        )
    };
    let formats = || {
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
        )
    };
    let obj_raw = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        formats(),
        max_rate(),
    );

    let obj_dma = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        formats(),
        max_rate(),
        pod::Property::new(
            FormatProperties::VideoModifier.as_raw(),
            pod::Value::Choice(pod::ChoiceValue::Long(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Enum {
                    default: DRM_FORMAT_MOD_INVALID as i64,
                    alternatives: vec![DRM_FORMAT_MOD_INVALID as i64, DRM_FORMAT_MOD_LINEAR as i64],
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
