//! Wayland window recording: xdg-desktop-portal ScreenCast for source
//! selection (the compositor's own window picker), PipeWire for frame
//! delivery, and the shared ffmpeg encoder for the mp4.
//! Frames arrive as MemPtr, MemFd, or DMA-buf buffers; MAP_BUFFERS asks
//! PipeWire to mmap MemFd for us. DMA-buf buffers are imported via EGL
//! and read back via glReadPixels, or via CPU mmap when permitted.

mod dma_buf;
mod gl;
mod stream;

#[cfg(test)]
mod tests;

pub(crate) use gl::GlContext;
pub(crate) use stream::{build_param_pods, on_param_changed, on_process, Shared, StreamData};

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::rc::Rc;

use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::PersistMode;
use enumflags2::BitFlags;
use pipewire::properties::properties;
use pipewire::spa::pod;
use pipewire::stream::StreamFlags;

use super::{RecordingSpec, CANCELLED_PREFIX};

/// Negotiate the portal side: session, source selection (window picker
/// shown by the compositor), start, PipeWire remote fd.
async fn portal_negotiate() -> Result<(OwnedFd, u32), String> {
    let screencast = Screencast::new().await.map_err(|e| {
        format!("portal ScreenCast unavailable (is xdg-desktop-portal running?): {e}")
    })?;
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
            if matches!(
                e,
                ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)
            ) {
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

/// `portal_negotiate` driven to completion on its own thread: the
/// caller polls the stop channel while the dialog is up.
fn portal_negotiate_blocking() -> Result<(OwnedFd, u32), String> {
    futures::executor::block_on(portal_negotiate())
}

/// Record a portal-selected window until `spec.stop` fires.
pub fn record_window(spec: RecordingSpec) -> Result<PathBuf, String> {
    crate::ilog!(
        "iris: record: record_window start -> {}",
        spec.output.display()
    );
    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(
            "Wayland recording needs WAYLAND_DISPLAY; this is not a Wayland session".to_string(),
        );
    }
    // The portal dialog can sit unanswered for minutes; poll the stop
    // channel while negotiating so a stop during the pick still joins.
    // The negotiate result arrives on its own channel: recv_timeout
    // wakes the instant the portal answers, instead of a 50ms poll.
    let (neg_tx, neg_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = neg_tx.send(portal_negotiate_blocking());
    });
    let (fd, node_id) = loop {
        match neg_rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(r) => break r?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !matches!(
                    spec.stop.try_recv(),
                    Err(std::sync::mpsc::TryRecvError::Empty)
                ) {
                    // The negotiate thread finishes on its own once
                    // the user answers; the recording ends as a
                    // cancel either way.
                    return Err(format!("{CANCELLED_PREFIX} stopped during source pick"));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("portal negotiate thread died".to_string());
            }
        }
    };

    pipewire::init();
    let mainloop =
        pipewire::main_loop::MainLoop::new(None).map_err(|e| format!("PipeWire main loop: {e}"))?;
    let context =
        pipewire::context::Context::new(&mainloop).map_err(|e| format!("PipeWire context: {e}"))?;
    let core = context
        .connect_fd(fd, None)
        .map_err(|e| format!("PipeWire connect (is pipewire running?): {e}"))?;

    let shared = Rc::new(RefCell::new(Shared {
        encoder: None,
        enc_fmt: (0, 0, super::encoder::PixFmt::Rgba),
        output: spec.output.clone(),
        error: None,
        quit: Some(mainloop.downgrade()),
        done: false,
    }));
    let stream = pipewire::stream::Stream::new(
        &core,
        "iris-rec",
        properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CLASS => "Stream/Input/Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| format!("PipeWire stream: {e}"))?;

    let gl_context = match GlContext::new() {
        Ok(ctx) => {
            crate::ilog!("iris: record: EGL context initialized for DMA-buf");
            Some(ctx)
        }
        Err(e) => {
            crate::ilog!("iris: record: EGL context unavailable: {e}");
            None
        }
    };

    let data = StreamData {
        fps: spec.fps,
        mic: spec.mic,
        rec_format: spec.format,
        rec_encoder: spec.encoder,
        format: None,
        shared: shared.clone(),
        unsupported_reported: false,
        buffer_logged: false,
        frames: 0,
        gl_context,
        scratch: Vec::new(),
        planes: Vec::new(),
    };

    let stop = spec.stop;
    let weak = mainloop.downgrade();
    let _stop_timer = mainloop.loop_().add_timer(move |_| {
        // A send OR a disconnect means stop: a dropped ActiveRecording
        // must still end the stream, not record forever detached.
        if !matches!(stop.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)) {
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
            crate::ilog!("iris: record: stream {new:?}");
        })
        .param_changed(on_param_changed)
        .process(on_process)
        .register()
        .map_err(|e| format!("PipeWire listener: {e}"))?;

    let params = build_param_pods()?;
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

    mainloop.run();

    // Stop the stream before touching shared: on_process runs on
    // PipeWire's data thread, and a borrow here racing its borrow_mut
    // aborts the process (RefCell panic in a non-unwinding callback).
    stream.disconnect().ok();
    let mut shared = shared.borrow_mut();

    shared.done = true;
    if let Some(error) = shared.error.take() {
        return Err(error);
    }
    let encoder = shared
        .encoder
        .take()
        .ok_or_else(|| "stream ended before any frame was captured".to_string())?;
    let output = shared.output.clone();
    drop(shared);
    encoder.finish()?;
    Ok(output)
}
