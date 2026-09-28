// WHY: two classes close here.
//
// "A segment is judged by the wrong line of ffmpeg's log": a progress
// report taken for an error (which sends a clean run down the slow exit
// path and into the error text), an error taken for a report (which
// ships a segment ffmpeg failed), a report split across reads or ended
// by CRLF that is missed, or a long log that grows without bound. Every
// log runs whole and one byte per read.
//
// "A segment goes out at the wrong time, or its ffmpeg is never
// reaped": each `settle` case runs a real child, this test binary acting
// as ffmpeg, that lingers after its log, and checks the result, whether
// the segment went out before the child exited, and that the child is
// reaped within the deadline. One child holds its segment as ffmpeg
// before 6.0 does; that segment goes out early only where a held file
// can be deleted. Not covered: an ffmpeg that writes to its output
// after the report; 4.4 and 6.0 write nothing to it after the report.

use std::io::Write;
use std::process::{Command, Stdio};

use super::*;

/// Yields one byte per read, as a pipe can.
struct Trickle<'a>(&'a [u8]);

impl Read for Trickle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some((first, rest)) = self.0.split_first() else {
            return Ok(0);
        };
        buf[0] = *first;
        self.0 = rest;
        Ok(1)
    }
}

/// What `read_log` sends at the end, and what it returns.
fn read_both(input: &str) -> [(Option<bool>, String); 2] {
    let run = |reader: &mut dyn Read| {
        let (tx, rx) = sync_channel(1);
        let text = read_log(reader, &tx);
        drop(tx);
        (rx.recv().ok(), text)
    };
    [
        run(&mut input.as_bytes()),
        run(&mut Trickle(input.as_bytes())),
    ]
}

const REPORT: &str = "frame=12\nfps=0.00\nstream_0_0_q=-1.0\nbitrate=  12.3kbits/s\n\
                      total_size=4096\nout_time_us=400000\nout_time=00:00:00.400000\n\
                      dup_frames=0\ndrop_frames=0\nspeed=   1x\n";

#[test]
fn each_log_ends_as_its_lines_say() {
    let end = format!("{REPORT}progress=end\n");
    let cases: [(&str, String, Option<bool>, &str); 8] = [
        (
            "reports only",
            format!("{REPORT}progress=continue\n{end}"),
            Some(true),
            "",
        ),
        ("CRLF reports", end.replace('\n', "\r\n"), Some(true), ""),
        (
            "an error before the end",
            format!("{REPORT}[h264_nvenc @ 0x55d0] EncodePicture failed\n{end}"),
            Some(false),
            "[h264_nvenc @ 0x55d0] EncodePicture failed",
        ),
        (
            "an error with an = in it",
            format!("Error: rate=0 is invalid\n{end}"),
            Some(false),
            "Error: rate=0 is invalid",
        ),
        (
            "an error after the end",
            format!("{end}[AVHWDeviceContext @ 0x1] cuCtxDestroy failed\n"),
            Some(true),
            "[AVHWDeviceContext @ 0x1] cuCtxDestroy failed",
        ),
        (
            "no end",
            format!("{REPORT}progress=continue\nConversion failed!\n"),
            None,
            "Conversion failed!",
        ),
        ("nothing", String::new(), None, ""),
        (
            "an end without a newline",
            "progress=end".to_string(),
            Some(true),
            "",
        ),
    ];
    for (case, input, want_end, want_text) in cases {
        for (how, (sent, text)) in ["whole", "trickled"].iter().zip(read_both(&input)) {
            assert_eq!(sent, want_end, "{case}, {how}");
            assert_eq!(text, want_text, "{case}, {how}");
        }
    }
}

#[test]
fn a_long_log_keeps_its_tail() {
    let mut text = "x".repeat(40 << 10);
    text.push('\n');
    for n in 0..2000 {
        text.push_str(&format!("[enc] line {n}\n"));
    }
    text.push_str("ffmpeg: the real error\n");
    for (sent, tail) in read_both(&text) {
        assert!(
            tail.ends_with("ffmpeg: the real error"),
            "{}",
            &tail[tail.len() - 40..]
        );
        assert!(tail.len() <= 16 << 10, "{} bytes kept", tail.len());
        assert_eq!(sent, None);
    }
}

/// Makes `fake_ffmpeg` act, as `<linger ms> <exit code>`.
const FAKE: &str = "IRIS_TEST_FAKE_FFMPEG";
/// What `fake_ffmpeg` writes on stderr.
const FAKE_LOG: &str = "IRIS_TEST_FAKE_FFMPEG_LOG";
/// A segment `fake_ffmpeg` holds open until it exits.
const FAKE_HOLD: &str = "IRIS_TEST_FAKE_FFMPEG_HOLD";

/// The ffmpeg of the `settle` cases, run in a child of this test
/// binary: it holds its segment when told to, writes its log, lingers,
/// and exits. Run as a test of its own, without `FAKE`, it returns at
/// once.
#[test]
fn fake_ffmpeg() {
    let Ok(plan) = std::env::var(FAKE) else {
        return;
    };
    let (linger, code) = plan.split_once(' ').expect("<linger ms> <exit code>");
    let _held = std::env::var_os(FAKE_HOLD)
        .map(|p| crate::sys::record::hold(std::path::Path::new(&p)).expect("hold the segment"));
    let mut err = std::io::stderr();
    err.write_all(std::env::var(FAKE_LOG).unwrap_or_default().as_bytes())
        .and_then(|()| err.flush())
        .expect("write the log");
    std::thread::sleep(Duration::from_millis(linger.parse().expect("linger")));
    std::process::exit(code.parse().expect("exit code"));
}

/// One `settle` case: what the fake ffmpeg does, and what must follow.
struct Case {
    name: &'static str,
    log: String,
    linger: Duration,
    code: i32,
    fed: Result<(), String>,
    /// What the segment file holds when ffmpeg reports.
    file: &'static [u8],
    /// ffmpeg holds the segment open until it exits.
    hold: bool,
    /// The segment went out before ffmpeg exited.
    early: bool,
    /// `Ok` for the segment, or text its error contains.
    want: Result<(), &'static str>,
}

/// How long each case's ffmpeg may run before `settle` kills it.
const LIMIT: Duration = Duration::from_secs(3);
/// How long a lingering fake ffmpeg outlives its log.
const LINGER: Duration = Duration::from_millis(1500);
/// A fake ffmpeg that never exits by itself.
const HANG: Duration = Duration::from_secs(60);

/// Whether a file ffmpeg holds open can be deleted here, where the
/// join deletes its segments.
fn held_is_deletable(dir: &std::path::Path) -> bool {
    let probe = dir.join("probe.mkv");
    std::fs::write(&probe, b"segment").expect("write probe");
    let _held = crate::sys::record::hold(&probe).expect("hold probe");
    std::fs::remove_file(&probe).is_ok()
}

fn cases(dir: &std::path::Path) -> Vec<Case> {
    let end = format!("{REPORT}progress=end\n");
    let err = "[h264_nvenc @ 0x55d0] EncodePicture failed\n";
    let case = |name, log: &str, linger, code, early, want| Case {
        name,
        log: log.to_string(),
        linger,
        code,
        fed: Ok(()),
        file: b"segment",
        hold: false,
        early,
        want,
    };
    vec![
        case("a clean report", &end, LINGER, 0, true, Ok(())),
        case(
            "a failed exit after a clean report",
            &end,
            LINGER,
            1,
            true,
            Ok(()),
        ),
        case(
            "an error before the report, then a clean exit",
            &format!("{err}{end}"),
            LINGER,
            0,
            false,
            Ok(()),
        ),
        case(
            "an error before the report, then a failed exit",
            &format!("{err}{end}"),
            LINGER,
            1,
            false,
            Err("EncodePicture failed"),
        ),
        case(
            "a failed exit and no report",
            "Conversion failed!\n",
            LINGER,
            1,
            false,
            Err("Conversion failed!"),
        ),
        case("a clean exit and no report", "", LINGER, 0, false, Ok(())),
        case("a hang after a clean report", &end, HANG, 0, true, Ok(())),
        case(
            "a hang and no report",
            REPORT,
            HANG,
            0,
            false,
            Err("did not finish the segment in time; killed"),
        ),
        Case {
            fed: Err("write frame to ffmpeg: broken pipe".to_string()),
            want: Err("write frame to ffmpeg: broken pipe"),
            ..case(
                "a failed feed and a clean report",
                &end,
                LINGER,
                0,
                false,
                Ok(()),
            )
        },
        Case {
            file: b"",
            want: Err("is empty"),
            ..case(
                "an empty segment at a clean report",
                &end,
                LINGER,
                0,
                true,
                Ok(()),
            )
        },
        Case {
            hold: true,
            ..case(
                "a clean report while ffmpeg holds the segment",
                &end,
                LINGER,
                0,
                held_is_deletable(dir),
                Ok(()),
            )
        },
    ]
}

/// Start the fake ffmpeg of `case`, writing `segment`.
fn spawn(case: &Case, segment: &std::path::Path) -> (Child, Stderr) {
    #[allow(clippy::disallowed_methods)] // this test binary, which no upgrade replaces
    let exe = std::env::current_exe().expect("test binary path");
    let mut fake = Command::new(exe);
    fake.args([
        "--exact",
        "record::child::tests::fake_ffmpeg",
        "--nocapture",
    ])
    .env(FAKE, format!("{} {}", case.linger.as_millis(), case.code))
    .env(FAKE_LOG, &case.log)
    .env_remove(FAKE_HOLD)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    if case.hold {
        fake.env(FAKE_HOLD, segment);
    }
    let mut child = fake.spawn().expect("spawn fake ffmpeg");
    let stderr = Stderr::read(child.stderr.take().expect("piped stderr")).expect("read stderr");
    (child, stderr)
}

/// Run `case` through `settle` and check what follows. `Err` names what
/// went wrong.
fn check(case: Case, dir: &std::path::Path) -> Result<(), String> {
    let path = dir.join(format!("{}.mkv", case.name.replace(' ', "-")));
    std::fs::write(&path, case.file).map_err(|e| e.to_string())?;
    let segment = Segment { path, audio: false };
    let (mut child, stderr) = spawn(&case, &segment.path);
    let start = Instant::now();
    let (tx, settled) = std::sync::mpsc::channel();
    let (fed, want_segment) = (case.fed.clone(), segment.clone());
    let closing = Closing::spawn(move |done| {
        settle(&mut child, Some(stderr), fed, segment, start + LIMIT, done);
        let _ = tx.send((Instant::now(), child.try_wait()));
    });
    let got = closing.wait();
    let sent = Instant::now();
    let (settled, reaped) = settled
        .recv_timeout(LIMIT + Duration::from_secs(5))
        .map_err(|_| "settle never returned".to_string())?;
    match (&got, case.want) {
        (Ok(s), Ok(())) if *s == want_segment => {}
        (Err(e), Err(part)) if e.contains(part) => {}
        (got, want) => return Err(format!("got {got:?}, want {want:?}")),
    }
    // The segment went out early when ffmpeg exited well after it.
    let early = settled.saturating_duration_since(sent) > LINGER / 2;
    if early != case.early {
        return Err(format!(
            "segment out {:?} before settle returned; early: {early}",
            settled.saturating_duration_since(sent)
        ));
    }
    if !matches!(reaped, Ok(Some(_))) {
        return Err(format!("child not reaped after settle: {reaped:?}"));
    }
    let took = settled - start;
    if took > LIMIT + Duration::from_secs(1) {
        return Err(format!("settle took {took:?}, past its deadline"));
    }
    Ok(())
}

#[test]
fn a_segment_goes_out_when_ffmpeg_has_written_it() {
    let dir = tempfile::tempdir().unwrap();
    let failures: Vec<String> = std::thread::scope(|s| {
        let runs: Vec<_> = cases(dir.path())
            .into_iter()
            .map(|case| {
                let name = case.name;
                (name, s.spawn(|| check(case, dir.path())))
            })
            .collect();
        runs.into_iter()
            .filter_map(|(name, run)| match run.join() {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(format!("{name}: {e}")),
                Err(_) => Some(format!("{name}: panicked")),
            })
            .collect()
    });
    assert!(failures.is_empty(), "{failures:#?}");
}
