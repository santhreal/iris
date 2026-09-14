// macOS capture via ffmpeg avfoundation. ffmpeg is already the encoder
// dependency, so this backend adds no crates. It records the whole
// screen: the interactive picker is not implemented here, and macOS
// requires Screen Recording permission for the app — the ffmpeg error
// surfaces when that permission is missing.
#![cfg(target_os = "macos")]

use crate::capture::Frame;
use std::process::Command;

/// Screen device for `avfoundation`: ffmpeg indexes the main display as
/// "1" on the standard device list ("Capture screen 0" is its name on
/// recent ffmpeg builds; the numeric form is stable across versions).
pub fn capture_args() -> Vec<String> {
    vec![
        "-f".into(),
        "avfoundation".into(),
        "-capture_cursor".into(),
        "1".into(),
        "-i".into(),
        "1:".into(),
    ]
}

pub fn capture_full_frame() -> Result<Frame, String> {
    let out = std::env::temp_dir().join("iris-grab.png");
    let _ = std::fs::remove_file(&out);
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error"])
        .args(capture_args())
        .args(["-frames:v", "1", "-y"])
        .arg(&out)
        .status()
        .map_err(|e| format!("ffmpeg avfoundation failed to start (is ffmpeg installed?): {e}"))?;
    if !status.success() {
        return Err(format!(
            "ffmpeg avfoundation exited {status}; macOS needs Screen Recording permission for iris"
        ));
    }
    let png = std::fs::read(&out).map_err(|e| format!("read grab: {e}"))?;
    let _ = std::fs::remove_file(&out);
    let img = image::load_from_memory(&png)
        .map_err(|e| format!("decode avfoundation frame: {e}"))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Ok(Frame {
        width,
        height,
        rgba: img.into_raw(),
    })
}

pub struct MacosBackend;

impl crate::capture::CaptureBackend for MacosBackend {
    fn grab_screen(&self) -> Result<Frame, String> {
        capture_full_frame()
    }
}
