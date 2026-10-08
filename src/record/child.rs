//! The ffmpeg child of one recording segment: stderr read as it
//! arrives, the report that ends the segment, a wait that ends at a
//! deadline, and the thread a segment finishes on.
//!
//! A segment is complete when ffmpeg reports its output written, not
//! when ffmpeg exits. With [`REPORT_END`], ffmpeg prints
//! `progress=end` on stderr after it writes and flushes every output
//! file; the rest of its run is teardown, and h264_nvenc spends 160 to
//! 300 ms of it releasing its CUDA context.

use std::io::Read;
use std::process::{Child, ExitStatus};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::time::{Duration, Instant};

use super::join::Segment;

/// The arguments that make ffmpeg report on stderr, as `progress=end`,
/// that its output is written.
pub const REPORT_END: [&str; 2] = ["-progress", "pipe:2"];

/// A segment on its way out, finishing on its own thread.
pub struct Closing {
    done: Result<Receiver<Result<Segment, String>>, String>,
}

impl Closing {
    /// Run `finish` on its own thread: ffmpeg flushes while the
    /// recording goes on. `finish` sends the segment on its [`Done`] as
    /// soon as the segment is known, and may wait out ffmpeg after.
    pub fn spawn(finish: impl FnOnce(Done) + Send + 'static) -> Self {
        let (tx, rx) = sync_channel(1);
        let done = std::thread::Builder::new()
            .name("iris-rec-finish".into())
            .spawn(move || finish(Done(tx)))
            .map(|_| rx)
            .map_err(|e| format!("spawn segment finisher: {e}"));
        Self { done }
    }

    /// Block until ffmpeg has written the segment.
    pub fn wait(self) -> Result<Segment, String> {
        self.done?
            .recv()
            .unwrap_or_else(|_| Err("segment finisher ended without a segment".to_string()))
    }
}

/// Where a finisher sends its segment, once.
pub struct Done(SyncSender<Result<Segment, String>>);

impl Done {
    pub fn send(self, segment: Result<Segment, String>) {
        // A recording that ended without waiting no longer listens.
        let _ = self.0.send(segment);
    }
}

/// Send the segment `child` writes on `done`, then wait out `child`.
///
/// When feeding ffmpeg succeeded (`fed`), ffmpeg reports its output
/// written with nothing logged before the report, and no process holds
/// the output open any more, the segment goes out at the report and a
/// failed exit after it is only logged. Otherwise the exit status
/// decides, with ffmpeg's log as the error. ffmpeg is killed at
/// `deadline`.
pub fn settle(
    child: &mut Child,
    stderr: Option<Stderr>,
    fed: Result<(), String>,
    segment: Segment,
    deadline: Instant,
    done: Done,
) {
    if fed.is_ok()
        && stderr.as_ref().is_some_and(|e| e.ended_clean(deadline))
        && crate::sys::record::released(&segment.path)
    {
        let path = segment.path.clone();
        done.send(written(segment));
        let status = wait_until(child, deadline);
        let text = stderr.map(Stderr::text).unwrap_or_default();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => crate::ilog!(
                "iris: record: ffmpeg exited {s} after writing {}: {text}",
                path.display()
            ),
            Err(e) => crate::ilog!("iris: record: after writing {}: {e}", path.display()),
        }
        return;
    }
    let status = wait_until(child, deadline);
    let text = stderr.map(Stderr::text).unwrap_or_default();
    done.send(match status {
        Err(e) => Err(e),
        Ok(s) if !s.success() => Err(format!("ffmpeg exited {s}: {text}")),
        Ok(_) => fed.and_then(|()| written(segment)),
    });
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

/// A child's stderr, read on its own thread so a chatty child never
/// blocks on a full pipe.
pub struct Stderr {
    /// Sends once, at `progress=end`: whether nothing else came first.
    end: Receiver<bool>,
    /// Sends once, when the child closes stderr: what it logged. The
    /// reading thread sends before it exits: a Windows thread's exit
    /// waits for the loader lock, which a DLL load on any other thread of
    /// the process holds.
    log: Receiver<String>,
}

impl Stderr {
    pub fn read(err: impl Read + Send + 'static) -> std::io::Result<Self> {
        let (tx, end) = sync_channel(1);
        let (sent, log) = sync_channel(1);
        std::thread::Builder::new()
            .name("iris-rec-stderr".into())
            .spawn(move || {
                // The receiver is gone only once its `Stderr` is dropped.
                let _ = sent.send(read_log(err, &tx));
            })?;
        Ok(Self { end, log })
    }

    /// Whether ffmpeg reports its output written by `deadline` with
    /// nothing logged before the report. False as soon as ffmpeg closes
    /// stderr without the report.
    fn ended_clean(&self, deadline: Instant) -> bool {
        let left = deadline.saturating_duration_since(Instant::now());
        self.end.recv_timeout(left) == Ok(true)
    }

    /// What ffmpeg logged, without its progress reports: the last 8KB,
    /// trimmed. Blocks until ffmpeg closes stderr, and not until the
    /// reading thread exits.
    pub fn text(self) -> String {
        self.log.recv().unwrap_or_default()
    }
}

/// A longer line is cut here: ffmpeg's lines are short, and the cut
/// keeps a runaway child's log bounded.
const LINE_MAX: usize = 4 << 10;

/// Read `err` to its end and return the tail of what it logged.
/// `progress=end` sends on `end` whether anything but progress report
/// lines came before it.
fn read_log(mut err: impl Read, end: &SyncSender<bool>) -> String {
    let mut log = Log::default();
    let mut line = Vec::new();
    let mut chunk = [0u8; 4096];
    while let Ok(n @ 1..) = err.read(&mut chunk) {
        for piece in chunk[..n].split_inclusive(|&b| b == b'\n') {
            let room = LINE_MAX.saturating_sub(line.len());
            line.extend_from_slice(&piece[..piece.len().min(room)]);
            if piece.ends_with(b"\n") {
                log.line(&line, end);
                line.clear();
            }
        }
    }
    log.line(&line, end);
    String::from_utf8_lossy(&log.text).trim().to_string()
}

/// The lines of a log that are not progress reports, and whether any
/// came.
#[derive(Default)]
struct Log {
    text: Vec<u8>,
    logged: bool,
}

impl Log {
    fn line(&mut self, line: &[u8], end: &SyncSender<bool>) {
        let line = line.trim_ascii_end();
        if line.is_empty() {
            return;
        }
        if line == b"progress=end" {
            let _ = end.try_send(!self.logged);
        } else if !is_report(line) {
            self.logged = true;
            self.text.extend_from_slice(line);
            self.text.push(b'\n');
            if self.text.len() > 16 << 10 {
                self.text.drain(..self.text.len() - (8 << 10));
            }
        }
    }
}

/// Whether `line` is a line of ffmpeg's progress report: `key=value`,
/// the key of lowercase letters, digits, and `_`.
fn is_report(line: &[u8]) -> bool {
    match line.iter().position(|&b| b == b'=') {
        Some(eq) if eq > 0 => line[..eq]
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        _ => false,
    }
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
mod tests;
