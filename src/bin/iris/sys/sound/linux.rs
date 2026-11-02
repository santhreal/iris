//! Linux: the synthesized shutter (`click`) through a PipeWire playback
//! stream on the notification role, or through `paplay` or `aplay`
//! where PipeWire plays no audio.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

use pipewire::main_loop::{MainLoopRc, MainLoopWeak};
use pipewire::properties::properties;
use pipewire::spa::param::audio::{AudioFormat, AudioInfoRaw};
use pipewire::spa::pod::{self, Pod};
use pipewire::stream::{StreamFlags, StreamState};
use pipewire::types::ObjectType;

use super::click;

/// The longest a shutter thread lives: the 110 ms sound, after a sink
/// that resumes from suspend first.
const DEADLINE: Duration = Duration::from_secs(3);

/// Play the shutter on its own thread and return at once.
pub fn shutter() {
    let _ = std::thread::Builder::new()
        .name("iris-shutter".into())
        .spawn(|| match play() {
            Ok(()) => {}
            Err(Fallback(why)) => {
                iris_lib::ilog!("iris: shutter: {why}; playing through a sound player");
                players();
            }
        });
}

/// PipeWire could not play the sound, and why.
struct Fallback(String);

fn quit(main: &MainLoopWeak) {
    if let Some(main) = main.upgrade() {
        main.quit();
    }
}

/// Play the PCM through PipeWire. A missing daemon or a graph with no
/// audio sink (PipeWire running for screen sharing beside PulseAudio)
/// is a `Fallback`; anything after the stream starts is not, since a
/// second player would sound the shutter twice.
fn play() -> Result<(), Fallback> {
    let fail = |what: &str, e: pipewire::Error| Fallback(format!("PipeWire {what}: {e}"));
    pipewire::init();
    let main = MainLoopRc::new(None).map_err(|e| fail("main loop", e))?;
    let context = pipewire::context::ContextRc::new(&main, None).map_err(|e| fail("context", e))?;
    let core = context.connect_rc(None).map_err(|e| fail("connect", e))?;

    let expired = Rc::new(Cell::new(false));
    let timer = main.loop_().add_timer({
        let (main, expired) = (main.downgrade(), expired.clone());
        move |_| {
            expired.set(true);
            quit(&main);
        }
    });
    timer.update_timer(Some(DEADLINE), None);
    let error = Rc::new(RefCell::new(None));
    let _errors = core
        .add_listener_local()
        .error({
            let (main, error) = (main.downgrade(), error.clone());
            move |_, _, _, message| {
                error.replace(Some(message.to_string()));
                quit(&main);
            }
        })
        .register();

    if !has_sink(&main, &core).map_err(|e| fail("registry", e))? {
        return Err(Fallback(match error.take() {
            Some(e) => format!("PipeWire: {e}"),
            None if expired.get() => "PipeWire did not answer".into(),
            None => "PipeWire has no audio sink".into(),
        }));
    }

    let stream = pipewire::stream::StreamRc::new(
        core,
        "iris-shutter",
        properties! {
            *pipewire::keys::MEDIA_TYPE => "Audio",
            *pipewire::keys::MEDIA_CATEGORY => "Playback",
            *pipewire::keys::MEDIA_ROLE => "Notification",
            *pipewire::keys::MEDIA_NAME => "Capture shutter",
        },
    )
    .map_err(|e| fail("stream", e))?;
    let started = Rc::new(Cell::new(false));
    let _listener = stream
        .add_local_listener_with_user_data(0usize)
        .state_changed({
            let main = main.downgrade();
            move |_, _, _, state| {
                if matches!(state, StreamState::Error(_) | StreamState::Unconnected) {
                    quit(&main);
                }
            }
        })
        .process({
            let started = started.clone();
            move |stream, at| {
                started.set(true);
                fill(stream, at);
            }
        })
        .drained({
            let main = main.downgrade();
            move |_, _| quit(&main)
        })
        .register()
        .map_err(|e| fail("listener", e))?;

    let format = format_pod().map_err(Fallback)?;
    let mut params = [
        Pod::from_bytes(&format).ok_or_else(|| Fallback("invalid shutter format pod".into()))?
    ];
    stream
        .connect(
            pipewire::spa::utils::Direction::Output,
            None,
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| fail("stream connect", e))?;
    main.run();
    if let Some(e) = error.take() {
        iris_lib::ilog!("iris: shutter: PipeWire: {e}");
    } else if !started.get() {
        iris_lib::ilog!("iris: shutter: the PipeWire stream did not start");
    }
    let _ = stream.disconnect();
    Ok(())
}

/// Whether the graph holds an audio sink, by one registry round trip.
fn has_sink(main: &MainLoopRc, core: &pipewire::core::CoreRc) -> Result<bool, pipewire::Error> {
    let registry = core.get_registry_rc()?;
    let sink = Rc::new(Cell::new(false));
    let _globals = registry
        .add_listener_local()
        .global({
            let sink = sink.clone();
            move |global| {
                let class = global
                    .props
                    .and_then(|p| p.get(*pipewire::keys::MEDIA_CLASS));
                if global.type_ == ObjectType::Node && class == Some("Audio/Sink") {
                    sink.set(true);
                }
            }
        })
        .register();
    let pending = core.sync(0)?;
    let _done = core
        .add_listener_local()
        .done({
            let main = main.downgrade();
            move |id, seq| {
                if id == pipewire::core::PW_ID_CORE && seq == pending {
                    quit(&main);
                }
            }
        })
        .register();
    main.run();
    Ok(sink.get())
}

/// Queue the next samples, as many as the buffer holds, and drain the
/// stream after the last. PipeWire's converter feeds them to the graph
/// one quantum at a time.
fn fill(stream: &pipewire::stream::Stream, at: &mut usize) {
    let pcm = click::pcm();
    if *at >= pcm.len() {
        return;
    }
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    let Some(data) = buffer.datas_mut().first_mut() else {
        return;
    };
    let Some(out) = data.data() else {
        return;
    };
    let n = (pcm.len() - *at).min(out.len() / 2);
    let (frames, _) = out.as_chunks_mut::<2>();
    for (o, s) in frames.iter_mut().zip(&pcm[*at..*at + n]) {
        *o = s.to_le_bytes();
    }
    *at += n;
    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = 2;
    *chunk.size_mut() = (n * 2) as u32;
    drop(buffer);
    if *at >= pcm.len() {
        let _ = stream.flush(true);
    }
}

/// The EnumFormat pod: 16-bit mono at the click's rate. PipeWire mixes
/// it up to the sink's channels and rate.
fn format_pod() -> Result<Vec<u8>, String> {
    let mut info = AudioInfoRaw::new();
    info.set_format(AudioFormat::S16LE);
    info.set_rate(click::RATE);
    info.set_channels(1);
    let mut position = [0; pipewire::spa::sys::SPA_AUDIO_MAX_CHANNELS as usize];
    position[0] = pipewire::spa::sys::SPA_AUDIO_CHANNEL_MONO;
    info.set_position(position);
    let format = pod::Object {
        type_: pipewire::spa::sys::SPA_TYPE_OBJECT_Format,
        id: pipewire::spa::sys::SPA_PARAM_EnumFormat,
        properties: info.into(),
    };
    Ok(pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pod::Value::Object(format),
    )
    .map_err(|e| format!("serialize the shutter format: {e}"))?
    .0
    .into_inner())
}

/// The WAV through the first sound player that plays it, on stdin. Each
/// player is waited on, so none is left a zombie, and one that exits
/// nonzero (no sound server) falls through to the next.
fn players() {
    let wav = click::wav();
    let players: [(&str, &[&str]); 2] = [("paplay", &[]), ("aplay", &["-q"])];
    for (bin, args) in players {
        let Ok(mut child) = Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&wav);
        }
        if child.wait().is_ok_and(|s| s.success()) {
            return;
        }
    }
}
