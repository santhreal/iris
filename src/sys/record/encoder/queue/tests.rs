// WHY: the class closed here is "a captured frame reaches ffmpeg late,
// out of order, or never": a writer that sleeps while a frame waits in
// the slot (a source gone still never sends another), the slot's frame
// overtaking an older queued one, a newer frame lost to an older one in
// the slot, a displaced buffer that does not come back for reuse, a
// gone writer reported as a queued frame, a tail written ahead of a
// frame or not at all, and a hang-up that leaves the writer asleep.
// The concurrent run checks order and delivery under real scheduling;
// it cannot force the one interleaving an unlocked empty check loses.
// Not covered: the bytes written (the recorder tests decode real
// segments).

use std::sync::mpsc::{channel, sync_channel, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use super::*;

const LONG: Duration = Duration::from_secs(5);

/// A queue `depth` frames deep: the encoder's end, the slot, and the
/// writer's end.
fn queue(depth: usize) -> (SyncSender<Stamped>, Slot, Queued) {
    let (tx, rx) = sync_channel(depth);
    let slot = Slot::default();
    let queued = Queued {
        rx,
        slot: slot.clone(),
    };
    (tx, slot, queued)
}

/// A frame stamped `ts` whose bytes hold the stamp.
fn frame(ts: u64) -> Stamped {
    (ts.to_le_bytes().to_vec(), ts)
}

/// A message as the tests compare it.
#[derive(Debug, PartialEq)]
enum Out {
    Frame(u64),
    Tail(u64),
}

/// `msg` as an `Out`, a frame checked against its bytes.
fn out(msg: Msg) -> Out {
    match msg {
        Msg::Frame(buf, ts) => {
            assert_eq!(
                buf,
                ts.to_le_bytes(),
                "frame {ts} holds another frame's bytes"
            );
            Out::Frame(ts)
        }
        Msg::Tail(ts) => Out::Tail(ts),
    }
}

/// The stamp of a frame message, checked against its bytes.
fn stamp(msg: Msg) -> u64 {
    match out(msg) {
        Out::Frame(ts) => ts,
        Out::Tail(ts) => panic!("a tail at {ts} where a frame was due"),
    }
}

/// Offer frames `from..=to`, none of which may displace another.
fn fill(tx: &SyncSender<Stamped>, slot: &Slot, from: u64, to: u64) {
    for ts in from..=to {
        assert_eq!(offer(tx, slot, frame(ts)), Ok(None), "frame {ts}");
    }
}

/// Everything left for the writer once the encoder hung up with `tail`.
fn rest(tx: SyncSender<Stamped>, slot: &Slot, queued: &Queued, tail: Option<u64>) -> Vec<Out> {
    hang_up(tx, slot, tail);
    std::iter::from_fn(|| queued.next()).map(out).collect()
}

/// The writer on its own thread, forwarding every message. A writer
/// that sleeps while a frame waits shows as a timeout, not a hang.
fn writer(queued: Queued) -> Receiver<Msg> {
    let (seen, rx) = channel();
    thread::spawn(move || {
        while let Some(msg) = queued.next() {
            if seen.send(msg).is_err() {
                break;
            }
        }
    });
    rx
}

#[test]
fn a_frame_the_full_queue_refused_goes_out_while_the_source_is_still() {
    let (tx, slot, queued) = queue(2);
    // 1 and 2 fill the queue; 3 waits in the slot.
    fill(&tx, &slot, 1, 3);
    // Nothing more is offered and the queue stays open, as with a
    // source that went still: the writer takes 3 by itself.
    let seen = writer(queued);
    let got: Vec<u64> = (1..=3)
        .map(|n| match seen.recv_timeout(LONG) {
            Ok(msg) => stamp(msg),
            Err(e) => panic!("frame {n} never reached the writer: {e}"),
        })
        .collect();
    assert_eq!(got, [1, 2, 3]);
    drop(tx);
}

#[test]
fn a_newer_frame_replaces_the_waiting_one() {
    let (tx, slot, queued) = queue(2);
    fill(&tx, &slot, 1, 3);
    // 4 takes 3's place in the slot; 3's buffer comes back for reuse.
    assert_eq!(offer(&tx, &slot, frame(4)), Ok(Some(frame(3).0)));
    assert_eq!(
        rest(tx, &slot, &queued, None),
        [Out::Frame(1), Out::Frame(2), Out::Frame(4)]
    );
}

#[test]
fn the_waiting_frame_goes_out_ahead_of_a_newer_one() {
    let (tx, slot, queued) = queue(2);
    fill(&tx, &slot, 1, 3);
    // The writer frees one place: 3 moves from the slot into it, and 4
    // waits in the slot behind it.
    assert_eq!(queued.next().map(stamp), Some(1));
    fill(&tx, &slot, 4, 4);
    assert_eq!(
        rest(tx, &slot, &queued, None),
        [Out::Frame(2), Out::Frame(3), Out::Frame(4)]
    );
}

#[test]
fn a_gone_writer_is_an_error_for_a_full_queue_too() {
    for queued_first in [0, 2] {
        let (tx, slot, queued) = queue(2);
        fill(&tx, &slot, 1, queued_first);
        drop(queued);
        assert_eq!(
            offer(&tx, &slot, frame(9)),
            Err(Gone),
            "{queued_first} queued"
        );
    }
}

#[test]
fn the_tail_goes_out_last_behind_the_waiting_frame() {
    let (tx, slot, queued) = queue(2);
    // 1 and 2 queued, 3 waiting in the slot.
    fill(&tx, &slot, 1, 3);
    let got = rest(tx, &slot, &queued, Some(9));
    let want = [Out::Frame(1), Out::Frame(2), Out::Frame(3), Out::Tail(9)];
    assert_eq!(got, want);
    // Ended: a further read neither blocks nor repeats the tail.
    assert!(queued.next().is_none());
}

#[test]
fn a_hang_up_wakes_a_writer_asleep_on_an_empty_queue() {
    for tail in [Some(5), None] {
        let (tx, slot, queued) = queue(2);
        let seen = writer(queued);
        // Let the writer take 1 and go to sleep on the empty queue.
        fill(&tx, &slot, 1, 1);
        assert_eq!(seen.recv_timeout(LONG).map(out), Ok(Out::Frame(1)));
        hang_up(tx, &slot, tail);
        if let Some(ts) = tail {
            assert_eq!(seen.recv_timeout(LONG).map(out), Ok(Out::Tail(ts)));
        }
        // The writer ends: its sender drops, and the wait disconnects
        // instead of timing out.
        assert!(matches!(
            seen.recv_timeout(LONG),
            Err(RecvTimeoutError::Disconnected)
        ));
    }
}

#[test]
fn frames_reach_the_writer_in_order_and_the_last_one_always() {
    const N: u64 = 20_000;
    let (tx, slot, queued) = queue(2);
    let (seen_tx, seen) = channel();
    thread::spawn(move || {
        while let Some(msg) = queued.next() {
            let ts = stamp(msg);
            // Slower than the source now and then, so frames wait in
            // the slot and newer ones replace them.
            if ts.is_multiple_of(7) {
                thread::sleep(Duration::from_micros(20));
            }
            if seen_tx.send(ts).is_err() {
                break;
            }
        }
    });
    let mut displaced = Vec::new();
    for ts in 1..=N {
        if let Some(buf) = offer(&tx, &slot, frame(ts)).unwrap() {
            displaced.push(u64::from_le_bytes(buf.try_into().unwrap()));
        }
    }
    // The queue stays open: the last frame goes out on its own.
    let mut got = Vec::new();
    while got.last() != Some(&N) {
        got.push(
            seen.recv_timeout(LONG)
                .expect("the last frame never reached the writer"),
        );
    }
    drop(tx);
    if let Some(w) = got.windows(2).find(|w| w[0] >= w[1]) {
        panic!("frame {} reached the writer after frame {}", w[1], w[0]);
    }
    assert!(!displaced.is_empty(), "the source never outran the writer");
    let mut all: Vec<u64> = got.iter().chain(&displaced).copied().collect();
    all.sort_unstable();
    assert_eq!(
        all,
        (1..=N).collect::<Vec<_>>(),
        "each frame is written or comes back once"
    );
}
