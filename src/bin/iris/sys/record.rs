//! Recording sources, one per platform, behind one interface. The
//! daemon owns session state (one active recording at a time, the chip
//! lifecycle); these functions start the platform source, and every
//! source sends `Command::RecordingEnded` when it returns.
//!
//! Linux records a picked window (the Wayland portal, or X11 with the
//! chip following the window) or an X11 root rect. Windows and macOS
//! record the desktop through ffmpeg, cropped to a region when given.

use std::path::PathBuf;

use gpui::App;
use iris_lib::capture::WinRect;
use iris_lib::config::{Config, RecordingEncoder, RecordingFormat};
use iris_lib::record::{self, codec, ActiveRecording};

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
    /// Whether the mic track starts on; never for a format without audio.
    pub mic: bool,
    pub format: RecordingFormat,
    pub encoder: RecordingEncoder,
}

impl Params {
    pub fn from_config(cfg: &Config) -> Self {
        let format = cfg.recording_format;
        Self {
            output: record::unique_recording_path(&cfg.recordings_dir, format.ext()),
            fps: cfg.recording_fps,
            mic: cfg.record_mic_default && format.audio_codec().is_some(),
            format,
            encoder: cfg.recording_encoder,
        }
    }

    /// The chip's mic button state: `None` hides it for a format without
    /// audio.
    pub fn chip_mic(&self) -> Option<bool> {
        self.format.audio_codec().map(|_| self.mic)
    }

    /// Spawn `source` with these parameters. The encoder probes start
    /// now, so a source that first picks its target starts encoding
    /// without waiting on them.
    fn spawn<F>(self, source: F) -> Result<ActiveRecording, String>
    where
        F: FnOnce(record::RecordingSpec) -> Result<PathBuf, String> + Send + 'static,
    {
        codec::warm(self.format, self.encoder);
        let ended = crate::daemon::command_tx();
        ActiveRecording::spawn(
            self.output,
            self.fps,
            self.mic,
            self.format,
            self.encoder,
            source,
            move || {
                if let Some(tx) = ended {
                    let _ = tx.unbounded_send(crate::daemon::Command::RecordingEnded);
                }
            },
        )
    }
}

/// Ok when ffmpeg, which encodes every recording, is installed; the
/// error is the command that installs it. Checked before any picker,
/// overlay, or chip opens.
pub fn ready() -> Result<(), String> {
    let ffmpeg = iris_lib::tools::Tool::Ffmpeg;
    ffmpeg.find().map(drop).ok_or_else(|| ffmpeg.missing())
}

/// Ok when this session can record a region; the error explains why
/// not before any overlay or chip opens.
pub fn region_available() -> Result<(), String> {
    native::region_available()
}

/// Start the default recording: a picked window on Linux, the main
/// display on Windows and macOS. Opens the chip; an X11 window pick
/// opens it once the pick lands, and a Wayland recording has none.
pub fn start_window(cx: &mut App, p: Params) -> Result<ActiveRecording, String> {
    native::start_window(cx, p)
}

/// Start recording `rect` (root pixels). Opens the chip.
pub fn start_region(cx: &mut App, p: Params, rect: WinRect) -> Result<ActiveRecording, String> {
    native::start_region(cx, p, rect)
}
