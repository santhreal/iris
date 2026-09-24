//! Wayland window recording: the ScreenCast portal selects the window
//! through the compositor's own picker, PipeWire delivers its frames,
//! and a `Recorder` encodes them.
//!
//! Frames arrive as MemPtr, MemFd, or DMA-buf buffers; MAP_BUFFERS has
//! PipeWire map MemFd. A DMA-buf is imported through EGL and read back
//! with glReadPixels, or read through its CPU mapping without EGL.

mod dma_buf;
mod gl;
mod portal;
mod stream;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use pipewire::properties::properties;
use pipewire::spa::pod;
use pipewire::spa::support::system::IoFlags;
use pipewire::stream::StreamFlags;

use gl::GlContext;
use portal::Portal;
use stream::{Controls, Reader, Shared, StreamData};

use super::recorder::Recorder;
use super::wake::Wake;
use super::RecordingSpec;

/// Record a portal-selected window until `spec.stop` fires.
pub fn record_window(spec: RecordingSpec) -> Result<PathBuf, String> {
    crate::ilog!(
        "iris: record: record_window start -> {}",
        spec.output.display()
    );
    if !crate::session::wayland() {
        return Err(
            "Wayland recording needs WAYLAND_DISPLAY; this is not a Wayland session".to_string(),
        );
    }
    // Installed first: the pick and the stream both sleep until a
    // ring, and read the channels the moment one arrives.
    let wake = Wake::install(&spec.bell)?;
    let (portal, fd) = Portal::open(&spec.stop, &spec.bell, &wake)?;

    pipewire::init();
    let mainloop =
        pipewire::main_loop::MainLoop::new(None).map_err(|e| format!("PipeWire main loop: {e}"))?;
    let context =
        pipewire::context::Context::new(&mainloop).map_err(|e| format!("PipeWire context: {e}"))?;
    let core = context
        .connect_fd(fd, None)
        .map_err(|e| format!("PipeWire connect (is pipewire running?): {e}"))?;

    let shared = Rc::new(RefCell::new(Shared {
        recorder: Some(Recorder::new(&spec)),
        error: None,
        quit: mainloop.downgrade(),
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

    let gl = match GlContext::new() {
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
        format: None,
        shared: shared.clone(),
        reader: Reader::new(gl),
        frames: 0,
    };

    let fps = spec.fps;
    let controls = Controls {
        stop: spec.stop,
        control: spec.control,
        closed: portal.closed.clone(),
    };
    // A stop, a chip control, and a close from the compositor each ring
    // the wake; a still window costs the loop no wakeup.
    let ring_shared = shared.clone();
    let _controls = mainloop
        .loop_()
        .add_io(wake, IoFlags::IN, move |wake: &mut Wake| {
            stream::on_ring(&ring_shared, &controls, wake);
        });
    // The pick drained the rings that arrived during it: the first
    // pass reads what they left in the channels.
    spec.bell.ring();

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(stream::on_state_changed)
        .param_changed(stream::on_param_changed)
        .process(stream::on_process)
        .register()
        .map_err(|e| format!("PipeWire listener: {e}"))?;

    let params = stream::build_param_pods(fps)?;
    let mut pods: Vec<&pod::Pod> = params
        .iter()
        .map(|v| pod::Pod::from_bytes(v).ok_or("invalid format param pod"))
        .collect::<Result<_, _>>()?;
    stream
        .connect(
            pipewire::spa::utils::Direction::Input,
            Some(portal.node),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut pods,
        )
        .map_err(|e| format!("PipeWire stream connect to node {}: {e}", portal.node))?;

    mainloop.run();

    // Disconnect before borrowing: the disconnect runs the stream's
    // callbacks, and each of them borrows `shared`.
    stream.disconnect().ok();
    let at = Instant::now();
    // Closes the portal session: the compositor ends the screencast.
    drop(portal);
    let mut shared = shared.borrow_mut();
    let rec = shared
        .recorder
        .take()
        .ok_or("the recording was already finished")?;
    match shared.error.take() {
        Some(e) => Err(rec.fail(at, e)),
        None => rec.finish(at),
    }
}
