use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

/// Runtime parameters for one encode.
pub struct EncoderConfig {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub mic: bool,
}

/// Raw RGBA frames in, H.264/AAC mp4 out, via a piped ffmpeg child.
///
/// ffmpeg is resolved from PATH at start; absence or a failed encode is a
/// hard error, never a silent drop. yuv420p requires even dimensions, so a
/// no-op-on-even scale filter is always applied.
///
/// The capture thread never blocks on ffmpeg's stdin: frames go through a
/// bounded queue to a writer thread, and each consumed buffer returns for
/// reuse. A queue deeper than `QUEUE_DEPTH` means ffmpeg is behind and
/// backpressure applies, exactly as a blocking write would.
pub struct Encoder {
    child: Option<Child>,
    output: PathBuf,
    frame_bytes: usize,
    /// Full frames bound for ffmpeg's stdin.
    tx: Option<SyncSender<Vec<u8>>>,
    /// Emptied buffers back from the writer thread.
    recycle: Receiver<Vec<u8>>,
    writer: Option<JoinHandle<Result<(), String>>>,
}

/// Frames in flight between the capture thread and the writer. At 60fps
/// this is ~1/6s of slack; deeper means ffmpeg cannot keep up and the
/// capture thread should wait rather than grow memory.
const QUEUE_DEPTH: usize = 10;

impl Encoder {
    pub fn start(cfg: &EncoderConfig) -> Result<Self, String> {
        if cfg.width == 0 || cfg.height == 0 || cfg.fps == 0 {
            return Err(format!(
                "invalid encoder geometry {}x{} @ {} fps",
                cfg.width, cfg.height, cfg.fps
            ));
        }
        if let Some(parent) = cfg.output.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create recordings dir {}: {e}", parent.display()))?;
        }

        let size = format!("{}x{}", cfg.width, cfg.height);
        let fps = cfg.fps.to_string();
        let mut args: Vec<&str> = vec![
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &size,
            "-r",
            &fps,
            "-i",
            "pipe:0",
        ];
        if cfg.mic {
            args.extend(["-f", "pulse", "-i", "default"]);
        }
        args.extend([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            "-vf",
            "scale=trunc(iw/2)*2:trunc(ih/2)*2",
        ]);
        if cfg.mic {
            args.extend(["-c:a", "aac", "-shortest"]);
        }
        let output = cfg.output.to_string_lossy().into_owned();
        args.push(&output);

        crate::ilog!("iris: record: spawning ffmpeg -> {output}");
        let mut child = Command::new("ffmpeg")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn ffmpeg (is it on PATH?): {e}"))?;

        // A 1080p frame is 8.3MB through a 64KB pipe: ~130 write
        // syscalls per frame. Grow the pipe so each frame is a handful
        // of writes; failure is not fatal, the default still works.
        if let Some(stdin) = child.stdin.as_mut() {
            use std::os::unix::io::AsRawFd;
            let fd = stdin.as_raw_fd();
            unsafe {
                libc::fcntl(fd, libc::F_SETPIPE_SZ, 4 * 1024 * 1024);
            }
        }

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "ffmpeg stdin not piped".to_string())?;
        let (tx, rx) = sync_channel::<Vec<u8>>(QUEUE_DEPTH);
        let (rtx, recycle) = std::sync::mpsc::channel::<Vec<u8>>();
        let writer = std::thread::Builder::new()
            .name("iris-enc-writer".into())
            .spawn(move || {
                let mut stdin = std::io::BufWriter::new(stdin);
                for frame in rx.iter() {
                    if let Err(e) = stdin.write_all(&frame) {
                        return Err(format!("write frame to ffmpeg: {e}"));
                    }
                    let mut buf = frame;
                    buf.clear();
                    let _ = rtx.send(buf);
                }
                stdin
                    .flush()
                    .map_err(|e| format!("flush ffmpeg stdin: {e}"))
            })
            .map_err(|e| format!("spawn encoder writer: {e}"))?;

        Ok(Self {
            child: Some(child),
            output: cfg.output.clone(),
            frame_bytes: cfg.width as usize * cfg.height as usize * 4,
            tx: Some(tx),
            recycle,
            writer: Some(writer),
        })
    }

    /// An emptied frame buffer for the caller to fill and hand back
    /// through write_frame. Capacity is exactly one frame.
    pub fn take_buf(&mut self) -> Vec<u8> {
        self.recycle
            .try_recv()
            .unwrap_or_else(|_| Vec::with_capacity(self.frame_bytes))
    }

    /// Queue one tightly packed RGBA frame for the writer thread. The
    /// buffer moves into the queue and returns through take_buf once
    /// written, so no frame bytes are ever copied.
    pub fn write_frame(&mut self, buf: Vec<u8>) -> Result<(), String> {
        if buf.len() != self.frame_bytes {
            return Err(format!(
                "frame size mismatch: got {} bytes, expected {}",
                buf.len(),
                self.frame_bytes
            ));
        }
        // A dead writer means ffmpeg's stdin is gone; surface its error
        // rather than queueing into the void.
        if let Some(w) = &self.writer {
            if w.is_finished() {
                let w = self.writer.take().unwrap();
                return Err(w
                    .join()
                    .unwrap_or_else(|_| Err("encoder writer panicked".into()))
                    .unwrap_err());
            }
        }
        self.tx
            .as_ref()
            .ok_or_else(|| "encoder already finished".to_string())?
            .send(buf)
            .map_err(|_| "encoder writer gone".to_string())
    }

    /// Close the frame queue, wait for the writer and ffmpeg to flush,
    /// and verify the output file exists and is non-empty.
    pub fn finish(mut self) -> Result<PathBuf, String> {
        drop(self.tx.take());
        if let Some(w) = self.writer.take() {
            match w.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err("encoder writer panicked".to_string()),
            }
        }
        let mut child = self
            .child
            .take()
            .ok_or_else(|| "encoder already finished".to_string())?;
        drop(child.stdin.take());
        let out = child
            .wait_with_output()
            .map_err(|e| format!("wait on ffmpeg: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!(
                "ffmpeg exited with {}: {}",
                out.status,
                stderr.trim()
            ));
        }
        let meta = std::fs::metadata(&self.output)
            .map_err(|e| format!("output {} missing after encode: {e}", self.output.display()))?;
        crate::ilog!("iris: record: encoder finished, {} bytes", meta.len());
        if meta.len() == 0 {
            return Err(format!("output {} is empty", self.output.display()));
        }
        Ok(self.output.clone())
    }
}

/// Best-effort cleanup if the session dies without finish(): kill the child
/// so a headless ffmpeg does not outlive the app.
impl Drop for Encoder {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
