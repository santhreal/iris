// WHY: the class closed here is "a recording loop that polls, or sleeps
// through its wake": a still or paused source that wakes the loop on a
// timer, a stop or a control that waits out a timeout, X input that
// does not wake it or that wakes it through the rate limit, and frames
// grabbed past the rate or during a pause. X is a socket pair: a byte
// written to it stands in for a damage event, and `events` reads it.
// Frames go through a real Recorder, so this runs ffmpeg. Not covered:
// the X side (the damage watch, the pick, the border), which needs a
// server; the recording smoke test covers those.

use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use super::*;
use crate::config::{RecordingEncoder, RecordingFormat};
use crate::record::Doorbell;

/// Bound on every wait for the loop: a lost wake fails, never hangs.
const LONG: Duration = Duration::from_secs(10);
/// Long enough for a timer-driven loop to wake several times.
const QUIET: Duration = Duration::from_millis(300);

/// A live loop over a fake X connection, and what steers it.
struct Rig {
    /// A byte written here is a damage event.
    x: UnixStream,
    stop: Option<Sender<()>>,
    control: Sender<RecControl>,
    bell: Doorbell,
    /// How often the loop drained its events, and grabbed a frame.
    events: Arc<AtomicUsize>,
    grabs: Arc<AtomicUsize>,
    /// Whether the loop drained its events since its last grab. A loop
    /// is busy from a grab to the next drain, and the first grab starts
    /// the encoder, which can outlast any sampling interval.
    drained: Arc<AtomicBool>,
    done: Receiver<Result<PathBuf, String>>,
    dir: tempfile::TempDir,
}

/// A loop recording at `fps`, its stop already sent when `stopped`.
fn rig(fps: u32, stopped: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let (stop, stop_rx) = channel();
    let (control, control_rx) = channel();
    if stopped {
        stop.send(()).unwrap();
    }
    let bell = Doorbell::default();
    let spec = RecordingSpec {
        output: dir.path().join("r.mp4"),
        fps,
        mic: false,
        format: RecordingFormat::Mp4,
        encoder: RecordingEncoder::Libx264,
        stop: stop_rx,
        control: control_rx,
        bell: bell.clone(),
    };
    let (x, input) = UnixStream::pair().unwrap();
    input.set_nonblocking(true).unwrap();
    let events = Arc::new(AtomicUsize::new(0));
    let grabs = Arc::new(AtomicUsize::new(0));
    let drained = Arc::new(AtomicBool::new(false));
    let (seen, grabbed, settled) = (events.clone(), grabs.clone(), drained.clone());
    let (done_tx, done) = channel();
    thread::spawn(move || {
        let wake = Wake::install(&spec.bell).unwrap();
        let events = |_paused: bool| {
            seen.fetch_add(1, Ordering::SeqCst);
            let (mut buf, mut dirty) = ([0u8; 64], false);
            while let Ok(1..) = (&input).read(&mut buf) {
                dirty = true;
            }
            settled.store(true, Ordering::SeqCst);
            Some(((64, 48), dirty))
        };
        let grab = |w: u32, h: u32, buf: &mut Vec<u8>| {
            settled.store(false, Ordering::SeqCst);
            grabbed.fetch_add(1, Ordering::SeqCst);
            buf.clear();
            buf.resize(w as usize * h as usize * 4, 0x80);
            Ok(())
        };
        let result = record_loop_inner(&spec, input.as_fd(), &wake, PixFmt::Bgrx, events, grab);
        let _ = done_tx.send(result);
    });
    Rig {
        x,
        stop: Some(stop),
        control,
        bell,
        events,
        grabs,
        drained,
        done,
        dir,
    }
}

impl Rig {
    fn events(&self) -> usize {
        self.events.load(Ordering::SeqCst)
    }

    fn grabs(&self) -> usize {
        self.grabs.load(Ordering::SeqCst)
    }

    fn damage(&mut self) {
        self.x.write_all(&[1]).unwrap();
    }

    fn send(&self, ctl: RecControl) {
        self.control.send(ctl).unwrap();
        self.bell.ring();
    }

    /// Wait until `what` holds.
    fn until(&self, what: &str, f: impl Fn(&Self) -> bool) {
        let t0 = Instant::now();
        while !f(self) {
            assert!(t0.elapsed() < LONG, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Let the loop go to sleep, then check it stays asleep. Asleep is
    /// a drain after the last grab, then no drain or grab for a while.
    fn stays_asleep(&self, what: &str) {
        let t0 = Instant::now();
        let mut before = (self.events(), self.grabs());
        loop {
            thread::sleep(Duration::from_millis(50));
            let now = (self.events(), self.grabs());
            if now == before && self.drained.load(Ordering::SeqCst) {
                break;
            }
            assert!(t0.elapsed() < LONG, "{what}: the loop never went to sleep");
            before = now;
        }
        thread::sleep(QUIET);
        assert_eq!(
            (self.events(), self.grabs()),
            before,
            "{what} woke the loop"
        );
    }

    /// Ring the stop, or drop its sender when `drop_it`, and return
    /// the loop's result.
    fn stop(mut self, drop_it: bool) -> (Result<PathBuf, String>, tempfile::TempDir) {
        let stop = self.stop.take().unwrap();
        if drop_it {
            drop(stop);
        } else {
            stop.send(()).unwrap();
        }
        self.bell.ring();
        let result = self
            .done
            .recv_timeout(LONG)
            .expect("the loop outlived its stop");
        (result, self.dir)
    }
}

#[test]
fn a_still_source_sleeps_until_x_input_or_a_ring() {
    let mut rig = rig(30, false);
    rig.until("the first frame", |r| r.grabs() == 1);
    rig.stays_asleep("a still source");
    // A damage event wakes it, and the change is grabbed.
    rig.damage();
    rig.until("the damaged frame", |r| r.grabs() == 2);
    rig.stays_asleep("a still source");
    // A ring wakes it once, and there is nothing to grab.
    let events = rig.events();
    rig.bell.ring();
    rig.until("the ring", |r| r.events() == events + 1);
    rig.stays_asleep("a ring");
    assert_eq!(rig.grabs(), 2);
    let (result, _dir) = rig.stop(false);
    assert!(result.unwrap().exists());
}

#[test]
fn damage_faster_than_the_rate_grabs_at_the_rate_and_wakes_no_more() {
    let mut rig = rig(10, false);
    rig.until("the first frame", |r| r.grabs() == 1);
    let t0 = Instant::now();
    let mut sent = 0;
    while t0.elapsed() < Duration::from_millis(600) {
        rig.damage();
        sent += 1;
        thread::sleep(Duration::from_millis(2));
    }
    let (events, grabs) = (rig.events(), rig.grabs());
    // 600 ms at 10 fps: the first frame, then at most seven more.
    assert!((4..=8).contains(&grabs), "{grabs} grabs");
    // Damage during the rate limit waits for it: about two drains a
    // frame, not one a damage event.
    assert!(
        events <= 3 * grabs + 3,
        "{events} drains for {grabs} grabs, {sent} events"
    );
    let (result, _dir) = rig.stop(false);
    result.unwrap();
}

#[test]
fn a_paused_source_grabs_nothing_and_sleeps() {
    let mut rig = rig(30, false);
    rig.until("the first frame", |r| r.grabs() == 1);
    let events = rig.events();
    rig.send(RecControl::Pause);
    rig.until("the pause", |r| r.events() > events);
    rig.stays_asleep("a paused source");
    // Damage while paused is drained, never grabbed.
    let events = rig.events();
    rig.damage();
    rig.until("the damage drain", |r| r.events() > events);
    rig.stays_asleep("a paused source");
    assert_eq!(rig.grabs(), 1);
    // The resume grabs at once, with no damage.
    rig.send(RecControl::Resume);
    rig.until("the frame after the resume", |r| r.grabs() == 2);
    let (result, _dir) = rig.stop(false);
    assert!(result.unwrap().exists());
}

#[test]
fn a_stop_ends_a_sleeping_loop_by_send_or_drop() {
    for drop_it in [false, true] {
        let rig = rig(30, false);
        rig.until("the first frame", |r| r.grabs() == 1);
        rig.stays_asleep("a still source");
        let (result, _dir) = rig.stop(drop_it);
        let path = result.unwrap_or_else(|e| panic!("drop_it {drop_it}: {e}"));
        assert!(path.exists(), "drop_it {drop_it}");
    }
}

#[test]
fn a_stop_sent_before_the_loop_started_ends_it() {
    // Sent before the loop installed its wake, so nothing rang for it.
    let rig = rig(30, true);
    let result = rig
        .done
        .recv_timeout(LONG)
        .expect("the loop missed a stop sent before it started");
    let err = result.unwrap_err();
    assert!(err.starts_with(crate::record::CANCELLED_PREFIX), "{err}");
}
