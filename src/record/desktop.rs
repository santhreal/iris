// Desktop recording for Windows and macOS: one ffmpeg process captures
// (gdigrab / avfoundation) and encodes in the same pipeline. Per-window
// capture with a live border is X11/Wayland-only; these platforms record
// the primary display.
#![cfg(any(windows, target_os = "macos"))]

use super::RecordingSpec;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn record_desktop(spec: RecordingSpec) -> Result<std::path::PathBuf, String> {
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
        args.extend(["-f".into(), "avfoundation".into(), "-i".into(), ":0".into()]);
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

    // Wait on the stop channel; a stop lands immediately instead of
    // up to a poll interval late. ffmpeg dying on its own still exits
    // promptly through try_wait.
    loop {
        match spec.stop.recv_timeout(Duration::from_millis(120)) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return Err(format!("ffmpeg exited during capture: {status}"));
            }
            Ok(None) => {}
            Err(e) => return Err(format!("ffmpeg poll: {e}")),
        }
    }

    // 'q' on stdin makes ffmpeg finalize the MP4 trailer and exit cleanly.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"q");
    }
    // A stuck device can hang finalize; bound the wait, then kill.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(spec.output.clone()),
            Ok(Some(status)) => {
                return Err(format!("ffmpeg exited {status} during finalize"));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("ffmpeg did not finalize within 10s; killed".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(format!("ffmpeg wait: {e}")),
        }
    }
}
