//! ffmpeg arguments every recording source shares: the video codec of
//! each format, the mic track, the timestamp mode, and the filter that
//! keeps every segment of one recording on the same canvas.
//!
//! A recording is a run of Matroska segments (see `join`): a killed
//! recorder leaves a playable file, and the join copies the streams
//! into the output container. Every segment of one recording uses the
//! same codec, canvas, and audio parameters, so the copy is valid.

use std::sync::LazyLock;

use crate::config::{RecordingEncoder, RecordingFormat};

impl RecordingFormat {
    /// The output file extension, without a dot.
    pub fn ext(self) -> &'static str {
        match self {
            RecordingFormat::Mp4 => "mp4",
            RecordingFormat::Gif => "gif",
            RecordingFormat::Webm => "webm",
        }
    }

    /// The ffmpeg encoder of the mic track, or None when the container
    /// holds no audio.
    pub fn audio_codec(self) -> Option<&'static str> {
        match self {
            RecordingFormat::Mp4 => Some("aac"),
            RecordingFormat::Webm => Some("libopus"),
            RecordingFormat::Gif => None,
        }
    }

    /// The capture rate for a configured rate, clamped to 1..=120. A
    /// GIF's size grows with its frame count, so GIF capture stops at
    /// 20 fps: 5 centiseconds per frame, a whole GIF delay unit.
    pub fn capture_fps(self, configured: u32) -> u32 {
        let fps = configured.clamp(1, 120);
        match self {
            RecordingFormat::Gif => fps.min(20),
            RecordingFormat::Mp4 | RecordingFormat::Webm => fps,
        }
    }
}

/// The video encoder of every segment of one recording, chosen once so
/// all of them join by stream copy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VideoCodec {
    /// H.264 on an NVIDIA GPU.
    Nvenc,
    /// H.264 in software.
    X264,
    /// VP9, for WebM.
    Vp9,
    /// Lossless RGB H.264: the intermediate the GIF palette pass reads.
    Lossless,
}

/// NVENC rejects frames smaller than this.
const NVENC_MIN: (u32, u32) = (145, 49);

impl VideoCodec {
    /// The codec for `format`. `canvas` is the recorded size (None: a
    /// whole display, larger than any encoder minimum). `nvenc` runs
    /// only when the choice depends on it.
    pub fn resolve(
        format: RecordingFormat,
        encoder: RecordingEncoder,
        canvas: Option<(u32, u32)>,
        nvenc: impl FnOnce() -> bool,
    ) -> Self {
        match format {
            RecordingFormat::Gif => VideoCodec::Lossless,
            RecordingFormat::Webm => VideoCodec::Vp9,
            RecordingFormat::Mp4 => {
                let fits = match canvas {
                    Some((w, h)) => w >= NVENC_MIN.0 && h >= NVENC_MIN.1,
                    None => true,
                };
                let wanted = match encoder {
                    RecordingEncoder::Libx264 => false,
                    RecordingEncoder::Nvenc => true,
                    RecordingEncoder::Auto => fits && nvenc(),
                };
                if wanted && fits {
                    VideoCodec::Nvenc
                } else {
                    VideoCodec::X264
                }
            }
        }
    }

    /// The codec as `resolve` picks it on this machine.
    pub fn for_recording(
        format: RecordingFormat,
        encoder: RecordingEncoder,
        canvas: Option<(u32, u32)>,
    ) -> Self {
        Self::resolve(format, encoder, canvas, nvenc_available)
    }

    /// Encoder arguments, output pixel format included. No H.264 encode
    /// uses B-frames. Segments are Matroska, which stores presentation
    /// times only, so the join's stream copy derives decode times, and
    /// for a reordered stream it derives them wrong: a two-frame segment
    /// of a still screen ended the MP4 two seconds early. h264_nvenc
    /// also reorders variable-rate timestamps into wrong ones.
    pub fn args(self) -> &'static [&'static str] {
        match self {
            VideoCodec::Nvenc => &[
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p4",
                "-cq",
                "23",
                "-bf",
                "0",
                "-pix_fmt",
                "yuv420p",
            ],
            VideoCodec::X264 => &[
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-bf", "0", "-pix_fmt",
                "yuv420p",
            ],
            VideoCodec::Vp9 => &[
                "-c:v",
                "libvpx-vp9",
                "-deadline",
                "realtime",
                "-cpu-used",
                "8",
                "-row-mt",
                "1",
                "-b:v",
                "0",
                "-crf",
                "32",
                "-pix_fmt",
                "yuv420p",
            ],
            VideoCodec::Lossless => &[
                "-c:v",
                "libx264rgb",
                "-preset",
                "ultrafast",
                "-qp",
                "0",
                "-bf",
                "0",
            ],
        }
    }
}

/// Mic track arguments for `format`, or None when it holds no audio.
/// Every segment encodes 48 kHz stereo, so the silent track the join
/// adds to a mic-less segment has the same parameters.
pub fn audio_args(format: RecordingFormat) -> Option<[&'static str; 8]> {
    let codec = format.audio_codec()?;
    let bitrate = if codec == "libopus" { "96k" } else { "128k" };
    Some(["-c:a", codec, "-b:a", bitrate, "-ar", "48000", "-ac", "2"])
}

/// Variable frame rate: every frame keeps its timestamp, and an
/// unchanged source writes no frames at all. ffmpeg 5.1 renamed
/// `-vsync` to `-fps_mode`; the flag is probed once per process.
pub fn vfr_args() -> [&'static str; 2] {
    static FPS_MODE: LazyLock<bool> = LazyLock::new(|| {
        crate::tools::Tool::Ffmpeg
            .command()
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "nullsrc=s=16x16:d=0.04",
                "-fps_mode",
                "vfr",
                "-f",
                "null",
                "-",
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success())
    });
    if *FPS_MODE {
        ["-fps_mode", "vfr"]
    } else {
        ["-vsync", "vfr"]
    }
}

/// Run the one-time ffmpeg probes a recording in `format` needs on a
/// background thread, so its first segment starts without waiting on
/// them. Called as a recording's source pick opens.
pub fn warm(format: RecordingFormat, encoder: RecordingEncoder) {
    let _ = std::thread::Builder::new()
        .name("iris-rec-probe".into())
        .spawn(move || {
            if format == RecordingFormat::Mp4 && encoder == RecordingEncoder::Auto {
                nvenc_available();
            }
            vfr_args();
        });
}

/// Whether h264_nvenc works on this machine. `-encoders` lists what the
/// binary was built with; a missing GPU or driver fails the first real
/// encode. The probe encodes one black frame, once per process.
fn nvenc_available() -> bool {
    static HAS: LazyLock<bool> = LazyLock::new(|| {
        crate::tools::Tool::Ffmpeg
            .command()
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=black:s=256x256:d=0.1:r=1",
                "-frames:v",
                "1",
                "-c:v",
                "h264_nvenc",
                "-f",
                "null",
                "-",
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success())
    });
    *HAS
}

/// `n` rounded down to even, at least 2: yuv420p encoders reject odd
/// frame dimensions.
pub fn even(n: u32) -> u32 {
    (n & !1).max(2)
}

/// The canvas of a recording whose first frame is `w`x`h`.
pub fn canvas_for(w: u32, h: u32) -> (u32, u32) {
    (even(w), even(h))
}

/// The filter that puts a `w`x`h` frame on `canvas`: an odd last row or
/// column is cropped, a frame that fits stays 1:1, a larger one scales
/// down to fit, and the result is centered on black. None when the
/// frame already is the canvas.
pub fn fit_filter(w: u32, h: u32, canvas: (u32, u32)) -> Option<String> {
    let (cw, ch) = canvas;
    // A 1-pixel edge has no even crop; the pad covers it.
    let ew = if w >= 2 { w & !1 } else { w };
    let eh = if h >= 2 { h & !1 } else { h };
    let (sw, sh) = if ew <= cw && eh <= ch {
        (ew, eh)
    } else {
        // Scale by the tighter axis; both results stay even and within
        // the canvas.
        let s = (f64::from(cw) / f64::from(ew)).min(f64::from(ch) / f64::from(eh));
        let fit = |n: u32, max: u32| even((f64::from(n) * s) as u32).min(max);
        (fit(ew, cw), fit(eh, ch))
    };
    let mut parts = Vec::new();
    if (ew, eh) != (w, h) {
        parts.push(format!("crop={ew}:{eh}:0:0"));
    }
    if (sw, sh) != (ew, eh) {
        parts.push(format!("scale={sw}:{sh}:flags=bicubic"));
    }
    if (sw, sh) != (cw, ch) {
        let x = ((cw - sw) / 2) & !1;
        let y = ((ch - sh) / 2) & !1;
        parts.push(format!("pad={cw}:{ch}:{x}:{y}:black"));
    }
    (!parts.is_empty()).then(|| parts.join(","))
}

// WHY: the class closed here is "segments of one recording do not
// join": a codec, canvas, or audio parameter that differs between two
// segments breaks the stream copy, and a format that names the wrong
// audio codec or extension breaks the container. Every format is
// matched exhaustively, so a new one fails to compile here until its
// row is written. Not covered: whether ffmpeg accepts the arguments
// (the recording smoke tests run ffmpeg).
#[cfg(test)]
mod tests {
    use super::*;
    use RecordingEncoder as E;
    use RecordingFormat as F;

    /// (extension, audio codec, capture fps for 60 configured).
    fn expected(f: F) -> (&'static str, Option<&'static str>, u32) {
        match f {
            F::Mp4 => ("mp4", Some("aac"), 60),
            F::Gif => ("gif", None, 20),
            F::Webm => ("webm", Some("libopus"), 60),
        }
    }

    const FORMATS: [F; 3] = [F::Mp4, F::Gif, F::Webm];

    #[test]
    fn every_format_names_its_container_audio_and_rate() {
        for f in FORMATS {
            let (ext, audio, fps) = expected(f);
            assert_eq!(
                (f.ext(), f.audio_codec(), f.capture_fps(60)),
                (ext, audio, fps)
            );
            assert_eq!(audio_args(f).map(|a| a[1]), audio, "{f:?}");
        }
    }

    #[test]
    fn capture_rate_is_clamped_before_the_gif_cap() {
        assert_eq!(F::Mp4.capture_fps(0), 1);
        assert_eq!(F::Mp4.capture_fps(500), 120);
        assert_eq!(F::Gif.capture_fps(12), 12);
        assert_eq!(F::Gif.capture_fps(0), 1);
    }

    #[test]
    fn audio_tracks_share_rate_and_layout() {
        for f in FORMATS {
            if let Some(a) = audio_args(f) {
                assert_eq!(a[4..], ["-ar", "48000", "-ac", "2"], "{f:?}");
            }
        }
    }

    #[test]
    fn codec_follows_format_encoder_and_canvas() {
        let big = Some((1920, 1080));
        let tiny = Some((144, 400));
        let cases = [
            (F::Mp4, E::Auto, big, true, VideoCodec::Nvenc),
            (F::Mp4, E::Auto, big, false, VideoCodec::X264),
            (F::Mp4, E::Auto, None, true, VideoCodec::Nvenc),
            (F::Mp4, E::Auto, tiny, true, VideoCodec::X264),
            (F::Mp4, E::Nvenc, big, false, VideoCodec::Nvenc),
            (F::Mp4, E::Nvenc, tiny, true, VideoCodec::X264),
            (F::Mp4, E::Libx264, big, true, VideoCodec::X264),
            (F::Webm, E::Nvenc, big, true, VideoCodec::Vp9),
            (F::Gif, E::Nvenc, big, true, VideoCodec::Lossless),
        ];
        for (f, e, canvas, has, want) in cases {
            assert_eq!(
                VideoCodec::resolve(f, e, canvas, || has),
                want,
                "{f:?} {e:?} {canvas:?}"
            );
        }
    }

    #[test]
    fn the_nvenc_probe_runs_only_when_the_choice_needs_it() {
        let probe = || -> bool { panic!("probed") };
        VideoCodec::resolve(F::Mp4, E::Libx264, Some((1920, 1080)), probe);
        VideoCodec::resolve(F::Mp4, E::Nvenc, Some((1920, 1080)), probe);
        VideoCodec::resolve(F::Mp4, E::Auto, Some((100, 100)), probe);
        VideoCodec::resolve(F::Gif, E::Auto, None, probe);
        VideoCodec::resolve(F::Webm, E::Auto, None, probe);
    }

    #[test]
    fn yuv_codecs_pin_420_and_the_gif_intermediate_is_lossless_rgb() {
        for c in [VideoCodec::Nvenc, VideoCodec::X264, VideoCodec::Vp9] {
            assert!(c.args().ends_with(&["-pix_fmt", "yuv420p"]), "{c:?}");
        }
        assert!(VideoCodec::Lossless.args().starts_with(&[
            "-c:v",
            "libx264rgb",
            "-preset",
            "ultrafast",
            "-qp",
            "0"
        ]));
    }

    /// Every codec. A new one fails to compile in the match until it is
    /// listed in the array too.
    fn every_codec() -> [VideoCodec; 4] {
        use VideoCodec::*;
        let all = [Nvenc, X264, Vp9, Lossless];
        for c in all {
            match c {
                Nvenc | X264 | Vp9 | Lossless => {}
            }
        }
        all
    }

    /// Segments keep presentation times only, so the join derives decode
    /// times, wrongly for a reordered stream: every H.264 encode turns
    /// B-frames off. The recorder tests prove the cut on x264, VP9, and
    /// the GIF intermediate; this covers NVENC, which needs a GPU.
    #[test]
    fn no_h264_codec_reorders_frames() {
        for c in every_codec() {
            let h264 = c.args()[1].contains("264");
            let no_bf = c.args().windows(2).any(|w| w == ["-bf", "0"]);
            assert_eq!(no_bf, h264, "{c:?}");
        }
    }

    #[test]
    fn even_rounds_down_with_floor_of_two() {
        assert_eq!([0, 1, 2, 3, 1919, 1080].map(even), [2, 2, 2, 2, 1918, 1080]);
    }

    #[test]
    fn canvas_frame_needs_no_filter() {
        assert_eq!(fit_filter(1920, 1080, (1920, 1080)), None);
    }

    #[test]
    fn odd_edges_are_cropped_not_scaled() {
        assert_eq!(
            fit_filter(801, 601, canvas_for(801, 601)).as_deref(),
            Some("crop=800:600:0:0")
        );
    }

    #[test]
    fn a_smaller_frame_stays_one_to_one_and_centered() {
        assert_eq!(
            fit_filter(400, 300, (800, 600)).as_deref(),
            Some("pad=800:600:200:150:black")
        );
    }

    #[test]
    fn a_larger_frame_scales_down_to_fit_and_centers() {
        // 1600x600 into 800x600: width-bound, half scale, 150px bars.
        assert_eq!(
            fit_filter(1600, 600, (800, 600)).as_deref(),
            Some("scale=800:300:flags=bicubic,pad=800:600:0:150:black")
        );
        // Odd and larger: crop first, then scale.
        assert_eq!(
            fit_filter(1601, 1201, (800, 600)).as_deref(),
            Some("crop=1600:1200:0:0,scale=800:600:flags=bicubic")
        );
    }

    #[test]
    fn fitted_sizes_stay_even_and_inside_the_canvas() {
        for (w, h) in [
            (1, 1),
            (3, 999),
            (1921, 1081),
            (5000, 7),
            (7, 5000),
            (641, 479),
        ] {
            let canvas = (640, 480);
            let Some(f) = fit_filter(w, h, canvas) else {
                continue;
            };
            let last = f.rsplit(',').next().unwrap();
            if let Some(dims) = last.strip_prefix("scale=") {
                let mut it = dims
                    .split([':', ','])
                    .map(|n| n.parse::<u32>().unwrap_or(0));
                let (sw, sh) = (it.next().unwrap(), it.next().unwrap());
                assert!(
                    sw % 2 == 0 && sh % 2 == 0 && sw <= 640 && sh <= 480,
                    "{w}x{h}: {f}"
                );
            } else {
                assert!(last.starts_with("pad=640:480:"), "{w}x{h}: {f}");
            }
        }
    }
}
