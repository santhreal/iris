//! One segment of a recording: timestamped raw frames in, a Matroska
//! file out, through a piped ffmpeg child.
//!
//! Frames reach ffmpeg framed by `mkv`, each stamped with its capture
//! time, and the encode keeps those times: an unchanged source writes
//! no frames. The mic is a PulseAudio input whose wallclock timestamps
//! are shifted onto the same timeline, so audio and video line up by
//! capture time, not by when each input happened to open.
//!
//! The capture thread never blocks on ffmpeg's stdin: frames go through
//! `queue` to a writer thread, and every written buffer returns for
//! reuse.

mod queue;

use std::io;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::child::{drain, wait_until, written, Closing};
use super::codec::{audio_args, fit_filter, vfr_args, VideoCodec};
use super::join::Segment;
use super::mkv::{self, PixFmt};
use crate::config::RecordingFormat;
use crate::tools::Tool;
use queue::{hang_up, offer, write_stream, Queued, Slot, Stamped};

/// What one segment encodes.
pub struct SegmentSpec {
    pub path: PathBuf,
    /// Size and layout of every frame the segment receives.
    pub width: u32,
    pub height: u32,
    pub pix: PixFmt,
    /// The recording's canvas: every segment encodes at this size.
    pub canvas: (u32, u32),
    /// Nominal rate: the last frame's duration and the reported rate.
    pub fps: u32,
    pub codec: VideoCodec,
    pub format: RecordingFormat,
    /// Record the default PulseAudio source.
    pub mic: bool,
    /// Time zero: the capture time of the segment's first frame.
    pub epoch: Instant,
}

/// Queued frames stay under this many bytes: ten 1080p frames, two 4K
/// frames. More queue means ffmpeg is behind, and frames then drop
/// instead of growing memory.
const QUEUE_BYTES: usize = 96 << 20;

/// How long a closing segment may take to drain its queue and let
/// ffmpeg flush. A wedged ffmpeg is killed at the limit.
const FLUSH_LIMIT: Duration = Duration::from_secs(10);

pub struct Encoder {
    child: Option<Child>,
    path: PathBuf,
    audio: bool,
    epoch: Instant,
    frame_bytes: usize,
    /// Timestamp of the newest frame accepted, queued or in the slot.
    last_ms: Option<u64>,
    slot: Slot,
    tx: Option<SyncSender<Stamped>>,
    recycle: Receiver<Vec<u8>>,
    spares: Vec<Vec<u8>>,
    writer: Option<Writer>,
    /// The writer's last frame, sent when it exits.
    held: Option<Receiver<Vec<u8>>>,
    stderr: Option<JoinHandle<String>>,
    dropped: u64,
}

/// The writer thread, and a channel that disconnects as it ends.
struct Writer {
    thread: JoinHandle<io::Result<()>>,
    exited: Receiver<()>,
}

impl Encoder {
    pub fn start(spec: &SegmentSpec) -> Result<Self, String> {
        let (w, h) = (spec.width, spec.height);
        if w == 0 || h == 0 {
            return Err(format!("invalid segment geometry {w}x{h}"));
        }
        if let Some(dir) = spec.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("create recordings dir {}: {e}", dir.display()))?;
        }
        let audio = spec.mic.then(|| audio_args(spec.format)).flatten();
        let args = ffmpeg_args(spec, audio.as_ref().map(|a| a.as_slice()));
        crate::ilog!("iris: record: segment {}", spec.path.display());
        let mut child = Tool::Ffmpeg
            .command()
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Tool::Ffmpeg.spawn_error(&e))?;
        let frame_bytes = w as usize * h as usize * spec.pix.bytes_per_pixel();
        let (stdin, stderr) = match (child.stdin.take(), child.stderr.take()) {
            (Some(i), Some(e)) => (i, e),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("ffmpeg stdio not piped".to_string());
            }
        };
        grow_pipe(&stdin, frame_bytes);
        let depth = (QUEUE_BYTES / frame_bytes).clamp(2, 10);
        let (tx, rx) = sync_channel::<Stamped>(depth);
        let (recycle_tx, recycle) = channel::<Vec<u8>>();
        let (held_tx, held) = channel::<Vec<u8>>();
        let header = mkv::header(w, h, spec.pix, spec.fps);
        let slot = Slot::default();
        let queued = Queued {
            rx,
            slot: slot.clone(),
        };
        let (exit_tx, exited) = channel::<()>();
        let spawned = std::thread::Builder::new()
            .name("iris-rec-writer".into())
            .spawn(move || {
                // Dropped as the thread ends, however it ends.
                let _exit = exit_tx;
                let mut last: Option<Vec<u8>> = None;
                let result = write_stream(stdin, &header, queued, &recycle_tx, &mut last);
                if let Some(frame) = last {
                    let _ = held_tx.send(frame);
                }
                result
            })
            .and_then(|thread| drain(stderr).map(|err| (Writer { thread, exited }, err)));
        let (writer, stderr) = match spawned {
            Ok(pair) => pair,
            Err(e) => {
                // No writer: ffmpeg would wait on stdin forever.
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("spawn segment writer: {e}"));
            }
        };
        Ok(Self {
            child: Some(child),
            path: spec.path.clone(),
            audio: audio.is_some(),
            epoch: spec.epoch,
            frame_bytes,
            last_ms: None,
            slot,
            tx: Some(tx),
            recycle,
            spares: Vec::new(),
            writer: Some(writer),
            held: Some(held),
            stderr: Some(stderr),
            dropped: 0,
        })
    }

    /// An empty buffer to capture the next frame into.
    pub fn take_buf(&mut self) -> Vec<u8> {
        self.recycle
            .try_recv()
            .ok()
            .or_else(|| self.spares.pop())
            .unwrap_or_else(|| Vec::with_capacity(self.frame_bytes))
    }

    /// Return a buffer that was taken and not written.
    pub fn give_back(&mut self, mut buf: Vec<u8>) {
        if self.spares.len() < 2 {
            buf.clear();
            self.spares.push(buf);
        }
    }

    /// Queue one frame captured at `at`. While the queue is full it
    /// waits in the slot, where a newer frame replaces it.
    pub fn write(&mut self, frame: Vec<u8>, at: Instant) -> Result<(), String> {
        if frame.len() != self.frame_bytes {
            return Err(format!(
                "frame of {} bytes, segment expects {}",
                frame.len(),
                self.frame_bytes
            ));
        }
        let ts = self.stamp(at);
        self.last_ms = Some(ts);
        let Some(tx) = &self.tx else {
            return Err("segment already finished".to_string());
        };
        match offer(tx, &self.slot, (frame, ts)) {
            Ok(None) => Ok(()),
            Ok(Some(displaced)) => {
                self.dropped += 1;
                if self.dropped == 1 || self.dropped.is_multiple_of(120) {
                    crate::ilog!(
                        "iris: record: encoder behind, {} frame(s) dropped",
                        self.dropped
                    );
                }
                self.give_back(displaced);
                Ok(())
            }
            Err(_) => Err(self.failure()),
        }
    }

    /// Milliseconds from the epoch to `at`, kept strictly increasing:
    /// two frames in one millisecond would share a timestamp.
    fn stamp(&self, at: Instant) -> u64 {
        let ms = at.saturating_duration_since(self.epoch).as_millis() as u64;
        match self.last_ms {
            Some(last) if ms <= last => last + 1,
            _ => ms,
        }
    }

    /// End the segment at `at`. The writer writes the frame waiting in
    /// the slot and a tail frame at `at`, then ffmpeg flushes, all on
    /// the returned finisher. The receiver delivers the segment's last
    /// frame once its writer exits.
    pub fn finish(mut self, at: Instant) -> (Closing, Receiver<Vec<u8>>) {
        // The picture lasted until `at`: repeat it there, unless `at`
        // is no later than the newest frame.
        let tail = self.last_ms.and_then(|last| {
            let ms = at.saturating_duration_since(self.epoch).as_millis() as u64;
            (ms > last).then_some(ms)
        });
        if let Some(tx) = self.tx.take() {
            hang_up(tx, &self.slot, tail);
        }
        let (child, writer) = (self.child.take(), self.writer.take());
        let stderr = self.stderr.take();
        let held = self.held.take().expect("held receiver taken once");
        let (path, audio, dropped) = (self.path.clone(), self.audio, self.dropped);
        let closing = Closing::spawn(move || {
            let deadline = Instant::now() + FLUSH_LIMIT;
            close(child, writer, stderr, path, audio, dropped, deadline)
        });
        // Without a finisher the queue still closes: the writer drains,
        // ffmpeg reads EOF and exits on its own.
        (closing, held)
    }

    /// The error of a writer that stopped early: ffmpeg's own message
    /// when it exited, the pipe error otherwise.
    fn failure(&mut self) -> String {
        drop(self.tx.take());
        let wrote = self.writer.take().map(|w| w.thread.join());
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = self
            .child
            .as_mut()
            .and_then(|c| wait_until(c, deadline).ok());
        let text = self
            .stderr
            .take()
            .and_then(|e| e.join().ok())
            .unwrap_or_default();
        match (status, wrote) {
            (Some(s), _) if !s.success() && !text.is_empty() => {
                format!("ffmpeg exited {s}: {text}")
            }
            (_, Some(Ok(Err(e)))) => format!("write frame to ffmpeg: {e}"),
            _ => "ffmpeg stopped reading frames".to_string(),
        }
    }
}

/// A dropped encoder kills its ffmpeg before joining the writer: a
/// writer blocked on a full pipe only returns once ffmpeg is gone.
impl Drop for Encoder {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(w) = self.writer.take() {
            let _ = w.thread.join();
        }
    }
}

/// The ffmpeg arguments of one segment.
fn ffmpeg_args(spec: &SegmentSpec, audio: Option<&[&str]>) -> Vec<String> {
    let mut a: Vec<String> = Vec::with_capacity(48);
    let mut push = |args: &[&str]| a.extend(args.iter().map(|s| s.to_string()));
    push(&["-hide_banner", "-loglevel", "error", "-y", "-copyts"]);
    // The header states every stream parameter: nothing to probe.
    push(&["-probesize", "32", "-analyzeduration", "0"]);
    push(&["-f", "matroska", "-i", "pipe:0"]);
    if audio.is_some() {
        // Pulse stamps packets with wallclock microseconds; the offset
        // moves them onto the video's timeline, where the epoch is 0.
        let now = (SystemTime::now(), Instant::now());
        let wall = now.0 - now.1.saturating_duration_since(spec.epoch);
        let epoch = wall.duration_since(UNIX_EPOCH).unwrap_or_default();
        let offset = format!("-{}.{:06}", epoch.as_secs(), epoch.subsec_micros());
        push(&["-thread_queue_size", "512", "-itsoffset", &offset]);
        push(&["-f", "pulse", "-name", "iris", "-i", "default"]);
    }
    push(&["-map", "0:v"]);
    if audio.is_some() {
        push(&["-map", "1:a"]);
    }
    if let Some(f) = fit_filter(spec.width, spec.height, spec.canvas) {
        push(&["-vf", &f]);
    }
    push(spec.codec.args());
    // Millisecond encoder clock: the default, 1/fps, would round
    // capture times onto a fixed grid and merge nearby frames.
    push(&["-enc_time_base:v", "1:1000"]);
    push(&vfr_args());
    if let Some(audio) = audio {
        push(audio);
        // Pulse never ends; the segment ends with its last frame.
        push(&["-shortest"]);
    }
    push(&["-f", "matroska"]);
    a.push(spec.path.to_string_lossy().into_owned());
    a
}

/// Grow the pipe toward one frame: a 64KB pipe splits a 1080p frame
/// into ~130 writes. Unprivileged pipes stop at
/// /proc/sys/fs/pipe-max-size (1MB by default), so a refused size
/// falls back to 1MB; failing that the default pipe still works.
fn grow_pipe(stdin: &ChildStdin, frame_bytes: usize) {
    use std::os::fd::AsRawFd;
    let fd = stdin.as_raw_fd();
    for size in [frame_bytes.clamp(1 << 20, 16 << 20), 1 << 20] {
        // SAFETY: fd is the open write end of the child's stdin pipe.
        if unsafe { libc::fcntl(fd, libc::F_SETPIPE_SZ, size as libc::c_int) } > 0 {
            return;
        }
    }
}

/// The finisher: the writer drains its queue and closes stdin, ffmpeg
/// flushes and exits, and the segment file must hold something. An
/// ffmpeg that stops reading is killed at `deadline`, which also frees
/// a writer blocked on the full pipe.
fn close(
    child: Option<Child>,
    writer: Option<Writer>,
    stderr: Option<JoinHandle<String>>,
    path: PathBuf,
    audio: bool,
    dropped: u64,
    deadline: Instant,
) -> Result<Segment, String> {
    let mut child = child.ok_or("segment has no ffmpeg")?;
    if let Some(w) = &writer {
        let left = deadline.saturating_duration_since(Instant::now());
        if matches!(w.exited.recv_timeout(left), Err(RecvTimeoutError::Timeout)) {
            let _ = child.kill();
        }
    }
    let wrote = writer.map(|w| w.thread.join());
    let status = wait_until(&mut child, deadline);
    let text = stderr.and_then(|e| e.join().ok()).unwrap_or_default();
    let status = status?;
    if !status.success() {
        return Err(format!("ffmpeg exited {status}: {text}"));
    }
    match wrote {
        Some(Ok(Ok(()))) => {}
        Some(Ok(Err(e))) => return Err(format!("write frame to ffmpeg: {e}")),
        _ => return Err("segment writer panicked".to_string()),
    }
    crate::ilog!("iris: record: segment done, {dropped} frame(s) dropped");
    written(Segment { path, audio })
}
