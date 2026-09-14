//! Wayland window recording: xdg-desktop-portal ScreenCast for source
//! selection (the compositor's own picker — per-window by user choice,
//! nothing else leaves the compositor), PipeWire for frame delivery, and
//! the shared ffmpeg encoder for the mp4.
//!
//! Frames arrive as MemPtr buffers; a compositor that offers only DMA-buf
//! is reported as unsupported rather than silently producing nothing.

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;

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
}

struct StreamData {
    stop: Receiver<()>,
    output: PathBuf,
    fps: u32,
    mic: bool,
    format: Option<(u32, u32, VideoFormat)>,
    shared: Rc<RefCell<Shared>>,
    unsupported_reported: bool,
}

impl StreamData {
    fn quit(&self) {
        if let Some(weak) = &self.shared.borrow().quit {
            if let Some(mainloop) = weak.upgrade() {
                mainloop.quit();
            }
        }
    }

    fn fail(&mut self, error: String) {
        self.shared.borrow_mut().error = Some(error);
        self.quit();
    }
}

fn on_process(stream: &StreamRef, data: &mut StreamData) {
    if data.stop.try_recv().is_ok() {
        data.quit();
        return;
    }

    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let datas = buffer.datas_mut();
    let Some(first) = datas.first_mut() else {
        return;
    };
    if first.type_() != DataType::MemPtr {
        if !data.unsupported_reported {
            data.unsupported_reported = true;
            data.fail(format!(
                "compositor offers only {:?} buffers; iris's Wayland path needs MemPtr",
                first.type_()
            ));
        }
        return;
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

    let rgba = match convert_frame(src, stride, width, height, format) {
        Ok(rgba) => rgba,
        Err(e) => {
            data.fail(e);
            return;
        }
    };

    let mut shared = data.shared.borrow_mut();
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
    let write_result = shared.encoder.as_mut().unwrap().write_frame(&rgba);
    drop(shared);
    if let Err(e) = write_result {
        data.fail(e);
    }
}

fn convert_frame(
    src: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    format: VideoFormat,
) -> Result<Vec<u8>, String> {
    let (w, h) = (width as usize, height as usize);
    let stride = if stride == 0 { w * 4 } else { stride };
    if src.len() < stride * h {
        return Err(format!(
            "short frame buffer: {} bytes for {w}x{h} at stride {stride}",
            src.len()
        ));
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    match format {
        VideoFormat::BGRx | VideoFormat::BGRA => {
            for row in src.chunks(stride).take(h) {
                for px in row[..w * 4].chunks_exact(4) {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
        }
        VideoFormat::RGBx | VideoFormat::RGBA => {
            for row in src.chunks(stride).take(h) {
                for px in row[..w * 4].chunks_exact(4) {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
        other => {
            return Err(format!(
                "unsupported negotiated pixel format {other:?}; expected BGRx/BGRA/RGBx/RGBA"
            ));
        }
    }
    Ok(rgba)
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
}

/// Record a portal-selected window until `spec.stop` fires.
pub fn record_window(spec: RecordingSpec) -> Result<(), String> {
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
    }));

    let stream = pipewire::stream::Stream::new(
        &core,
        "iris-rec",
        properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| format!("PipeWire stream: {e}"))?;

    let data = StreamData {
        stop: spec.stop,
        output: spec.output.clone(),
        fps: spec.fps,
        mic: spec.mic,
        format: None,
        shared: shared.clone(),
        unsupported_reported: false,
    };

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .param_changed(on_param_changed)
        .process(on_process)
        .register()
        .map_err(|e| format!("PipeWire listener: {e}"))?;

    // Offer raw BGRx (preferred) plus the other 32-bit layouts, memory
    // pointers only (no DMA-buf advertisement).
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
    let values: Vec<u8> = pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pod::Value::Object(obj),
    )
    .map_err(|e| format!("serialize format params: {e}"))?
    .0
    .into_inner();
    let mut params = [pod::Pod::from_bytes(&values).ok_or("invalid format param pod")?];

    stream
        .connect(
            pipewire::spa::utils::Direction::Input,
            Some(node_id),
            StreamFlags::AUTOCONNECT,
            &mut params,
        )
        .map_err(|e| format!("PipeWire stream connect to node {node_id}: {e}"))?;

    mainloop.run();

    let mut shared = shared.borrow_mut();
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
