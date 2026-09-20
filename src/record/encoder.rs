use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

/// The raw pixel format frames arrive in, declared to ffmpeg through
/// -pix_fmt so the capture path never swizzles: a BGRX grab is fed as
/// `bgra` and memcpy'd, not converted per pixel. Alpha is dropped by
/// the yuv420p/rgb24 conversion downstream, so no stamping either.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PixFmt {
    /// 4 bytes/pixel, B,G,R,X in memory (X11 BGRX, PipeWire BGRx).
    Bgra,
    /// 4 bytes/pixel, R,G,B,A in memory (GL readPixels, PipeWire RGBx).
    Rgba,
    /// 3 bytes/pixel, B,G,R in memory (packed 24bpp X11 ZPixmap).
    Bgr24,
    /// 3 bytes/pixel, R,G,B in memory.
    Rgb24,
}

impl PixFmt {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgra | Self::Rgba => 4,
            Self::Bgr24 | Self::Rgb24 => 3,
        }
    }
    fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::Bgra => "bgra",
            Self::Rgba => "rgba",
            Self::Bgr24 => "bgr24",
            Self::Rgb24 => "rgb24",
        }
    }
}

/// Runtime parameters for one encode.
pub struct EncoderConfig {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub mic: bool,
    pub format: crate::config::RecordingFormat,
    pub encoder: crate::config::RecordingEncoder,
    /// Native format of the frames passed to write_frame.
    pub pix_fmt: PixFmt,
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
    /// Full frames and repeat markers bound for ffmpeg's stdin.
    tx: Option<SyncSender<FrameMsg>>,
    /// Emptied buffers back from the writer thread.
    recycle: Receiver<Vec<u8>>,
    /// Buffers freed by dropped frames, kept for the next take_buf.
    /// Without this a backpressured drop frees an 8MB frame and the
    /// next take_buf re-allocates it.
    spares: Vec<Vec<u8>>,
    writer: Option<JoinHandle<Result<(), String>>>,
    /// Frames dropped because the queue was full; logged at finish.
    dropped: u64,
}

/// Frames in flight between the capture thread and the writer. At 60fps
/// this is ~1/6s of slack; deeper means ffmpeg cannot keep up and the
/// capture thread should wait rather than grow memory.
const QUEUE_DEPTH: usize = 10;

/// One unit of writer work: a fresh frame buffer, or an instruction
/// to resend the buffer the writer already holds (an unchanged frame
/// costs no grab and no copy upstream).
enum FrameMsg {
    Buf(Vec<u8>),
    Repeat,
}

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

        use crate::config::{RecordingEncoder, RecordingFormat};
        let size = format!("{}x{}", cfg.width, cfg.height);
        let fps = cfg.fps.to_string();
        // GIF carries no audio; webm does, through opus.
        let mic = cfg.mic && cfg.format != RecordingFormat::Gif;
        let mut args: Vec<String> = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-y".into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            cfg.pix_fmt.ffmpeg_name().into(),
            "-s".into(),
            size,
            "-r".into(),
            fps,
            "-i".into(),
            "pipe:0".into(),
        ];
        if mic {
            args.extend(["-f".into(), "pulse".into(), "-i".into(), "default".into()]);
        }
        match cfg.format {
            RecordingFormat::Mp4 => {
                // NVENC refuses frames below its minimum dimension
                // (~145x49); a tiny window falls back to x264.
                let use_nvenc = match cfg.encoder {
                    RecordingEncoder::Nvenc => true,
                    RecordingEncoder::Libx264 => false,
                    RecordingEncoder::Auto => nvenc_available(),
                } && cfg.width >= 145 && cfg.height >= 49;
                if use_nvenc {
                    args.extend([
                        "-c:v".into(),
                        "h264_nvenc".into(),
                        "-preset".into(),
                        "p4".into(),
                        "-cq".into(),
                        "23".into(),
                        "-pix_fmt".into(),
                        "yuv420p".into(),
                    ]);
                } else {
                    args.extend([
                        "-c:v".into(),
                        "libx264".into(),
                        "-preset".into(),
                        "veryfast".into(),
                        "-crf".into(),
                        "23".into(),
                        "-pix_fmt".into(),
                        "yuv420p".into(),
                    ]);
                }
                args.extend([
                    "-vf".into(),
                    "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
                ]);
            }
            RecordingFormat::Gif => {
                // Per-frame palettes keep memory bounded on long
                // recordings; a single global palette would buffer
                // every frame before writing. GIF fps is capped: the
                // format's cost scales with frame count.
                let gif_fps = cfg.fps.min(20).to_string();
                args.extend([
                    "-vf".into(),
                    // format=rgb24 first: BGRX sources carry a garbage
                    // alpha byte that palettegen would read as
                    // transparency.
                    format!(
                        "format=rgb24,fps={gif_fps},scale=trunc(iw/2)*2:trunc(ih/2)*2:flags=lanczos,split[s0][s1];[s0]palettegen=stats_mode=single[p];[s1][p]paletteuse=new=1"
                    ),
                    "-f".into(),
                    "gif".into(),
                ]);
            }
            RecordingFormat::Webm => {
                args.extend([
                    "-c:v".into(),
                    "libvpx-vp9".into(),
                    "-deadline".into(),
                    "realtime".into(),
                    "-cpu-used".into(),
                    "5".into(),
                    "-crf".into(),
                    "32".into(),
                    "-b:v".into(),
                    "0".into(),
                    "-pix_fmt".into(),
                    "yuv420p".into(),
                    "-vf".into(),
                    "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
                ]);
            }
        }
        if mic {
            let codec = if cfg.format == RecordingFormat::Webm {
                "libopus"
            } else {
                "aac"
            };
            args.extend(["-c:a".into(), codec.into(), "-shortest".into()]);
        }
        let output = cfg.output.to_string_lossy().into_owned();
        args.push(output.clone());

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
        let (tx, rx) = sync_channel::<FrameMsg>(QUEUE_DEPTH);
        let (rtx, recycle) = std::sync::mpsc::channel::<Vec<u8>>();
        // The buffer must hold a whole frame or the 4MB pipe is never
        // used: an 8KB BufWriter still emits ~1000 writes per 8MB
        // frame. Cap at the pipe size so a huge frame degrades to a
        // few writes, not thousands.
        let buf_cap = (cfg.width as usize * cfg.height as usize * cfg.pix_fmt.bytes_per_pixel())
            .min(4 * 1024 * 1024);
        let writer = match std::thread::Builder::new()
            .name("iris-enc-writer".into())
            .spawn(move || {
                let mut stdin = std::io::BufWriter::with_capacity(buf_cap, stdin);
                // The last written buffer stays here so a Repeat can
                // resend it: an unchanged frame then costs no grab and
                // no copy anywhere upstream.
                let mut last: Option<Vec<u8>> = None;
                for msg in rx.iter() {
                    match msg {
                        FrameMsg::Buf(frame) => {
                            if let Err(e) = stdin.write_all(&frame) {
                                return Err(format!("write frame to ffmpeg: {e}"));
                            }
                            if let Some(old) = last.replace(frame) {
                                let mut buf = old;
                                buf.clear();
                                let _ = rtx.send(buf);
                            }
                        }
                        FrameMsg::Repeat => {
                            let Some(frame) = &last else {
                                continue;
                            };
                            if let Err(e) = stdin.write_all(frame) {
                                return Err(format!("write frame to ffmpeg: {e}"));
                            }
                        }
                    }
                }
                stdin
                    .flush()
                    .map_err(|e| format!("flush ffmpeg stdin: {e}"))
            }) {
            Ok(w) => w,
            Err(e) => {
                // No writer means ffmpeg would block on stdin forever:
                // kill it instead of orphaning a headless encoder.
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("spawn encoder writer: {e}"));
            }
        };

        Ok(Self {
            child: Some(child),
            output: cfg.output.clone(),
            frame_bytes: cfg.width as usize * cfg.height as usize * cfg.pix_fmt.bytes_per_pixel(),
            tx: Some(tx),
            recycle,
            spares: Vec::new(),
            writer: Some(writer),
            dropped: 0,
        })
    }

    /// An emptied frame buffer for the caller to fill and hand back
    /// through write_frame. Capacity is exactly one frame.
    pub fn take_buf(&mut self) -> Vec<u8> {
        if let Some(buf) = self.spares.pop() {
            return buf;
        }
        self.recycle
            .try_recv()
            .unwrap_or_else(|_| Vec::with_capacity(self.frame_bytes))
    }

    /// Queue one tightly packed RGBA frame for the writer thread. The
    /// buffer moves into the queue and returns through take_buf once
    /// written, so no frame bytes are ever copied. A full queue drops
    /// the frame rather than stalling the capture thread: a blocked
    /// grab loop slips the absolute frame schedule and the video plays
    /// fast-forwarded, while a dropped frame keeps real-time pacing.
    pub fn write_frame(&mut self, buf: Vec<u8>) -> Result<(), String> {
        if buf.len() != self.frame_bytes {
            return Err(format!(
                "frame size mismatch: got {} bytes, expected {}",
                buf.len(),
                self.frame_bytes
            ));
        }
        self.send(FrameMsg::Buf(buf))
    }

    /// Resend the frame the writer last wrote: the source did not
    /// change, so the grab and the frame copy are both skipped. Same
    /// drop-on-full backpressure as write_frame.
    pub fn repeat_frame(&mut self) -> Result<(), String> {
        self.send(FrameMsg::Repeat)
    }

    /// Shared send path: writer-liveness check, then try_send with
    /// drop-on-full backpressure.
    fn send(&mut self, msg: FrameMsg) -> Result<(), String> {
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
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| "encoder already finished".to_string())?;
        match tx.try_send(msg) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(msg)) => {
                // The frame is dropped; keep its buffer for the next
                // take_buf rather than freeing a frame-sized alloc.
                if let FrameMsg::Buf(mut buf) = msg {
                    if self.spares.len() < 2 {
                        buf.clear();
                        self.spares.push(buf);
                    }
                }
                self.dropped += 1;
                if self.dropped == 1 || self.dropped % 120 == 0 {
                    crate::ilog!(
                        "iris: record: encoder behind, {} frame(s) dropped",
                        self.dropped
                    );
                }
                Ok(())
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                Err("encoder writer gone".to_string())
            }
        }
    }

    /// Close the frame queue, wait for the writer and ffmpeg to flush,
    /// and verify the output file exists and is non-empty. The ffmpeg
    /// wait is bounded: a wedged encoder is killed rather than hanging
    /// the recording stop path (and with it, the daemon).
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
        // Poll try_wait: wait_with_output blocks forever on a wedged
        // child, and a recording stop must always come back.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("ffmpeg did not exit within 10s of stdin close; killed".into());
                }
                Err(e) => return Err(format!("wait on ffmpeg: {e}")),
            }
        };
        let mut stderr = String::new();
        if let Some(mut err) = child.stderr.take() {
            use std::io::Read;
            let _ = err.read_to_string(&mut stderr);
        }
        if !status.success() {
            return Err(format!(
                "ffmpeg exited with {}: {}",
                status,
                stderr.trim()
            ));
        }
        let meta = std::fs::metadata(&self.output)
            .map_err(|e| format!("output {} missing after encode: {e}", self.output.display()))?;
        crate::ilog!(
            "iris: record: encoder finished, {} bytes, {} dropped",
            meta.len(),
            self.dropped
        );
        if meta.len() == 0 {
            return Err(format!("output {} is empty", self.output.display()));
        }
        Ok(self.output.clone())
    }
}

/// Best-effort cleanup if the session dies without finish(): kill the
/// child BEFORE joining the writer — a writer blocked on a full pipe
/// only unblocks once ffmpeg is dead, so joining first deadlocks.
impl Drop for Encoder {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
    }
}

/// Whether h264_nvenc actually works on this machine. `-encoders` only
/// says the binary was built with it; a missing GPU or driver makes the
/// first real encode fail. Probe by encoding one black frame, once per
/// process, and cache the verdict.
fn nvenc_available() -> bool {
    use std::sync::LazyLock;
    static HAS: LazyLock<bool> = LazyLock::new(|| {
        Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel", "error",
                "-f", "lavfi",
                "-i", "color=black:s=256x256:d=0.1:r=1",
                "-frames:v", "1",
                "-c:v", "h264_nvenc",
                "-f", "null",
                "-",
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    });
    *HAS
}
