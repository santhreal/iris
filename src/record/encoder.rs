use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

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
pub struct Encoder {
    child: Option<Child>,
    output: PathBuf,
    frame_bytes: usize,
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

        eprintln!("iris: record: spawning ffmpeg -> {output}");
        let child = Command::new("ffmpeg")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn ffmpeg (is it on PATH?): {e}"))?;

        Ok(Self {
            child: Some(child),
            output: cfg.output.clone(),
            frame_bytes: cfg.width as usize * cfg.height as usize * 4,
        })
    }

    /// Write one tightly packed RGBA frame.
    pub fn write_frame(&mut self, rgba: &[u8]) -> Result<(), String> {
        if rgba.len() != self.frame_bytes {
            return Err(format!(
                "frame size mismatch: got {} bytes, expected {}",
                rgba.len(),
                self.frame_bytes
            ));
        }
        let stdin = self
            .child
            .as_mut()
            .and_then(|c| c.stdin.as_mut())
            .ok_or_else(|| "ffmpeg stdin closed".to_string())?;
        stdin
            .write_all(rgba)
            .map_err(|e| format!("write frame to ffmpeg: {e}"))
    }

    /// Close stdin, wait for ffmpeg to flush the container, and verify the
    /// output file exists and is non-empty.
    pub fn finish(mut self) -> Result<PathBuf, String> {
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
        eprintln!("iris: record: encoder finished, {} bytes", meta.len());
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
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
    }
}


