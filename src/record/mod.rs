#[cfg(target_os = "linux")]
pub mod encoder;

#[cfg(target_os = "linux")]
pub mod x11;

#[cfg(target_os = "linux")]
pub mod wayland;

#[cfg(any(windows, target_os = "macos"))]
pub mod desktop;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Instant;

/// What a platform recording source needs: where to encode, at what rate,
/// and the signal to stop. The source blocks until `stop` fires or it fails,
/// having written the finished mp4 to `spec.output`.
pub struct RecordingSpec {
    pub output: PathBuf,
    pub fps: u32,
    pub mic: bool,
    pub stop: Receiver<()>,
}

/// User aborted before any frame was encoded (e.g. Escape during window
/// pick). Sources signal this with an error message starting with this
/// prefix; the session then tears down quietly without reporting failure.
pub const CANCELLED_PREFIX: &str = "cancelled:";

pub enum Phase {
    /// Source is still picking its target (e.g. waiting for the user to
    /// click a window).
    Picking,
    Recording,
}

pub struct ActiveRecording {
    pub output: PathBuf,
    pub started: Instant,
    pub mic: bool,
    pub phase: Phase,
    stop: Option<Sender<()>>,
    join: Option<JoinHandle<Result<(), String>>>,
}

impl ActiveRecording {
    /// Spawn a platform source on its own thread. `source` must honor
    /// `spec.stop` promptly and return after the encoder has finished.
    pub fn spawn<F>(output: PathBuf, fps: u32, mic: bool, source: F) -> Self
    where
        F: FnOnce(RecordingSpec) -> Result<(), String> + Send + 'static,
    {
        let (tx, rx) = channel();
        let spec = RecordingSpec {
            output: output.clone(),
            fps,
            mic,
            stop: rx,
        };
        let join = std::thread::spawn(move || source(spec));
        Self {
            output,
            started: Instant::now(),
            mic,
            phase: Phase::Picking,
            stop: Some(tx),
            join: Some(join),
        }
    }

    /// Signal stop and wait for the source to flush the file. A `cancelled:`
    /// result is reported as Ok(None); anything else is a real error.
    pub fn stop(mut self) -> Result<Option<PathBuf>, String> {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        let result = self
            .join
            .take()
            .expect("stop called twice")
            .join()
            .map_err(|_| "recording thread panicked".to_string())?;
        match result {
            Ok(()) => Ok(Some(self.output)),
            Err(e) if e.starts_with(CANCELLED_PREFIX) => {
                let _ = std::fs::remove_file(&self.output);
                Ok(None)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&self.output);
                Err(e)
            }
        }
    }
}

#[derive(Default)]
pub struct RecordingManager {
    pub active: Option<ActiveRecording>,
}

impl RecordingManager {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }    /// Stop the active recording, if any. Returns the finished file path,
    /// or None when the session was cancelled before encoding.
    pub fn stop(&mut self) -> Result<Option<PathBuf>, String> {
        match self.active.take() {
            Some(rec) => rec.stop(),
            None => Ok(None),
        }
    }
}

/// Path for a new recording inside `dir`, using the {date}_{time} template
/// and de-duplicating with a numeric suffix.
pub fn unique_recording_path(dir: &Path) -> PathBuf {
    let now = chrono::Local::now();
    let stem = format!("{}_{}", now.format("%Y-%m-%d"), now.format("%H-%M-%S"));
    let candidate = dir.join(format!("{stem}.mp4"));
    if !candidate.exists() {
        return candidate;
    }
    for n in 2..1000 {
        let candidate = dir.join(format!("{stem}_{n}.mp4"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}_{}.mp4", now.timestamp_millis()))
}
