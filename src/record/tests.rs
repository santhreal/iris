// WHY: the class closed here is "a recording leaks, double-stops, or
// disagrees with its source": a stop that leaves the thread running, a
// cancelled pick that leaves the file, an end the daemon cannot tell
// apart from a running source, a rate or mic the format cannot record,
// a toggle that never reaches the source, or a send that does not ring
// a source blocked without a timeout. Sources are stub closures over
// the real channels, so the lifecycle itself is what is tested.
// Not covered: ffmpeg output.
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::time::Duration;

use super::*;

/// An mp4 recording to `out`, run by `source`.
fn spawn(
    out: PathBuf,
    source: impl FnOnce(RecordingSpec) -> Result<PathBuf, String> + Send + 'static,
) -> ActiveRecording {
    let (fmt, enc) = (RecordingFormat::Mp4, RecordingEncoder::Libx264);
    ActiveRecording::spawn(out, 30, false, fmt, enc, source, || {}).unwrap()
}

/// What a source saw: its rate, its mic, and every control sent
/// before stop.
type Seen = (u32, bool, Vec<RecControl>);

/// A recording whose source waits for stop, then reports what it saw.
fn probe(fps: u32, mic: bool, format: RecordingFormat) -> (ActiveRecording, mpsc::Receiver<Seen>) {
    let (tx, rx) = mpsc::channel();
    let rec = ActiveRecording::spawn(
        PathBuf::from("probe"),
        fps,
        mic,
        format,
        RecordingEncoder::Auto,
        move |spec| {
            let _ = spec.stop.recv();
            let _ = tx.send((spec.fps, spec.mic, spec.control.try_iter().collect()));
            Ok(spec.output)
        },
        || {},
    );
    (rec.unwrap(), rx)
}

#[test]
fn recording_reservation_dedupes_with_numeric_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let stem = "2025-01-02_03-04-05";
    let first = reserve_recording_stem(dir.path(), "mp4", stem).unwrap();
    assert_eq!(first.path(), dir.path().join(format!("{stem}.mp4")));
    let second = reserve_recording_stem(dir.path(), "mp4", stem).unwrap();
    assert_eq!(second.path(), dir.path().join(format!("{stem}_2.mp4")));
}

#[test]
fn concurrent_recording_reservations_get_every_distinct_name() {
    const WRITERS: usize = 8;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let barrier = std::sync::Barrier::new(WRITERS);
    let mut paths = std::thread::scope(|scope| {
        let writers: Vec<_> = (0..WRITERS)
            .map(|_| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    reserve_recording_stem(root, "mp4", "capture")
                        .map(crate::output::ReservedFile::keep)
                })
            })
            .collect();
        writers
            .into_iter()
            .map(|writer| writer.join().unwrap().unwrap())
            .collect::<Vec<_>>()
    });
    paths.sort();
    let names: Vec<_> = paths
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let expected: Vec<_> = [
        "capture.mp4",
        "capture_2.mp4",
        "capture_3.mp4",
        "capture_4.mp4",
        "capture_5.mp4",
        "capture_6.mp4",
        "capture_7.mp4",
        "capture_8.mp4",
    ]
    .map(str::to_string)
    .to_vec();
    assert_eq!(names, expected);
    for path in paths {
        assert_eq!(std::fs::read(path).unwrap(), b"");
    }
}

#[test]
fn an_unstarted_recording_removes_its_reserved_output() {
    let dir = tempfile::tempdir().unwrap();
    let recordings = dir.path().join("recordings");
    let output = {
        let reserved = reserve_recording_path(&recordings, "mp4").unwrap();
        let output = reserved.path().to_path_buf();
        assert!(output.is_file());
        output
    };
    assert!(!output.exists());
}

#[test]
fn stop_returns_output_on_clean_source() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("r.mp4");
    let rec = spawn(out.clone(), |spec| {
        spec.stop.recv().unwrap();
        std::fs::write(&spec.output, b"mp4").unwrap();
        Ok(spec.output)
    });
    assert_eq!(rec.stop().unwrap(), Some(out));
}

#[test]
fn cancelled_source_removes_output_and_reports_none() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("r.mp4");
    std::fs::write(&out, b"partial").unwrap();
    let rec = spawn(out.clone(), |spec| {
        spec.stop.recv().unwrap();
        Err(format!("{CANCELLED_PREFIX}user escaped the pick"))
    });
    assert_eq!(rec.stop().unwrap(), None);
    assert!(!out.exists());
}

/// A segment can fail after earlier ones joined into the output: a
/// failed source keeps a non-empty file and removes an empty stub.
#[test]
fn failed_source_keeps_only_a_nonempty_output() {
    for (stub, kept) in [(&b""[..], false), (&b"partial"[..], true)] {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("r.mp4");
        std::fs::write(&out, stub).unwrap();
        let rec = spawn(out.clone(), |spec| {
            spec.stop.recv().unwrap();
            Err("encoder died".to_string())
        });
        assert_eq!(rec.stop(), Err("encoder died".to_string()));
        assert_eq!(out.exists(), kept, "stub of {} bytes", stub.len());
    }
}

#[test]
fn source_sees_the_rate_and_mic_its_format_records() {
    for format in [
        RecordingFormat::Mp4,
        RecordingFormat::Gif,
        RecordingFormat::Webm,
    ] {
        for (fps, mic) in [(0, true), (60, true), (500, false)] {
            let (rec, seen) = probe(fps, mic, format);
            let want = (
                format.capture_fps(fps),
                mic && format.audio_codec().is_some(),
            );
            assert_eq!(rec.mic, want.1, "{format:?}");
            rec.stop().unwrap();
            let (got_fps, got_mic, _) = seen.recv().unwrap();
            assert_eq!(
                (got_fps, got_mic),
                want,
                "{format:?} at {fps} fps, mic {mic}"
            );
        }
    }
}

#[test]
fn toggles_reach_the_source_in_order() {
    let (mut rec, seen) = probe(30, true, RecordingFormat::Mp4);
    assert_eq!((rec.toggle_pause(), rec.toggle_pause()), (true, false));
    assert_eq!((rec.toggle_mic(), rec.toggle_mic()), (Ok(false), Ok(true)));
    assert!(!rec.ended(), "a source waiting for stop has not ended");
    rec.stop().unwrap();
    use RecControl::*;
    assert_eq!(
        seen.recv().unwrap().2,
        [Pause, Resume, ToggleMic, ToggleMic]
    );
}

#[test]
fn mic_toggle_fails_without_an_audio_track() {
    let (mut rec, seen) = probe(12, true, RecordingFormat::Gif);
    assert!(rec.toggle_mic().is_err());
    assert!(!rec.mic);
    rec.stop().unwrap();
    assert!(seen.recv().unwrap().2.is_empty());
}

#[test]
fn dropped_recording_stops_its_source() {
    let (rec, seen) = probe(30, false, RecordingFormat::Mp4);
    drop(rec);
    assert!(seen.recv_timeout(Duration::from_secs(5)).is_ok());
}

/// The daemon collects a recording on its end notice only when
/// `ended` reads true, so the flag is set before the hook runs, on a
/// return and on a panic. The hook blocks until the flag is read: a
/// flag set after the hook reads false every time.
#[test]
fn ended_is_set_before_the_end_hook_runs() {
    for panics in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (hook_tx, hook_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let rec = ActiveRecording::spawn(
            dir.path().join("r.mp4"),
            30,
            false,
            RecordingFormat::Mp4,
            RecordingEncoder::Auto,
            move |_| {
                assert!(!panics, "source bug");
                Err("the window closed".to_string())
            },
            move || {
                let _ = hook_tx.send(());
                let _ = go_rx.recv();
            },
        )
        .unwrap();
        hook_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(rec.ended(), "panics: {panics}");
        drop(go_tx);
        let want = if panics {
            "recording thread panicked"
        } else {
            "the window closed"
        };
        assert_eq!(rec.stop(), Err(want.to_string()));
    }
}

/// A recording whose source installs a counting ring and waits for
/// stop; returned once the ring is in.
fn rung() -> (ActiveRecording, Arc<AtomicUsize>) {
    let rings = Arc::new(AtomicUsize::new(0));
    let (count, (ready, installed)) = (rings.clone(), mpsc::channel());
    let rec = ActiveRecording::spawn(
        PathBuf::from("rung"),
        30,
        true,
        RecordingFormat::Mp4,
        RecordingEncoder::Auto,
        move |spec| {
            spec.bell.install(move || {
                count.fetch_add(1, Ordering::SeqCst);
            });
            ready.send(()).unwrap();
            let _ = spec.stop.recv();
            Ok(spec.output)
        },
        || {},
    )
    .unwrap();
    installed.recv_timeout(Duration::from_secs(5)).unwrap();
    (rec, rings)
}

/// A source blocked on its own loop reads its channels only when rung,
/// so each send rings once: a control, the stop by every path, and
/// nothing more after it.
#[test]
fn every_send_rings_the_source_once() {
    let n = |rings: &AtomicUsize| rings.load(Ordering::SeqCst);
    let (mut rec, rings) = rung();
    rec.toggle_pause();
    assert_eq!(n(&rings), 1, "pause");
    rec.toggle_pause();
    assert_eq!(n(&rings), 2, "resume");
    rec.toggle_mic().unwrap();
    assert_eq!(n(&rings), 3, "mic");
    rec.stop().unwrap();
    assert_eq!(n(&rings), 4, "stop, then the drop sends nothing");

    let (rec, rings) = rung();
    let (done, result) = mpsc::channel();
    let flush = rec.stop_async(move |r| done.send(r).unwrap());
    // The handle joins the thread that runs `done`: once it joins, the
    // result is there without a wait.
    flush.join().unwrap();
    result.try_recv().unwrap().unwrap();
    assert_eq!(n(&rings), 1, "stop_async");

    let (rec, rings) = rung();
    drop(rec);
    assert_eq!(n(&rings), 1, "drop");
}
