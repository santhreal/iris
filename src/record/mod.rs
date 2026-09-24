//! Screen recording: one live source per platform, encoded as a run of
//! Matroska segments and joined into the output file on stop.
//!
//! `codec` and `join` are shared by every platform. On Linux the source
//! grabs frames itself and hands them to a `recorder::Recorder`, which
//! streams them through `encoder` segments framed by `mkv`. On Windows
//! and macOS, `desktop` runs ffmpeg's own screen capture per segment.

mod child;
pub mod codec;
mod doorbell;
pub mod join;

pub use doorbell::Doorbell;

#[cfg(target_os = "linux")]
pub mod encoder;
#[cfg(target_os = "linux")]
pub mod mkv;
#[cfg(target_os = "linux")]
pub mod recorder;
#[cfg(target_os = "linux")]
mod wake;

#[cfg(target_os = "linux")]
pub mod x11;

#[cfg(target_os = "linux")]
pub mod wayland;

#[cfg(any(windows, target_os = "macos"))]
pub mod desktop;

#[cfg(any(windows, target_os = "macos", test))]
mod devices;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::config::{RecordingEncoder, RecordingFormat};

/// Controls for a live recording, from the chip or the CLI. Pause ends
/// the open segment and resume starts the next; a mic toggle ends the
/// segment and starts the next with or without the mic track.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecControl {
    Pause,
    Resume,
    ToggleMic,
}

/// What a platform recording source needs: where to encode, at what rate,
/// and the signal to stop. The source blocks until `stop` fires or it fails,
/// having written the finished file to `spec.output`.
pub struct RecordingSpec {
    pub output: PathBuf,
    /// Frames a second, within the format's capture bounds.
    pub fps: u32,
    /// Whether the mic track starts on. Never set for a format without
    /// audio.
    pub mic: bool,
    pub format: RecordingFormat,
    pub encoder: RecordingEncoder,
    pub stop: Receiver<()>,
    pub control: Receiver<RecControl>,
    /// Rung after every send on `stop` and `control`. A source that
    /// blocks without a timeout installs a ring that wakes it.
    pub bell: Doorbell,
}

/// User aborted before any frame was encoded (e.g. Escape during window
/// pick). Sources signal this with an error message starting with this
/// prefix; the session then tears down quietly without reporting failure.
pub const CANCELLED_PREFIX: &str = "cancelled:";

/// A live recording: the source thread, and the pause and mic state the
/// chip and the CLI toggle.
pub struct ActiveRecording {
    pub output: PathBuf,
    format: RecordingFormat,
    mic: bool,
    paused: bool,
    ended: Arc<AtomicBool>,
    stop: Option<Sender<()>>,
    control: Option<Sender<RecControl>>,
    bell: Doorbell,
    join: Option<JoinHandle<Result<PathBuf, String>>>,
}

/// Marks the source ended and runs the end hook, on return or panic.
struct EndGuard<E: FnOnce()> {
    ended: Arc<AtomicBool>,
    on_end: Option<E>,
}

impl<E: FnOnce()> Drop for EndGuard<E> {
    fn drop(&mut self) {
        self.ended.store(true, Ordering::Release);
        if let Some(on_end) = self.on_end.take() {
            on_end();
        }
    }
}

impl ActiveRecording {
    /// Run `source` on its own thread. `fps` is clamped to the format's
    /// capture bounds, and `mic` holds only for a format with audio.
    /// `on_end` runs on that thread once the source returned or
    /// panicked, stopped or not; `ended` is already true then.
    pub fn spawn<F, E>(
        output: PathBuf,
        fps: u32,
        mic: bool,
        format: RecordingFormat,
        encoder: RecordingEncoder,
        source: F,
        on_end: E,
    ) -> Result<Self, String>
    where
        F: FnOnce(RecordingSpec) -> Result<PathBuf, String> + Send + 'static,
        E: FnOnce() + Send + 'static,
    {
        let mic = mic && format.audio_codec().is_some();
        let (tx, rx) = channel();
        let (ctx, crx) = channel();
        let bell = Doorbell::default();
        let spec = RecordingSpec {
            output: output.clone(),
            fps: format.capture_fps(fps),
            mic,
            format,
            encoder,
            stop: rx,
            control: crx,
            bell: bell.clone(),
        };
        let ended = Arc::new(AtomicBool::new(false));
        let guard = EndGuard {
            ended: ended.clone(),
            on_end: Some(on_end),
        };
        let join = std::thread::Builder::new()
            .name("iris-record".into())
            .spawn(move || {
                let _guard = guard;
                source(spec)
            })
            .map_err(|e| format!("start the recording thread: {e}"))?;
        Ok(Self {
            output,
            format,
            mic,
            paused: false,
            ended,
            stop: Some(tx),
            control: Some(ctx),
            bell,
            join: Some(join),
        })
    }

    /// Whether the source returned: stopped, failed, lost its target, or
    /// its pick was cancelled.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    /// The mic track's state, or `None` for a format without audio.
    pub fn mic(&self) -> Option<bool> {
        self.format.audio_codec().map(|_| self.mic)
    }

    /// Whether the recording is paused.
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Pause, or resume when paused. Returns whether it is paused now.
    pub fn toggle_pause(&mut self) -> bool {
        self.paused = !self.paused;
        self.send(if self.paused {
            RecControl::Pause
        } else {
            RecControl::Resume
        });
        self.paused
    }

    /// Turn the mic track on or off. Returns whether it is on now, or an
    /// error for a format without audio.
    pub fn toggle_mic(&mut self) -> Result<bool, String> {
        if self.format.audio_codec().is_none() {
            return Err(format!(
                "a {} recording has no audio track",
                self.format.ext()
            ));
        }
        self.mic = !self.mic;
        self.send(RecControl::ToggleMic);
        Ok(self.mic)
    }

    /// Forward a control to the source thread. No-op once the recording
    /// has been stopped or the source exited.
    fn send(&self, ctl: RecControl) {
        if let Some(tx) = &self.control {
            let _ = tx.send(ctl);
            self.bell.ring();
        }
    }

    /// Send the stop, once.
    fn signal_stop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
            self.bell.ring();
        }
    }

    /// Signal stop and wait for the source to flush the file. A `cancelled:`
    /// result is reported as Ok(None); anything else is a real error.
    pub fn stop(mut self) -> Result<Option<PathBuf>, String> {
        self.signal_stop();
        let result = self
            .join
            .take()
            .expect("stop called twice")
            .join()
            .map_err(|_| "recording thread panicked".to_string())?;
        Self::finish_result(result, &self.output)
    }

    /// Map a source's join result to the public stop result: a
    /// `cancelled:` error becomes Ok(None) with the stub removed, and a
    /// real error keeps a non-empty file (the segments joined before
    /// the failure).
    fn finish_result(
        result: Result<PathBuf, String>,
        output: &Path,
    ) -> Result<Option<PathBuf>, String> {
        match result {
            Ok(path) => Ok(Some(path)),
            Err(e) if e.starts_with(CANCELLED_PREFIX) => {
                let _ = std::fs::remove_file(output);
                Ok(None)
            }
            Err(e) => {
                let empty = std::fs::metadata(output)
                    .map(|m| m.len() == 0)
                    .unwrap_or(true);
                if empty {
                    let _ = std::fs::remove_file(output);
                }
                Err(e)
            }
        }
    }

    /// Signal stop and join on a background thread: the encoder's
    /// trailer flush can take seconds on a long recording, and joining
    /// on the UI thread freezes hotkeys and socket commands for the
    /// whole flush. `done` runs on the joining thread with the result.
    /// The returned handle joins that thread: a process that exits
    /// before it ends loses the recording, whose segments are joined
    /// into the output file last.
    pub fn stop_async(
        mut self,
        done: impl FnOnce(Result<Option<PathBuf>, String>) + Send + 'static,
    ) -> std::thread::JoinHandle<()> {
        self.signal_stop();
        let join = self.join.take().expect("stop called twice");
        let output = self.output.clone();
        std::thread::spawn(move || {
            let result = join
                .join()
                .map_err(|_| "recording thread panicked".to_string())
                .and_then(|r| Self::finish_result(r, &output));
            done(result);
        })
    }
}

impl Drop for ActiveRecording {
    /// A dropped recording still signals its source: every source loop
    /// treats the stop, or a disconnected stop channel, as a stop.
    /// Without this a leaked ActiveRecording records forever.
    fn drop(&mut self) {
        self.signal_stop();
    }
}

/// Path for a new recording inside `dir`, using the {date}_{time} template
/// and de-duplicating with a numeric suffix. `ext` is the container
/// extension without a dot ("mp4", "gif", "webm").
pub fn unique_recording_path(dir: &Path, ext: &str) -> PathBuf {
    let (y, mo, d, h, mi, s) = crate::time::local_now();
    let stem = format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{s:02}");
    let candidate = dir.join(format!("{stem}.{ext}"));
    if !candidate.exists() {
        return candidate;
    }
    for n in 2..1000 {
        let candidate = dir.join(format!("{stem}_{n}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}_{}.{ext}", crate::time::now_millis()))
}

#[cfg(test)]
mod tests;
