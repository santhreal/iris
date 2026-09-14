//! Wayland window recording: xdg-desktop-portal ScreenCast for source
//! selection (the compositor's own picker — per-window by user choice,
//! nothing else leaves the compositor), PipeWire for frame delivery, and
//! the shared ffmpeg encoder for the mp4.
//! Frames arrive as MemPtr or MemFd buffers; MAP_BUFFERS asks PipeWire
//! to mmap MemFd for us. A compositor that offers only DMA-buf is
//! reported as unsupported rather than silently producing nothing.

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::rc::Rc;

use ashpd::desktop::screencast::{
    CursorMode, SelectSourcesOptions, Screencast, SourceType,
};
use ashpd::desktop::PersistMode;
use enumflags2::BitFlags;
use pipewire::properties::properties;
use pipewire::spa::buffer::DataType;
use pipewire::spa::param::video::{VideoFormat, VideoInfoRaw};
use pipewire::spa::param::ParamType;
use pipewire::spa::pod;
use pipewire::stream::{StreamFlags, StreamRef};

use super::encoder::{Encoder, EncoderConfig};
use super::{RecordingSpec, CANCELLED_PREFIX};

/// Negotiate the portal side: session, source selection (window picker
/// shown by the compositor), start, PipeWire remote fd.
async fn portal_negotiate() -> Result<(OwnedFd, u32), String> {
    let screencast = Screencast::new()
        .await
        .map_err(|e| format!("portal ScreenCast unavailable (is xdg-desktop-portal running?): {e}"))?;
    let session = screencast
        .create_session(Default::default())
        .await
        .map_err(|e| format!("portal session create failed: {e}"))?;

    let options = SelectSourcesOptions::default()
        .set_sources(BitFlags::from(SourceType::Window))
        .set_cursor_mode(CursorMode::Hidden)
        .set_multiple(false)
        .set_persist_mode(PersistMode::DoNot);
    screencast
        .select_sources(&session, options)
        .await
        .map_err(|e| format!("portal select_sources failed: {e}"))?;

    let streams = screencast
        .start(&session, None, Default::default())
        .await
        .map_err(|e| format!("portal start failed: {e}"))?
        .response()
        .map_err(|e| {
            if matches!(e, ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) {
                format!("{CANCELLED_PREFIX} portal dialog dismissed")
            } else {
                format!("portal start denied or failed: {e}")
            }
        })?;

    let stream = streams
        .streams()
        .first()
        .ok_or_else(|| "portal returned no streams".to_string())?;
    let node_id = stream.pipe_wire_node_id();

    let fd = screencast
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|e| format!("portal PipeWire remote failed: {e}"))?;
    Ok((fd, node_id))
}

struct Shared {
    encoder: Option<Encoder>,
    error: Option<String>,
    quit: Option<pipewire::main_loop::WeakMainLoop>,
    /// Set once the encoder is taken for finish(): on_process can fire
    /// once more as the loop drains, and must not respawn ffmpeg.
    done: bool,
}

struct StreamData {
    output: PathBuf,
    fps: u32,
    mic: bool,
    format: Option<(u32, u32, VideoFormat)>,
    shared: Rc<RefCell<Shared>>,
    unsupported_reported: bool,
    frames: u64,
    /// Reused RGBA scratch for convert_frame: recording would otherwise
    /// allocate a multi-MB buffer per frame.
    scratch: Vec<u8>,
}

impl StreamData {
    fn quit(&self) {
        // pw_main_loop_quit can re-enter on_process on some PipeWire
        // versions; the borrow must be released before it runs or the
        // nested borrow_mut panics.
        let mainloop = self.shared.borrow().quit.as_ref().and_then(|w| w.upgrade());
        if let Some(mainloop) = mainloop {
            mainloop.quit();
        }
    }

    fn fail(&mut self, error: String) {
        self.shared.borrow_mut().error = Some(error);
        self.quit();
    }
}
fn on_process(stream: &StreamRef, data: &mut StreamData) {
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let datas = buffer.datas_mut();
    let Some(first) = datas.first_mut() else {
        return;
    };
    match first.type_() {
        DataType::MemPtr | DataType::MemFd => {}
        other => {
            if !data.unsupported_reported {
                data.unsupported_reported = true;
                data.fail(format!(
                    "compositor offers only {other:?} buffers; iris's Wayland path needs CPU-mappable memory"
                ));
            }
            return;
        }
    }

    let Some((width, height, format)) = data.format else {
        return; // Format not negotiated yet; drop the frame.
    };
    let (offset, stride, size) = {
        let chunk = first.chunk();
        (chunk.offset() as usize, chunk.stride() as usize, chunk.size() as usize)
    };
    let Some(buf) = first.data() else {
        return;
    };
    let end = offset.saturating_add(size).min(buf.len());
    let src = &buf[offset.min(end)..end];

    if let Err(e) = convert_frame(src, stride, width, height, format, &mut data.scratch) {
        data.fail(e);
        return;
    }

    let mut shared = data.shared.borrow_mut();
    if shared.done {
        return; // encoder already taken for finish(); drop the frame
    }
    if shared.encoder.is_none() {
        match Encoder::start(&EncoderConfig {
            output: data.output.clone(),
            width,
            height,
            fps: data.fps,
            mic: data.mic,
        }) {
            Ok(encoder) => shared.encoder = Some(encoder),
            Err(e) => {
                drop(shared);
                data.fail(e);
                return;
            }
        }
    }
    let write_result = shared.encoder.as_mut().unwrap().write_frame(&data.scratch);
    drop(shared);
    data.frames += 1;
    if data.frames == 1 {
        // First encoded frame is the observable "recording is live"
        // edge; the QA rig waits on this line before stopping.
        eprintln!("iris: record: first frame written");
    }
    if let Err(e) = write_result {
        eprintln!("iris: record: write_frame failed: {e}");
        data.fail(e);
    }
}

/// Swizzle one PipeWire frame into `out` (resized to w*h*4), forcing
/// alpha to opaque. `out` is reused across frames so recording does not
/// allocate a multi-MB buffer per frame.
fn convert_frame(
    src: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    format: VideoFormat,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let (w, h) = (width as usize, height as usize);
    let stride = if stride == 0 { w * 4 } else { stride };
    if src.len() < stride * h {
        return Err(format!(
            "short frame buffer: {} bytes for {w}x{h} at stride {stride}",
            src.len()
        ));
    }
    out.clear();
    out.resize(w * h * 4, 0);
    // Word-level swizzle, same as the X11 grab: a per-pixel
    // extend_from_slice is a visible slice of per-frame latency at
    // 1280x800 and up. BGRx swaps bytes 0<->2; RGBx only forces alpha.
    match format {
        VideoFormat::BGRx | VideoFormat::BGRA => {
            for (row_out, row_in) in out
                .chunks_exact_mut(w * 4)
                .zip(src.chunks(stride).take(h))
            {
                for (o, px) in row_out
                    .chunks_exact_mut(4)
                    .zip(row_in[..w * 4].chunks_exact(4))
                {
                    let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                    let rgb = (v & 0xFF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF);
                    o.copy_from_slice(&(rgb | 0xFF00_0000).to_le_bytes());
                }
            }
        }
        VideoFormat::RGBx | VideoFormat::RGBA => {
            for (row_out, row_in) in out
                .chunks_exact_mut(w * 4)
                .zip(src.chunks(stride).take(h))
            {
                for (o, px) in row_out
                    .chunks_exact_mut(4)
                    .zip(row_in[..w * 4].chunks_exact(4))
                {
                    let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                    o.copy_from_slice(&((v & 0xFF_FFFF) | 0xFF00_0000).to_le_bytes());
                }
            }
        }
        other => {
            return Err(format!(
                "unsupported negotiated pixel format {other:?}; expected BGRx/BGRA/RGBx/RGBA"
            ));
        }
    }
    Ok(())
}

fn on_param_changed(
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
    data.format = Some((size.width, size.height, info.format()));
    eprintln!("iris: record: negotiated format {}x{} {:?}", size.width, size.height, info.format());
}

/// Record a portal-selected window until `spec.stop` fires.
pub fn record_window(spec: RecordingSpec) -> Result<(), String> {
    eprintln!("iris: record: record_window start -> {}", spec.output.display());
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(
            "Wayland recording needs WAYLAND_DISPLAY; this is not a Wayland session".to_string(),
        );
    }
    let (fd, node_id) = futures::executor::block_on(portal_negotiate())?;

    pipewire::init();
    let mainloop = pipewire::main_loop::MainLoop::new(None)
        .map_err(|e| format!("PipeWire main loop: {e}"))?;
    let context = pipewire::context::Context::new(&mainloop)
        .map_err(|e| format!("PipeWire context: {e}"))?;
    let core = context
        .connect_fd(fd, None)
        .map_err(|e| format!("PipeWire connect (is pipewire running?): {e}"))?;

    let shared = Rc::new(RefCell::new(Shared {
        encoder: None,
        error: None,
        quit: Some(mainloop.downgrade()),
        done: false,
    }));

    let stream = pipewire::stream::Stream::new(
        &core,
        "iris-rec",
        properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            // media.class marks this as a capture stream so the session
            // manager classifies it correctly.
            *pipewire::keys::MEDIA_CLASS => "Stream/Input/Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| format!("PipeWire stream: {e}"))?;

    let data = StreamData {
        output: spec.output.clone(),
        fps: spec.fps,
        mic: spec.mic,
        format: None,
        shared: shared.clone(),
        unsupported_reported: false,
        frames: 0,
        scratch: Vec::new(),
    };

    // The stop signal cannot live in on_process: once the stream pauses
    // no more process callbacks fire, so the loop would run forever.
    // A repeating timer polls it on the main-loop thread instead.
    let stop = spec.stop;
    let weak = mainloop.downgrade();
    let _stop_timer = mainloop.loop_().add_timer(move |_| {
        if stop.try_recv().is_ok() {
            if let Some(mainloop) = weak.upgrade() {
                mainloop.quit();
            }
        }
    });
    _stop_timer
        .update_timer(
            Some(std::time::Duration::from_millis(50)),
            Some(std::time::Duration::from_millis(50)),
        )
        .into_result()
        .map_err(|e| format!("PipeWire stop timer: {e}"))?;

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(|_stream, _data, _old, new| {
            // Streaming is the observable "frames are flowing" edge;
            // the QA rig waits on this line before stopping.
            eprintln!("iris: record: stream {new:?}");
        })
        .param_changed(on_param_changed)
        .process(on_process)
        .register()
        .map_err(|e| format!("PipeWire listener: {e}"))?;

    // Offer raw BGRx (preferred) plus the other 32-bit layouts. The
    // Buffers param advertises MemPtr and MemFd; MAP_BUFFERS makes
    // PipeWire mmap MemFd buffers so on_process can read either kind.
    let obj = pod::object!(
        pipewire::spa::utils::SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(
            pipewire::spa::param::format::FormatProperties::MediaType,
            Id,
            pipewire::spa::param::format::MediaType::Video
        ),
        pod::property!(
            pipewire::spa::param::format::FormatProperties::MediaSubtype,
            Id,
            pipewire::spa::param::format::MediaSubtype::Raw
        ),
        pod::property!(
            pipewire::spa::param::format::FormatProperties::VideoFormat,
            Choice, Enum, Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
    );
    let buffers_obj = pod::object!(
        pipewire::spa::utils::SpaTypes::ObjectParamBuffers,
        ParamType::Buffers,
        pod::Property::new(
            pipewire::spa::sys::SPA_PARAM_BUFFERS_dataType,
            pod::Value::Int(
                (1 << DataType::MemPtr.as_raw()) | (1 << DataType::MemFd.as_raw())
            ),
        ),
    );
    let mut params = Vec::new();
    for obj in [obj, buffers_obj] {
        let values: Vec<u8> = pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &pod::Value::Object(obj),
        )
        .map_err(|e| format!("serialize format params: {e}"))?
        .0
        .into_inner();
        params.push(values);
    }
    let mut pods: Vec<&pod::Pod> = params
        .iter()
        .map(|v| pod::Pod::from_bytes(v).ok_or("invalid format param pod"))
        .collect::<Result<_, _>>()?;

    stream
        .connect(
            pipewire::spa::utils::Direction::Input,
            Some(node_id),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut pods,
        )
        .map_err(|e| format!("PipeWire stream connect to node {node_id}: {e}"))?;
    // Drive the loop until the stop timer quits it; on_process encodes
    // each frame as it arrives.
    mainloop.run();

    let mut shared = shared.borrow_mut();
    shared.done = true;
    if let Some(error) = shared.error.take() {
        return Err(error);
    }
    let encoder = shared
        .encoder
        .take()
        .ok_or_else(|| "stream ended before any frame was captured".to_string())?;
    encoder.finish()?;
    Ok(())
}
