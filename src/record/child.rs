//! The ffmpeg child of one recording segment: stderr read as it
//! arrives, a wait that ends at a deadline, and the thread a segment
//! finishes on.

use std::io::Read;
use std::process::{Child, ChildStderr, ExitStatus};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::join::Segment;

/// A segment on its way out, finishing on its own thread.
pub struct Closing {
    done: Result<JoinHandle<Result<Segment, String>>, String>,
}

impl Closing {
    /// Run `finish` on its own thread: ffmpeg flushes while the
    /// recording goes on.
    pub fn spawn(finish: impl FnOnce() -> Result<Segment, String> + Send + 'static) -> Self {
        let done = std::thread::Builder::new()
            .name("iris-rec-finish".into())
            .spawn(finish)
            .map_err(|e| format!("spawn segment finisher: {e}"));
        Self { done }
    }

    /// Block until ffmpeg has written the segment.
    pub fn wait(self) -> Result<Segment, String> {
        self.done?
            .join()
            .unwrap_or_else(|_| Err("segment finisher panicked".to_string()))
    }
}

/// `segment` once its file holds something: ffmpeg can exit cleanly
/// having written nothing.
pub fn written(segment: Segment) -> Result<Segment, String> {
    let len = std::fs::metadata(&segment.path)
        .map_err(|e| format!("segment {} missing: {e}", segment.path.display()))?
        .len();
    if len == 0 {
        return Err(format!("segment {} is empty", segment.path.display()));
    }
    Ok(segment)
}

/// Read `err` on its own thread, so a chatty child never blocks on a
/// full pipe. The handle yields the last 8KB, trimmed.
pub fn drain(err: ChildStderr) -> std::io::Result<JoinHandle<String>> {
    std::thread::Builder::new()
        .name("iris-rec-stderr".into())
        .spawn(move || read_tail(err))
}

fn read_tail(mut err: impl Read) -> String {
    let mut text = Vec::new();
    let mut chunk = [0u8; 4096];
    while let Ok(n @ 1..) = err.read(&mut chunk) {
        text.extend_from_slice(&chunk[..n]);
        if text.len() > 16 << 10 {
            text.drain(..text.len() - (8 << 10));
        }
    }
    String::from_utf8_lossy(&text).trim().to_string()
}

/// Wait for `child` until `deadline`, then kill it.
pub fn wait_until(child: &mut Child, deadline: Instant) -> Result<ExitStatus, String> {
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Ok(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("ffmpeg did not finish the segment in time; killed".to_string());
            }
            Err(e) => return Err(format!("wait on ffmpeg: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_stream_keeps_its_tail() {
        let mut text = "x".repeat(40 << 10);
        text.push_str("\nffmpeg: the real error\n");
        let tail = read_tail(text.as_bytes());
        assert!(
            tail.ends_with("ffmpeg: the real error"),
            "{}",
            &tail[tail.len() - 40..]
        );
        assert!(tail.len() <= 16 << 10, "{} bytes kept", tail.len());
    }
}
