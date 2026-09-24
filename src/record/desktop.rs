//! Desktop recording on Windows and macOS: ffmpeg's own screen capture
//! (gdigrab on Windows, avfoundation on macOS) writes each unpaused
//! stretch as a Matroska segment, and `join` makes the segments the
//! output file. A region is a gdigrab rect on Windows and a crop of the
//! screen that holds it on macOS.
#![cfg(any(windows, target_os = "macos"))]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::child::{drain, wait_until, written, Closing};
use super::codec::{audio_args, even, VideoCodec};
use super::join::{self, segment_path, Segment};
use super::{RecControl, RecordingSpec};
use crate::capture::WinRect;
use crate::config::RecordingFormat;
use crate::tools::Tool;

/// How long a segment may take to write its trailer after `q`. A
/// stuck device can hang it; ffmpeg is killed at the limit.
const FLUSH_LIMIT: Duration = Duration::from_secs(10);

/// The control poll period: the latency of a stop or a chip control.
const TICK: Duration = Duration::from_millis(50);

/// Crops a whole screen to even dimensions: yuv420p encoders reject
/// odd ones.
const EVEN: &str = "crop=trunc(iw/2)*2:trunc(ih/2)*2";

/// ffmpeg's device listing for `format` (printed on stderr; ffmpeg then
/// fails to open the dummy input, which is expected).
fn list_devices(format: &str) -> Result<String, String> {
    let out = Tool::Ffmpeg
        .command()
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
        .map_err(|e| Tool::Ffmpeg.spawn_error(&e))?;
    Ok(String::from_utf8_lossy(&out.stderr).into_owned())
}

/// The screen input of a recording, resolved once.
struct Screen {
    /// Input options, up to the device.
    args: Vec<String>,
    /// gdigrab's `desktop`, or avfoundation's screen index.
    device: String,
    /// The filter that crops the capture to the region, or to even
    /// dimensions.
    crop: String,
    /// The recorded size, when a region sets it.
    canvas: Option<(u32, u32)>,
    /// The mic device, once found.
    mic: Option<String>,
}

#[cfg(windows)]
fn screen(fps: u32, region: Option<WinRect>) -> Result<Screen, String> {
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
    Ok(Screen {
        args,
        device: "desktop".into(),
        crop: EVEN.into(),
        canvas: region.map(|r| (even(r.width), even(r.height))),
        mic: None,
    })
}

/// The first dshow audio device.
#[cfg(windows)]
fn find_mic() -> Result<String, String> {
    super::devices::dshow_first_audio(&list_devices("dshow")?).ok_or_else(|| {
        "no microphone found (ffmpeg lists no dshow audio device); record without mic".into()
    })
}

/// The screen, then the mic as its own dshow input.
#[cfg(windows)]
fn input_args(screen: &Screen, mic: Option<&str>) -> Vec<String> {
    let mut v = screen.args.clone();
    v.extend(["-i".into(), screen.device.clone()]);
    if let Some(name) = mic {
        v.extend([
            "-f".into(),
            "dshow".into(),
            "-i".into(),
            format!("audio={name}"),
        ]);
    }
    v
}

#[cfg(target_os = "macos")]
fn screen(fps: u32, region: Option<WinRect>) -> Result<Screen, String> {
    let (ord, crop) = crate::capture::macos::recording_screen(region)?;
    let (screens, audio) = super::devices::avfoundation_indices(&list_devices("avfoundation")?);
    let screen = *screens.get(ord).ok_or(
        "ffmpeg lists no screen capture device for this display: grant Screen Recording \
         to iris in System Settings > Privacy & Security",
    )?;
    let mut args: Vec<String> = ["-f", "avfoundation", "-framerate"]
        .map(String::from)
        .to_vec();
    args.push(fps.to_string());
    args.extend(["-capture_cursor", "1"].map(String::from));
    Ok(Screen {
        args,
        device: screen.to_string(),
        crop: match crop {
            Some((x, y, w, h)) => format!("crop={}:{}:{x}:{y}", even(w), even(h)),
            None => EVEN.into(),
        },
        canvas: crop.map(|(_, _, w, h)| (even(w), even(h))),
        mic: audio.map(|a| a.to_string()),
    })
}

/// The first avfoundation audio device.
#[cfg(target_os = "macos")]
fn find_mic() -> Result<String, String> {
    let (_, audio) = super::devices::avfoundation_indices(&list_devices("avfoundation")?);
    audio.map(|a| a.to_string()).ok_or_else(|| {
        "no microphone found (ffmpeg lists no avfoundation audio device); record without mic".into()
    })
}

/// One avfoundation input holds the screen and the mic, so both run on
/// the device's clock.
#[cfg(target_os = "macos")]
fn input_args(screen: &Screen, mic: Option<&str>) -> Vec<String> {
    let mut v = screen.args.clone();
    v.extend([
        "-i".into(),
        format!("{}:{}", screen.device, mic.unwrap_or("none")),
    ]);
    v
}

/// A segment ffmpeg is capturing into. Dropped, it kills ffmpeg: a
/// capture never outlives its recording.
struct Open {
    child: Child,
    segment: Segment,
    stderr: Option<JoinHandle<String>>,
}

impl Open {
    /// `q` on stdin: ffmpeg stops capturing, writes the trailer, and
    /// exits.
    fn finish(mut self) -> Result<Segment, String> {
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = stdin.write_all(b"q");
        }
        let status = wait_until(&mut self.child, Instant::now() + FLUSH_LIMIT);
        let text = self.stderr.take().and_then(|e| e.join().ok());
        match status? {
            s if s.success() => written(self.segment.clone()),
            s => Err(format!("ffmpeg exited {s}: {}", text.unwrap_or_default())),
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One recording: its settings, the open segment, and those finishing.
struct Desktop {
    screen: Screen,
    format: RecordingFormat,
    codec: VideoCodec,
    output: PathBuf,
    mic: bool,
    paused: bool,
    open: Option<Open>,
    closing: Vec<Closing>,
}

impl Desktop {
    /// Start the next segment, with the mic when it is on.
    fn open(&mut self) -> Result<(), String> {
        if self.mic && self.screen.mic.is_none() {
            self.screen.mic = Some(find_mic()?);
        }
        let mic = self.screen.mic.as_deref().filter(|_| self.mic);
        let audio = mic.and_then(|_| audio_args(self.format));
        let path = segment_path(&self.output, self.closing.len());
        let mut c = Tool::Ffmpeg.command();
        c.args(["-hide_banner", "-loglevel", "error", "-nostats", "-y"])
            .args(input_args(&self.screen, mic))
            .args(["-vf", self.screen.crop.as_str()])
            .args(self.codec.args());
        match audio {
            Some(a) => c.args(a),
            None => c.arg("-an"),
        };
        c.args(["-sn", "-f", "matroska"])
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = c.spawn().map_err(|e| Tool::Ffmpeg.spawn_error(&e))?;
        let stderr = child.stderr.take();
        let mut open = Open {
            child,
            segment: Segment {
                path,
                audio: audio.is_some(),
            },
            stderr: None,
        };
        if let Some(err) = stderr {
            // A failed drain drops `open`, which kills ffmpeg.
            open.stderr = Some(drain(err).map_err(|e| format!("read ffmpeg errors: {e}"))?);
        }
        self.open = Some(open);
        Ok(())
    }

    /// End the open segment; ffmpeg finishes it on its own thread.
    fn close(&mut self) {
        if let Some(open) = self.open.take() {
            self.closing.push(Closing::spawn(move || open.finish()));
        }
    }

    fn apply(&mut self, ctl: RecControl) -> Result<(), String> {
        match ctl {
            RecControl::Pause if !self.paused => {
                self.paused = true;
                self.close();
                Ok(())
            }
            RecControl::Resume if self.paused => {
                self.paused = false;
                self.open()
            }
            RecControl::ToggleMic if self.format.audio_codec().is_some() => {
                self.mic = !self.mic;
                if self.paused {
                    return Ok(());
                }
                self.close();
                self.open()
            }
            RecControl::Pause | RecControl::Resume | RecControl::ToggleMic => Ok(()),
        }
    }

    /// An error once the open segment's ffmpeg exited by itself; what
    /// it wrote still joins.
    fn check(&mut self) -> Result<(), String> {
        let Some(open) = self.open.as_mut() else {
            return Ok(());
        };
        let status = match open.child.try_wait() {
            Ok(None) => return Ok(()),
            Ok(Some(status)) => status,
            Err(e) => return Err(format!("wait on ffmpeg: {e}")),
        };
        let mut open = self.open.take().expect("open segment");
        let text = open.stderr.take().and_then(|e| e.join().ok());
        let segment = open.segment.clone();
        self.closing.push(Closing::spawn(move || written(segment)));
        Err(format!(
            "ffmpeg exited {status} during capture: {}",
            text.unwrap_or_default()
        ))
    }

    /// Capture until stop, applying chip controls as they arrive.
    fn run(&mut self, spec: &RecordingSpec) -> Result<(), String> {
        loop {
            // A stop is a send OR a disconnect.
            match spec.stop.recv_timeout(TICK) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            while let Ok(ctl) = spec.control.try_recv() {
                self.apply(ctl)?;
            }
            self.check()?;
        }
    }
}

/// Record the desktop, or `region` (root pixels) of it, until stop.
pub fn record_desktop(spec: RecordingSpec, region: Option<WinRect>) -> Result<PathBuf, String> {
    let screen = screen(spec.fps, region)?;
    let codec = VideoCodec::for_recording(spec.format, spec.encoder, screen.canvas);
    let mut rec = Desktop {
        screen,
        format: spec.format,
        codec,
        output: spec.output.clone(),
        mic: spec.mic,
        paused: false,
        open: None,
        closing: Vec::new(),
    };
    rec.open()?;
    let result = rec.run(&spec);
    rec.close();
    let closing = std::mem::take(&mut rec.closing);
    match result {
        Ok(()) => join::finish(closing, rec.format, &rec.output),
        Err(e) => Err(join::finish_after(e, closing, rec.format, &rec.output)),
    }
}

#[cfg(test)]
mod tests {
    // WHY: the class closed here is "the recording is not what was on
    // screen for the unpaused time": inputs ffmpeg rejects (option order,
    // odd sizes), a region that is not applied, paused time kept in the
    // file, or a mic toggle that breaks the join. Needs an interactive
    // desktop and ffmpeg, so ignored by default: `cargo test --
    // --ignored desktop_` from the logged-in session. The mic track is
    // not covered (it needs a microphone).
    use super::*;
    use std::path::Path;
    use std::sync::mpsc::channel;

    fn probe(path: &Path) -> (u32, u32, f64) {
        let out = std::process::Command::new("ffprobe")
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
        for format in [RecordingFormat::Mp4, RecordingFormat::Webm] {
            let output = dir.join(format!("region.{}", format.ext()));
            let (stop_tx, stop) = channel();
            let (ctl_tx, control) = channel();
            let spec = RecordingSpec {
                output: output.clone(),
                fps: 30,
                mic: false,
                format,
                encoder: crate::config::RecordingEncoder::Libx264,
                stop,
                control,
                bell: Default::default(),
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
            assert_eq!((w, h), (320, 240), "{format:?}");
            // ~4s recorded; the 3s pause is not in the file.
            assert!((3.0..5.5).contains(&secs), "{format:?} duration {secs}");
            let leftovers: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().contains(".part"))
                .collect();
            assert!(leftovers.is_empty(), "segments left behind");
        }
    }
}
