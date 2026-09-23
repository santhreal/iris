//! Recording sources, one per platform, behind one interface. The
//! daemon owns session state (one active recording at a time, the chip
//! lifecycle); these functions start the platform source.
//!
//! Linux records a picked window (the Wayland portal, or X11 with the
//! chip following the window) or an X11 root rect. Windows and macOS
//! record the desktop through ffmpeg, cropped to a region when given.

use std::path::PathBuf;

use gpui::App;
use iris_lib::capture::WinRect;
use iris_lib::config::{Config, RecordingEncoder, RecordingFormat};
use iris_lib::record::{self, ActiveRecording};

#[cfg(any(windows, target_os = "macos"))]
mod desktop;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(any(windows, target_os = "macos"))]
use desktop as native;
#[cfg(target_os = "linux")]
use linux as native;

/// The parameters every source shares: output path under the
/// configured template, frame rate, mic default, container and codec.
pub struct Params {
    pub output: PathBuf,
    pub fps: u32,
    pub mic: bool,
    pub format: RecordingFormat,
    pub encoder: RecordingEncoder,
}

impl Params {
    pub fn from_config(cfg: &Config) -> Self {
        let ext = match cfg.recording_format {
            RecordingFormat::Mp4 => "mp4",
            RecordingFormat::Gif => "gif",
            RecordingFormat::Webm => "webm",
        };
        Self {
            output: record::unique_recording_path(&cfg.recordings_dir, ext),
            fps: cfg.recording_fps,
            mic: cfg.record_mic_default,
            format: cfg.recording_format,
            encoder: cfg.recording_encoder,
        }
    }

    /// Spawn `source` with these parameters.
    fn spawn<F>(self, source: F) -> ActiveRecording
    where
        F: FnOnce(record::RecordingSpec) -> Result<PathBuf, String> + Send + 'static,
    {
        ActiveRecording::spawn(
            self.output,
            self.fps,
            self.mic,
            self.format,
            self.encoder,
            source,
        )
    }
}

/// Ok when this session can record a region; the error explains why
/// not before any overlay or chip opens.
pub fn region_available() -> Result<(), String> {
    native::region_available()
}

/// Start the default recording: a picked window on Linux, the main
/// display on Windows and macOS. Opens the chip.
pub fn start_window(cx: &mut App, p: Params) -> Result<ActiveRecording, String> {
    native::start_window(cx, p)
}

/// Start recording `rect` (root pixels). Opens the chip.
pub fn start_region(cx: &mut App, p: Params, rect: WinRect) -> Result<ActiveRecording, String> {
    native::start_region(cx, p, rect)
}
