// Desktop recording for Windows and macOS through ffmpeg: gdigrab reads
// the desktop (or a region of it) on Windows, avfoundation one screen
// on macOS (a region is a crop of it). Each unpaused stretch is its own
// ffmpeg segment; stop joins the segments into the output file.
#![cfg(any(windows, target_os = "macos"))]

use super::devices::{concat_list, even};
use super::{RecControl, RecordingSpec};
use crate::capture::WinRect;
use crate::config::RecordingFormat;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

/// ffmpeg's device listing for `format` (printed on stderr; ffmpeg then
/// fails to open the dummy input, which is expected).
fn list_devices(format: &str) -> Result<String, String> {
    let out = ffmpeg()
        .args([
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            format,
            "-i",
            "dummy",
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg start (is ffmpeg on PATH?): {e}"))?;
    Ok(String::from_utf8_lossy(&out.stderr).into_owned())
}

fn ffmpeg() -> Command {
    let mut c = Command::new("ffmpeg");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// The ffmpeg inputs for one segment, resolved once per recording.
struct Inputs {
    /// Input arguments (`-f ... -i ...`), screen first, then the mic.
    args: Vec<String>,
    /// Video filter prefix (a crop), if any.
    crop: Option<String>,
    audio: bool,
}

#[cfg(windows)]
fn inputs(fps: u32, mic: bool, region: Option<WinRect>) -> Result<Inputs, String> {
    let mut args: Vec<String> = ["-f", "gdigrab", "-framerate"].map(String::from).to_vec();
    args.push(fps.to_string());
    args.extend(["-draw_mouse", "1"].map(String::from));
    if let Some(r) = region {
        // Input options: they must precede `-i` to reach gdigrab.
        args.extend([
            "-offset_x".into(),
            r.x.to_string(),
            "-offset_y".into(),
            r.y.to_string(),
            "-video_size".into(),
            format!("{}x{}", even(r.width), even(r.height)),
        ]);
    }
    args.extend(["-i", "desktop"].map(String::from));
    if mic {
        let name = super::devices::dshow_first_audio(&list_devices("dshow")?).ok_or(
            "no microphone found (ffmpeg lists no dshow audio device); record without mic",
        )?;
        args.extend([
            "-f".into(),
            "dshow".into(),
            "-i".into(),
            format!("audio={name}"),
        ]);
    }
    Ok(Inputs {
        args,
        crop: None,
        audio: mic,
    })
}

#[cfg(target_os = "macos")]
fn inputs(fps: u32, mic: bool, region: Option<WinRect>) -> Result<Inputs, String> {
    let (ord, crop) = crate::capture::macos::recording_screen(region)?;
    let (screens, audio) = super::devices::avfoundation_indices(&list_devices("avfoundation")?);
    let screen = *screens.get(ord).ok_or(
        "ffmpeg lists no screen capture device for this display: grant Screen Recording \
         to iris in System Settings > Privacy & Security",
    )?;
    let audio = if mic {
        Some(audio.ok_or(
            "no microphone found (ffmpeg lists no avfoundation audio device); record without mic",
        )?)
    } else {
        None
    };
    let device = match audio {
        Some(a) => format!("{screen}:{a}"),
        None => format!("{screen}:none"),
    };
    let mut args: Vec<String> = ["-f", "avfoundation", "-framerate"]
        .map(String::from)
        .to_vec();
    args.push(fps.to_string());
    args.extend(["-capture_cursor", "1", "-i"].map(String::from));
    args.push(device);
    Ok(Inputs {
        args,
        crop: crop.map(|(x, y, w, h)| format!("crop={}:{}:{x}:{y}", even(w), even(h))),
        audio: audio.is_some(),
    })
}

/// Encoder arguments for one segment. GIF segments are intermediate
/// H.264; the palette pass runs once over the joined result.
fn segment_codec(format: RecordingFormat, audio: bool) -> Vec<&'static str> {
    let mut v = match format {
        RecordingFormat::Mp4 => vec!["-c:v", "libx264", "-preset", "veryfast", "-crf", "20"],
        RecordingFormat::Gif => vec!["-c:v", "libx264", "-preset", "ultrafast", "-crf", "12"],
        RecordingFormat::Webm => vec![
            "-c:v",
            "libvpx-vp9",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
            "-row-mt",
            "1",
            "-b:v",
            "0",
            "-crf",
            "32",
        ],
    };
    v.extend(["-pix_fmt", "yuv420p"]);
    v.extend(if audio {
        ["-c:a", "aac"]
    } else {
        ["-an", "-sn"]
    });
    v
}

fn segment_path(output: &Path, format: RecordingFormat, n: usize) -> PathBuf {
    let ext = match format {
        RecordingFormat::Webm => "webm",
        RecordingFormat::Mp4 | RecordingFormat::Gif => "mp4",
    };
    output.with_extension(format!("part{n}.{ext}"))
}

fn start_segment(inp: &Inputs, format: RecordingFormat, path: &Path) -> Result<Child, String> {
    let mut c = ffmpeg();
    c.args(["-hide_banner", "-loglevel", "error"])
        .args(&inp.args);
    if let Some(crop) = &inp.crop {
        c.args(["-vf", crop]);
    }
    c.args(segment_codec(format, inp.audio))
        .arg("-y")
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg start (is ffmpeg on PATH?): {e}"))
}

/// `q` on stdin makes ffmpeg write the trailer and exit. A stuck device
/// can hang the finalize, so the wait is bounded, then the child killed.
fn finish_segment(mut child: Child) -> Result<(), String> {
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"q");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => return Ok(()),
            Ok(Some(s)) => return Err(format!("ffmpeg exited {s} during finalize")),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("ffmpeg did not finalize within 10s; killed".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("ffmpeg wait: {e}")),
        }
    }
}

/// Join `segments` into `output`: a stream copy for mp4/webm, the
/// palette pass for GIF.
fn join(segments: &[PathBuf], format: RecordingFormat, output: &Path) -> Result<(), String> {
    if segments.len() == 1 && format != RecordingFormat::Gif {
        return std::fs::rename(&segments[0], output)
            .map_err(|e| format!("move {} into place: {e}", segments[0].display()));
    }
    let list = output.with_extension("parts.txt");
    let refs: Vec<&Path> = segments.iter().map(PathBuf::as_path).collect();
    std::fs::write(&list, concat_list(&refs))
        .map_err(|e| format!("write {}: {e}", list.display()))?;
    let mut c = ffmpeg();
    c.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ])
    .arg(&list);
    match format {
        RecordingFormat::Gif => c.args([
            "-vf",
            "fps=15,scale='min(iw,960)':-2:flags=lanczos,split[a][b];[a]palettegen[p];[b][p]paletteuse",
        ]),
        _ => c.args(["-c", "copy"]),
    };
    let status = c.arg("-y").arg(output).stdin(Stdio::null()).status();
    let _ = std::fs::remove_file(&list);
    match status {
        Ok(s) if s.success() => {
            for s in segments {
                let _ = std::fs::remove_file(s);
            }
            Ok(())
        }
        Ok(s) => Err(format!(
            "ffmpeg join exited {s}; segments kept beside the output"
        )),
        Err(e) => Err(format!("ffmpeg join start: {e}")),
    }
}

/// Record the desktop, or `region` (root pixels) of it, until stop.
/// Pause ends the current segment; resume starts the next.
pub fn record_desktop(spec: RecordingSpec, region: Option<WinRect>) -> Result<PathBuf, String> {
    // GIF and WebM carry no audio track (config contract).
    let mic = spec.mic && spec.format == RecordingFormat::Mp4;
    let inp = inputs(spec.fps, mic, region)?;
    let mut segments = vec![segment_path(&spec.output, spec.format, 0)];
    let mut child = Some(start_segment(&inp, spec.format, &segments[0])?);
    loop {
        match spec.stop.recv_timeout(Duration::from_millis(120)) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        while let Ok(ctl) = spec.control.try_recv() {
            match (ctl, child.take()) {
                (RecControl::Pause, Some(c)) => finish_segment(c)?,
                (RecControl::Resume, None) => {
                    let path = segment_path(&spec.output, spec.format, segments.len());
                    child = Some(start_segment(&inp, spec.format, &path)?);
                    segments.push(path);
                }
                // The mic is fixed for a desktop recording: segments
                // with and without an audio stream cannot be joined.
                (_, c) => child = c,
            }
        }
        if let Some(c) = child.as_mut() {
            match c.try_wait() {
                Ok(Some(status)) => return Err(format!("ffmpeg exited during capture: {status}")),
                Ok(None) => {}
                Err(e) => return Err(format!("ffmpeg poll: {e}")),
            }
        }
    }
    if let Some(c) = child {
        finish_segment(c)?;
    }
    join(&segments, spec.format, &spec.output)?;
    Ok(spec.output)
}

#[cfg(test)]
mod tests {
    // WHY: the class closed here is "the recording is not what was on
    // screen for the unpaused time": inputs ffmpeg rejects (option order,
    // odd sizes), a region that is not applied, or paused time kept in
    // the file. Needs an interactive desktop and ffmpeg, so ignored by
    // default: `cargo test -- --ignored desktop_` from the logged-in
    // session. Mic capture is not covered.
    use super::*;
    use std::sync::mpsc::channel;

    fn probe(path: &Path) -> (u32, u32, f64) {
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
            .args([
                "stream=width,height:format=duration",
                "-of",
                "default=nw=1:nk=1",
            ])
            .arg(path)
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        let v: Vec<&str> = s.split_whitespace().collect();
        (
            v[0].parse().unwrap(),
            v[1].parse().unwrap(),
            v[2].parse().unwrap(),
        )
    }

    #[test]
    #[ignore = "needs an interactive desktop session and ffmpeg"]
    fn desktop_region_recording_drops_paused_time() {
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join("rec-test");
        std::fs::create_dir_all(&dir).unwrap();
        let output = dir.join("region.mp4");
        let (stop_tx, stop) = channel();
        let (ctl_tx, control) = channel();
        let spec = RecordingSpec {
            output: output.clone(),
            fps: 30,
            mic: false,
            format: RecordingFormat::Mp4,
            encoder: crate::config::RecordingEncoder::Libx264,
            stop,
            control,
        };
        let mons = crate::capture::monitors().unwrap();
        // Odd size: the recording rounds it down to even.
        let region = WinRect {
            x: mons[0].x + 7,
            y: mons[0].y + 9,
            width: 321,
            height: 241,
        };
        let rec = std::thread::spawn(move || record_desktop(spec, Some(region)));
        std::thread::sleep(Duration::from_secs(2));
        ctl_tx.send(RecControl::Pause).unwrap();
        std::thread::sleep(Duration::from_secs(3));
        ctl_tx.send(RecControl::Resume).unwrap();
        std::thread::sleep(Duration::from_secs(2));
        stop_tx.send(()).unwrap();
        let path = rec.join().unwrap().unwrap();
        assert_eq!(path, output);
        let (w, h, secs) = probe(&path);
        assert_eq!((w, h), (320, 240));
        // ~4s recorded; the 3s pause is not in the file.
        assert!((3.0..5.5).contains(&secs), "duration {secs}");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".part"))
            .collect();
        assert!(leftovers.is_empty(), "segments left behind");
    }
}
