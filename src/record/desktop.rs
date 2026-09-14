// Desktop recording for Windows and macOS: one ffmpeg process captures
// (gdigrab / avfoundation) and encodes in the same pipeline. Per-window
// capture with a live border is X11/Wayland-only; these platforms record
// the primary display.
#![cfg(any(windows, target_os = "macos"))]

use super::RecordingSpec;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn record_desktop(spec: RecordingSpec) -> Result<(), String> {
    #[cfg(windows)]
    let input = crate::capture::windows::capture_args();
    #[cfg(target_os = "macos")]
    let input = crate::capture::macos::capture_args();

    let mut args: Vec<String> = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    args.extend(input);
    args.extend(["-framerate".into(), spec.fps.to_string()]);
    let mut map_audio = false;
    if spec.mic {
        #[cfg(windows)]
        args.extend([
            "-f".into(),
            "dshow".into(),
            "-i".into(),
            "audio=default".into(),
        ]);
        #[cfg(target_os = "macos")]
        args.extend([
            "-f".into(),
            "avfoundation".into(),
            "-i".into(),
            ":0".into(),
        ]);
        map_audio = true;
    }
    args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-crf".into(),
        "20".into(),
    ]);
    if map_audio {
        args.extend(["-c:a".into(), "aac".into(), "-shortest".into()]);
    }
    args.push("-y".into());
    args.push(spec.output.to_string_lossy().to_string());

    let mut child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg start (is ffmpeg on PATH?): {e}"))?;

    // Poll the stop channel; exit promptly when ffmpeg dies on its own.
    loop {
        match spec.stop.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!("ffmpeg exited during capture: {status}"));
            }
            Ok(None) => {}
            Err(e) => return Err(format!("ffmpeg poll: {e}")),
        }
        std::thread::sleep(Duration::from_millis(120));
    }

    // 'q' on stdin makes ffmpeg finalize the MP4 trailer and exit cleanly.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"q");
    }
    match child.wait() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("ffmpeg exited {status} during finalize")),
        Err(e) => Err(format!("ffmpeg wait: {e}")),
    }
}
