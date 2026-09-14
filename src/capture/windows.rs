// Windows capture via ffmpeg gdigrab. ffmpeg is already the encoder
// dependency, so this backend adds no crates. It records the primary
// desktop: gdigrab has no interactive window picker, so the target is
// the whole desktop and the stop path is the tray, the --stop-recording
// CLI flag, or the record hotkey.
#![cfg(windows)]

use crate::capture::Frame;
use std::process::Command;

pub fn capture_args() -> Vec<String> {
    vec![
        "-f".into(),
        "gdigrab".into(),
        "-i".into(),
        "desktop".into(),
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
        .map_err(|e| format!("ffmpeg gdigrab failed to start (is ffmpeg on PATH?): {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg gdigrab exited {status}"));
    }
    let png = std::fs::read(&out).map_err(|e| format!("read grab: {e}"))?;
    let _ = std::fs::remove_file(&out);
    let img = image::load_from_memory(&png)
        .map_err(|e| format!("decode gdigrab frame: {e}"))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Ok(Frame {
        width,
        height,
        rgba: img.into_raw(),
    })
}

pub struct WindowsBackend;

impl crate::capture::CaptureBackend for WindowsBackend {
    fn grab_screen(&self) -> Result<Frame, String> {
        capture_full_frame()
    }
}
