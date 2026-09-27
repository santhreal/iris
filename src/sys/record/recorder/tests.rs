// WHY: the class closed here is "the file is not what was on screen
// for the unpaused time": paused time kept in the output, a still source
// after a resume that ends the file early, a resize that changes the
// output size, a toggle that splits a format without audio, or a failure
// that discards the frames before it. Frames carry synthetic capture
// times, so the expected durations are exact and no test sleeps. Runs
// ffmpeg and ffprobe. Not covered: the mic track (needs a PulseAudio
// server; the recording smoke test covers it).

use std::path::Path;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use super::*;
use crate::record::CANCELLED_PREFIX;

const SMALL: Shape = Shape {
    width: 64,
    height: 48,
    pix: PixFmt::Bgrx,
};

const LARGE: Shape = Shape {
    width: 80,
    height: 60,
    pix: PixFmt::Bgrx,
};

fn spec(output: PathBuf, format: RecordingFormat) -> RecordingSpec {
    RecordingSpec {
        output,
        fps: 30,
        mic: false,
        format,
        encoder: RecordingEncoder::Libx264,
        stop: channel().1,
        control: channel().1,
        bell: Default::default(),
    }
}

/// Frames of `shape` every 100 ms from `from` to `to` ms after `t0`.
fn feed(rec: &mut Recorder, t0: Instant, (from, to): (u64, u64), shape: Shape) {
    let len = shape.width as usize * shape.height as usize * shape.pix.bytes_per_pixel();
    for ms in (from..=to).step_by(100) {
        let mut buf = rec.take_buf();
        buf.clear();
        buf.resize(len, (ms / 10) as u8);
        rec.frame(buf, shape, t0 + Duration::from_millis(ms))
            .unwrap();
    }
}

fn at(t0: Instant, ms: u64) -> Instant {
    t0 + Duration::from_millis(ms)
}

/// Width, height, and duration in seconds of `path`'s video.
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
        .expect("ffprobe runs");
    let text = String::from_utf8_lossy(&out.stdout);
    let v: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(v.len(), 3, "ffprobe {}: {text}", path.display());
    (
        v[0].parse().unwrap(),
        v[1].parse().unwrap(),
        v[2].parse().unwrap(),
    )
}

/// Files in `dir` other than `output`.
fn leftovers(dir: &Path, output: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p != output)
        .collect()
}

#[test]
fn pause_leaves_the_paused_span_out_of_every_format() {
    for format in [
        RecordingFormat::Mp4,
        RecordingFormat::Webm,
        RecordingFormat::Gif,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join(format!("r.{}", format.ext()));
        let mut rec = Recorder::new(&spec(output.clone(), format));
        let t0 = Instant::now();
        feed(&mut rec, t0, (0, 1000), SMALL);
        rec.apply(RecControl::Pause, at(t0, 1000)).unwrap();
        rec.apply(RecControl::Resume, at(t0, 3000)).unwrap();
        feed(&mut rec, t0, (3100, 4000), SMALL);
        assert_eq!(rec.finish(at(t0, 4000)).unwrap(), output);
        // Two recorded seconds, each segment ending one nominal frame
        // after its last frame; the two paused seconds are gone.
        let (w, h, secs) = probe(&output);
        assert_eq!((w, h), (64, 48), "{format:?}");
        assert!((1.9..2.3).contains(&secs), "{format:?}: {secs}s");
        assert_eq!(leftovers(dir.path(), &output), Vec::<PathBuf>::new());
    }
}

/// A segment of two frames, the held one and its repeat at stop: every
/// codec has to keep that stop time through the join. A reordered
/// H.264 stream lost it and the MP4 ended at the first segment.
#[test]
fn a_still_source_after_resume_lasts_until_stop() {
    for format in [
        RecordingFormat::Mp4,
        RecordingFormat::Webm,
        RecordingFormat::Gif,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join(format!("r.{}", format.ext()));
        let mut rec = Recorder::new(&spec(output.clone(), format));
        let t0 = Instant::now();
        feed(&mut rec, t0, (0, 1000), SMALL);
        rec.apply(RecControl::Pause, at(t0, 1000)).unwrap();
        rec.apply(RecControl::Resume, at(t0, 3000)).unwrap();
        // No frame after the resume: the picture did not change.
        rec.finish(at(t0, 5000)).unwrap();
        let (_, _, secs) = probe(&output);
        assert!((2.9..3.3).contains(&secs), "{format:?}: {secs}s");
    }
}

#[test]
fn a_resize_keeps_the_first_frame_size() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("r.mp4");
    let mut rec = Recorder::new(&spec(output.clone(), RecordingFormat::Mp4));
    let t0 = Instant::now();
    feed(&mut rec, t0, (0, 500), SMALL);
    feed(&mut rec, t0, (600, 1000), LARGE);
    assert_eq!(rec.closing.len(), 1, "the resize closes the first segment");
    rec.finish(at(t0, 1000)).unwrap();
    let (w, h, secs) = probe(&output);
    assert_eq!((w, h), (64, 48));
    assert!((0.9..1.2).contains(&secs), "{secs}s");
    assert_eq!(leftovers(dir.path(), &output), Vec::<PathBuf>::new());
}

#[test]
fn a_mic_toggle_without_an_audio_track_keeps_the_segment() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("r.gif");
    let mut rec = Recorder::new(&spec(output.clone(), RecordingFormat::Gif));
    let t0 = Instant::now();
    feed(&mut rec, t0, (0, 300), SMALL);
    rec.apply(RecControl::ToggleMic, at(t0, 300)).unwrap();
    assert!(rec.open.is_some() && rec.closing.is_empty());
    assert!(!rec.mic);
    rec.finish(at(t0, 300)).unwrap();
    assert_eq!(probe(&output).0, 64);
}

#[test]
fn stopping_before_the_first_frame_is_a_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("r.mp4");
    let mut rec = Recorder::new(&spec(output.clone(), RecordingFormat::Mp4));
    let t0 = Instant::now();
    rec.apply(RecControl::Pause, t0).unwrap();
    rec.apply(RecControl::Resume, at(t0, 100)).unwrap();
    let err = rec.finish(at(t0, 200)).unwrap_err();
    assert!(err.starts_with(CANCELLED_PREFIX), "{err}");
    assert_eq!(leftovers(dir.path(), &output), Vec::<PathBuf>::new());
    assert!(!output.exists());
}

#[test]
fn a_failure_keeps_the_frames_before_it() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("r.mp4");
    let mut rec = Recorder::new(&spec(output.clone(), RecordingFormat::Mp4));
    let t0 = Instant::now();
    feed(&mut rec, t0, (0, 1000), SMALL);
    let err = rec.fail(at(t0, 1000), "grab failed".to_string());
    assert_eq!(
        err,
        format!(
            "grab failed; the recording up to it is at {}",
            output.display()
        )
    );
    let (_, _, secs) = probe(&output);
    assert!((0.9..1.2).contains(&secs), "{secs}s");
}
