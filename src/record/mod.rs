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

/// Controls the chip can send a live recording. Wayland and desktop
/// sources ignore the channel: their chip is not shown.
#[derive(Clone, Copy, Debug)]
pub enum RecControl {
    Pause,
    Resume,
    ToggleMic,
}

/// What a platform recording source needs: where to encode, at what rate,
/// and the signal to stop. The source blocks until `stop` fires or it fails,
/// having written the finished mp4 to `spec.output`.
pub struct RecordingSpec {
    pub output: PathBuf,
    pub fps: u32,
    pub mic: bool,
    pub format: crate::config::RecordingFormat,
    pub encoder: crate::config::RecordingEncoder,
    pub stop: Receiver<()>,
    pub control: Receiver<RecControl>,
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
    control: Option<Sender<RecControl>>,
    join: Option<JoinHandle<Result<PathBuf, String>>>,
}

impl ActiveRecording {
    pub fn spawn<F>(
        output: PathBuf,
        fps: u32,
        mic: bool,
        format: crate::config::RecordingFormat,
        encoder: crate::config::RecordingEncoder,
        source: F,
    ) -> Self
    where
        F: FnOnce(RecordingSpec) -> Result<PathBuf, String> + Send + 'static,
    {
        let (tx, rx) = channel();
        let (ctx, crx) = channel();
        let spec = RecordingSpec {
            output: output.clone(),
            fps,
            mic,
            format,
            encoder,
            stop: rx,
            control: crx,
        };
        let join = std::thread::spawn(move || source(spec));
        Self {
            output,
            started: Instant::now(),
            mic,
            phase: Phase::Picking,
            stop: Some(tx),
            control: Some(ctx),
            join: Some(join),
        }
    }

    /// Forward a chip control to the source thread. No-op once the
    /// recording has been stopped or the source exited.
    pub fn send_control(&self, ctl: RecControl) {
        if let Some(tx) = &self.control {
            let _ = tx.send(ctl);
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
            // The source reports the LAST segment it wrote: a
            // mid-recording split (resize, mic toggle) renames the
            // output, and reporting spec.output would name a middle
            // segment as "the recording".
            Ok(path) => Ok(Some(path)),
            Err(e) if e.starts_with(CANCELLED_PREFIX) => {
                let _ = std::fs::remove_file(&self.output);
                Ok(None)
            }
            Err(e) => {
                // Keep a non-empty file: a mid-recording split (resize,
                // mic toggle) can fail on the SECOND segment while the
                // first is a complete, valid video. Only an empty stub
                // is removed.
                let empty = std::fs::metadata(&self.output)
                    .map(|m| m.len() == 0)
                    .unwrap_or(true);
                if empty {
                    let _ = std::fs::remove_file(&self.output);
                }
                Err(e)
            }
        }
    }
}

impl Drop for ActiveRecording {
    /// A dropped recording still signals its source: the stop channel
    /// disconnects, and every source loop treats a disconnected stop
    /// as a stop. Without this a leaked ActiveRecording records forever.
    fn drop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
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
    }

    /// Stop the active recording, if any. Returns the finished file path,
    /// or None when the session was cancelled before encoding.
    pub fn stop(&mut self) -> Result<Option<PathBuf>, String> {
        match self.active.take() {
            Some(rec) => rec.stop(),
            None => Ok(None),
        }
    }
}

/// The container extension of a recording path, so a mid-recording
/// split (resize, mic toggle, renegotiation) keeps the same format.
pub fn ext_of(path: &Path) -> &str {
    path.extension().and_then(|e| e.to_str()).unwrap_or("mp4")
}

/// Path for a new recording inside `dir`, using the {date}_{time} template
/// and de-duplicating with a numeric suffix. `ext` is the container
/// extension without a dot ("mp4", "gif", "webm").
pub fn unique_recording_path(dir: &Path, ext: &str) -> PathBuf {
    let now = chrono::Local::now();
    let stem = format!("{}_{}", now.format("%Y-%m-%d"), now.format("%H-%M-%S"));
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
    dir.join(format!("{stem}_{}.{ext}", now.timestamp_millis()))
}

// WHY: the class closed here is "a recording leaks or double-stops": a
// stop that leaves the thread running, a cancelled pick that leaves the
// file, or a manager that reports active after stop all hang or corrupt
// the next session. Sources are stub closures over the real channel so
// the lifecycle itself is what is tested. Not covered: ffmpeg output.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_dedupes_with_numeric_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let first = unique_recording_path(dir.path(), "mp4");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_recording_path(dir.path(), "mp4");
        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with("_2.mp4"));
    }

    #[test]
    fn stop_returns_output_on_clean_source() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("r.mp4");
        let rec = ActiveRecording::spawn(out.clone(), 30, false, crate::config::RecordingFormat::Mp4, crate::config::RecordingEncoder::Libx264, |spec| {
            spec.stop.recv().unwrap();
            std::fs::write(&spec.output, b"mp4").unwrap();
            Ok(spec.output.clone())
        });
        assert_eq!(rec.stop().unwrap(), Some(out));
    }

    #[test]
    fn cancelled_source_removes_output_and_reports_none() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("r.mp4");
        std::fs::write(&out, b"partial").unwrap();
        let rec = ActiveRecording::spawn(out.clone(), 30, false, crate::config::RecordingFormat::Mp4, crate::config::RecordingEncoder::Libx264, |spec| {
            spec.stop.recv().unwrap();
            Err(format!("{}user escaped the pick", CANCELLED_PREFIX))
        });
        assert_eq!(rec.stop().unwrap(), None);
        assert!(!out.exists());
    }

    #[test]
    fn failed_source_removes_empty_stub_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("r.mp4");
        std::fs::write(&out, b"").unwrap();
        let rec = ActiveRecording::spawn(out.clone(), 30, false, crate::config::RecordingFormat::Mp4, crate::config::RecordingEncoder::Libx264, |spec| {
            spec.stop.recv().unwrap();
            Err("encoder died".to_string())
        });
        assert!(rec.stop().is_err());
        assert!(!out.exists());
    }

    #[test]
    fn failed_source_keeps_nonempty_segment() {
        // A split can fail on the SECOND segment while the first is a
        // complete video: a non-empty file on error is kept, not deleted.
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("r.mp4");
        std::fs::write(&out, b"partial").unwrap();
        let rec = ActiveRecording::spawn(out.clone(), 30, false, crate::config::RecordingFormat::Mp4, crate::config::RecordingEncoder::Libx264, |spec| {
            spec.stop.recv().unwrap();
            Err("encoder died".to_string())
        });
        assert!(rec.stop().is_err());
        assert!(out.exists());
    }

    #[test]
    fn manager_stop_with_nothing_active_is_ok_none() {
        let mut mgr = RecordingManager::default();
        assert!(!mgr.is_active());
        assert_eq!(mgr.stop().unwrap(), None);
    }

    #[test]
    fn manager_clears_active_after_stop() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = RecordingManager::default();
        mgr.active = Some(ActiveRecording::spawn(
            dir.path().join("r.mp4"),
            30,
            false,
            crate::config::RecordingFormat::Mp4,
            crate::config::RecordingEncoder::Libx264,
            |spec| {
                spec.stop.recv().unwrap();
                Ok(spec.output.clone())
            },
        ));
        assert!(mgr.is_active());
        mgr.stop().unwrap();
        assert!(!mgr.is_active());
    }
}
