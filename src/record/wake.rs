//! A Linux source's end of a doorbell: a datagram socket pair. A ring
//! sends one byte on it, which wakes the source's poll(2) or PipeWire
//! loop; the source drains the bytes before it reads its channels.

use std::os::fd::{AsRawFd, BorrowedFd, RawFd};
use std::os::unix::net::UnixDatagram;
use std::time::Duration;

use super::doorbell::Doorbell;

pub(crate) struct Wake(UnixDatagram);

impl Wake {
    /// A wake that `bell` rings from now on.
    pub(crate) fn install(bell: &Doorbell) -> Result<Self, String> {
        let err = |e: std::io::Error| format!("recording wake socket: {e}");
        let (rx, tx) = UnixDatagram::pair().map_err(err)?;
        rx.set_nonblocking(true).map_err(err)?;
        tx.set_nonblocking(true).map_err(err)?;
        // A full queue already holds a wake, and a closed peer is a
        // source that ended: neither needs the byte.
        bell.install(move || {
            let _ = tx.send(&[1]);
        });
        Ok(Self(rx))
    }

    /// Consume every ring queued so far.
    pub(crate) fn drain(&self) {
        let mut byte = [0u8; 1];
        while self.0.recv(&mut byte).is_ok() {}
    }

    /// Block until a ring arrives, `input` is readable, or `timeout`
    /// passes; None waits for one of the first two. False on the
    /// timeout. Input already read into a user-space buffer does not
    /// wake the poll: drain it first.
    pub(crate) fn wait(&self, input: Option<BorrowedFd<'_>>, timeout: Option<Duration>) -> bool {
        let poll = |fd: RawFd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // poll(2) skips a negative descriptor.
        let input = input.map_or(-1, |fd| fd.as_raw_fd());
        let mut fds = [poll(self.0.as_raw_fd()), poll(input)];
        // Rounded up: a sub-millisecond remainder must sleep, not spin
        // on zero-timeout polls until it passes.
        let ms = timeout.map_or(-1, |t| {
            t.as_nanos().div_ceil(1_000_000).min(i32::MAX as u128) as libc::c_int
        });
        // SAFETY: two pollfds over descriptors borrowed for the call.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) != 0 }
    }
}

impl AsRawFd for Wake {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

// WHY: the class closed here is "a blocked source misses its wake or
// wakes for nothing": a ring that does not end the wait, rings that
// keep waking it after a drain, input on the source's own descriptor
// that does not wake it, or a ring after the source ended that fails
// loudly. Each wait is bounded, so a lost wake fails instead of
// hanging. Not covered: the sources' loops (see the x11 tests).
#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::fd::AsFd;
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    use super::*;

    const LONG: Duration = Duration::from_secs(5);
    const SHORT: Duration = Duration::from_millis(30);

    fn installed() -> (Doorbell, Wake, UnixStream, UnixStream) {
        let bell = Doorbell::default();
        let wake = Wake::install(&bell).unwrap();
        let (input, peer) = UnixStream::pair().unwrap();
        (bell, wake, input, peer)
    }

    #[test]
    fn a_ring_from_another_thread_ends_an_untimed_wait() {
        let (bell, wake, input, _peer) = installed();
        let t0 = Instant::now();
        let ringer = std::thread::spawn(move || {
            std::thread::sleep(SHORT);
            bell.ring();
            bell
        });
        // Untimed would hang on a lost ring: bound it at LONG and
        // require the ring, not the bound, to end it.
        assert!(wake.wait(Some(input.as_fd()), Some(LONG)));
        assert!(t0.elapsed() < LONG / 2, "{:?}", t0.elapsed());
        // With no input to watch, the ring alone ends the wait.
        let bell = ringer.join().unwrap();
        wake.drain();
        let t0 = Instant::now();
        let ringer = std::thread::spawn(move || {
            std::thread::sleep(SHORT);
            bell.ring();
        });
        assert!(wake.wait(None, Some(LONG)));
        assert!(t0.elapsed() < LONG / 2, "{:?}", t0.elapsed());
        ringer.join().unwrap();
    }

    #[test]
    fn input_on_the_source_descriptor_ends_the_wait() {
        let (_bell, wake, input, mut peer) = installed();
        peer.write_all(b"x").unwrap();
        assert!(wake.wait(Some(input.as_fd()), Some(LONG)));
    }

    #[test]
    fn drained_rings_do_not_wake_again() {
        let (bell, wake, input, _peer) = installed();
        for _ in 0..3 {
            bell.ring();
        }
        assert!(wake.wait(Some(input.as_fd()), Some(SHORT)));
        wake.drain();
        for input in [Some(input.as_fd()), None] {
            let t0 = Instant::now();
            assert!(!wake.wait(input, Some(SHORT)));
            assert!(t0.elapsed() >= SHORT);
        }
    }

    #[test]
    fn a_sub_millisecond_timeout_sleeps_it_out() {
        // A timeout truncated to 0 ms returns at once, and the X loop
        // spins on it until its frame is due.
        let (_bell, wake, _input, _peer) = installed();
        for short in [Duration::from_micros(300), Duration::from_micros(1300)] {
            let t0 = Instant::now();
            assert!(!wake.wait(None, Some(short)));
            assert!(t0.elapsed() >= short, "{short:?}: {:?}", t0.elapsed());
        }
    }

    #[test]
    fn a_ring_after_the_source_ended_is_harmless() {
        let (bell, wake, _input, _peer) = installed();
        drop(wake);
        bell.ring();
        bell.ring();
    }
}
