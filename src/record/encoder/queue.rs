//! The frame queue between a segment's capture thread and its writer
//! thread. Frames queue in capture order up to a fixed depth. When the
//! queue is full the newest frame waits in one slot, replaced by any
//! newer frame, and the writer takes it once the queue is empty: the
//! last change before a source goes still reaches ffmpeg without the
//! source calling back. The segment's end hangs the queue up with a
//! tail, which the writer writes after every frame. Neither side waits
//! on a timer.
//!
//! One lock orders the two sides. `offer` holds it across its sends and
//! the slot store; the writer holds it across its empty check and the
//! slot take. The writer therefore never takes the slot's frame ahead
//! of an older queued one, and never sleeps on an empty queue while a
//! frame waits: a frame enters the slot only when the queue is full,
//! and a full queue does not sleep the writer.

use std::io::{self, IoSlice, Write};
use std::process::ChildStdin;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;

use parking_lot::Mutex;

use crate::record::mkv;

/// What the writer writes next.
pub(super) enum Msg {
    /// A frame and its timestamp, ms since the epoch.
    Frame(Vec<u8>, u64),
    /// Write the last frame again at this timestamp: the segment ends
    /// here, and the picture on screen lasted until now.
    Tail(u64),
}

/// A frame and its timestamp, ms since the epoch.
pub(super) type Stamped = (Vec<u8>, u64);

/// What waits outside the queue.
#[derive(Default)]
pub(super) struct Pending {
    /// The newest frame the full queue could not take. It is newer than
    /// every queued frame.
    frame: Option<Stamped>,
    /// Set at the hang-up; written after every frame.
    tail: Option<u64>,
}

pub(super) type Slot = Arc<Mutex<Pending>>;

/// The writer is gone: it failed, or the segment was finished.
#[derive(Debug, PartialEq)]
pub(super) struct Gone;

/// Queue `frame` behind the frame waiting in the slot, if any. What the
/// full queue cannot take waits in the slot, and returns the waiting
/// frame it replaced, for reuse.
pub(super) fn offer(
    tx: &SyncSender<Stamped>,
    slot: &Mutex<Pending>,
    frame: Stamped,
) -> Result<Option<Vec<u8>>, Gone> {
    let mut pending = slot.lock();
    let mut full = false;
    let mut displaced = None;
    // Oldest first. After one send fails every later frame goes to the
    // slot: a queued frame is never newer than the waiting one.
    for stamped in pending.frame.take().into_iter().chain([frame]) {
        if !full {
            match tx.try_send(stamped) {
                Ok(()) => continue,
                Err(TrySendError::Full(stamped)) => {
                    full = true;
                    pending.frame = Some(stamped);
                    continue;
                }
                Err(TrySendError::Disconnected(_)) => return Err(Gone),
            }
        }
        displaced = pending.frame.replace(stamped).map(|(old, _)| old);
    }
    Ok(displaced)
}

/// End the queue: the writer writes what is queued, then the waiting
/// frame, then the last frame again at `tail`, and exits.
pub(super) fn hang_up(tx: SyncSender<Stamped>, slot: &Mutex<Pending>, tail: Option<u64>) {
    // Stored before the hang-up: the writer reads it after.
    slot.lock().tail = tail;
    drop(tx);
}

/// The writer's side: queued frames in order, then the slot's frame
/// once the queue is empty, and the tail once the queue is hung up.
pub(super) struct Queued {
    pub(super) rx: Receiver<Stamped>,
    pub(super) slot: Slot,
}

impl Queued {
    /// The next message, waiting for one. None once the queue is hung
    /// up and nothing is left.
    pub(super) fn next(&self) -> Option<Msg> {
        loop {
            {
                let mut pending = self.slot.lock();
                let hung_up = match self.rx.try_recv() {
                    Ok((frame, ts)) => return Some(Msg::Frame(frame, ts)),
                    Err(e) => e == TryRecvError::Disconnected,
                };
                if let Some((frame, ts)) = pending.frame.take() {
                    return Some(Msg::Frame(frame, ts));
                }
                if hung_up {
                    return pending.tail.take().map(Msg::Tail);
                }
            }
            // Empty, and no frame waits: sleep until a frame or the
            // hang-up.
            if let Ok((frame, ts)) = self.rx.recv() {
                return Some(Msg::Frame(frame, ts));
            }
        }
    }
}

/// The writer thread: header, then every frame behind its cluster
/// head. `last` keeps the newest frame for tails and handoff.
pub(super) fn write_stream(
    mut out: ChildStdin,
    header: &[u8],
    queued: Queued,
    recycle: &Sender<Vec<u8>>,
    last: &mut Option<Vec<u8>>,
) -> io::Result<()> {
    out.write_all(header)?;
    let mut head = Vec::with_capacity(40);
    while let Some(msg) = queued.next() {
        match msg {
            Msg::Frame(frame, ts) => {
                mkv::cluster_head(ts, frame.len(), &mut head);
                write_both(&mut out, &head, &frame)?;
                if let Some(mut old) = last.replace(frame) {
                    old.clear();
                    let _ = recycle.send(old);
                }
            }
            Msg::Tail(ts) => {
                if let Some(frame) = last.as_deref() {
                    mkv::cluster_head(ts, frame.len(), &mut head);
                    write_both(&mut out, &head, frame)?;
                }
            }
        }
    }
    out.flush()
}

/// `head` then `body` with vectored writes: no copy into a staging
/// buffer, one syscall per pipe-full.
fn write_both(out: &mut impl Write, head: &[u8], body: &[u8]) -> io::Result<()> {
    let mut slices = [IoSlice::new(head), IoSlice::new(body)];
    let mut bufs = &mut slices[..];
    while !bufs.is_empty() {
        match out.write_vectored(bufs) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => IoSlice::advance_slices(&mut bufs, n),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
