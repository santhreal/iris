//! The end of a recording: its segments become the output file.
//!
//! A recording is a run of Matroska segments, one per unpaused stretch
//! and one more per mic toggle or source resize. MP4 and WebM copy the
//! streams into the output container; GIF runs a two-pass palette
//! encode over them. When any segment has a mic track, the segments
//! without one get a silent track first, so every segment carries the
//! same streams and the copy is valid.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use super::child::Closing;
use super::codec::{audio_args, vfr_args};
use super::CANCELLED_PREFIX;
use crate::config::RecordingFormat;
use crate::tools::Tool;

/// One finished segment file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub path: PathBuf,
    /// Whether the segment has a mic track.
    pub audio: bool,
}

/// The file of segment `n` of the recording bound for `output`.
pub fn segment_path(output: &Path, n: usize) -> PathBuf {
    output.with_extension(format!("part{n}.mkv"))
}

/// A concat-demuxer list naming `paths` in order. Paths are single-
/// quoted; a `'` inside one is closed, escaped, and reopened (`'\''`).
pub fn concat_list(paths: &[&Path]) -> String {
    let mut out = String::new();
    for p in paths {
        out.push_str("file '");
        out.push_str(&p.to_string_lossy().replace('\'', "'\\''"));
        out.push_str("'\n");
    }
    out
}

/// GIF output is at most this wide; wider recordings scale down.
const GIF_MAX_W: u32 = 1280;

/// Join `segments`, in order, into `output`. On success the segment
/// files are gone; on failure they stay beside the output, and the
/// error names them.
pub fn join(segments: &[Segment], format: RecordingFormat, output: &Path) -> Result<(), String> {
    if segments.is_empty() {
        return Err("the recording has no frames".to_string());
    }
    let mut temps = Vec::new();
    let result = join_inner(segments, format, output, &mut temps);
    for t in &temps {
        let _ = std::fs::remove_file(t);
    }
    match result {
        Ok(()) => {
            for s in segments {
                let _ = std::fs::remove_file(&s.path);
            }
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(output);
            let kept: Vec<String> = segments
                .iter()
                .map(|s| s.path.display().to_string())
                .collect();
            Err(format!("{e}; segments kept: {}", kept.join(", ")))
        }
    }
}

/// Wait for every segment in `closing` and join, in order, those that
/// finished into `output`. A failed segment is left out, and its error
/// states where the others are.
pub fn finish(
    closing: Vec<Closing>,
    format: RecordingFormat,
    output: &Path,
) -> Result<PathBuf, String> {
    let mut segments = Vec::with_capacity(closing.len());
    let mut errors = Vec::new();
    for c in closing {
        match c.wait() {
            Ok(s) => segments.push(s),
            Err(e) => errors.push(e),
        }
    }
    if segments.is_empty() {
        return Err(errors
            .into_iter()
            .next()
            .unwrap_or_else(|| format!("{CANCELLED_PREFIX} stopped before the first frame")));
    }
    join(&segments, format, output)?;
    match errors.into_iter().next() {
        None => Ok(output.to_path_buf()),
        Some(e) => Err(format!(
            "{e}; the other segments are joined in {}",
            output.display()
        )),
    }
}

/// `finish` for a recording `err` ended: the error, and where the
/// recording up to it is.
pub fn finish_after(
    err: String,
    closing: Vec<Closing>,
    format: RecordingFormat,
    output: &Path,
) -> String {
    match finish(closing, format, output) {
        Ok(path) => format!("{err}; the recording up to it is at {}", path.display()),
        Err(e) if e.starts_with(CANCELLED_PREFIX) => err,
        Err(e) => format!("{err}; {e}"),
    }
}

fn join_inner(
    segments: &[Segment],
    format: RecordingFormat,
    output: &Path,
    temps: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let with_audio = segments.iter().any(|s| s.audio);
    let mut inputs: Vec<PathBuf> = Vec::with_capacity(segments.len());
    for s in segments {
        if with_audio && !s.audio {
            let silent = s.path.with_extension("silent.mkv");
            temps.push(silent.clone());
            add_silence(&s.path, format, &silent)?;
            inputs.push(silent);
        } else {
            inputs.push(s.path.clone());
        }
    }
    // One segment is read directly; more go through the concat demuxer.
    let input: Vec<String> = if let [only] = inputs.as_slice() {
        vec!["-i".into(), only.to_string_lossy().into_owned()]
    } else {
        let list = output.with_extension("parts.txt");
        let refs: Vec<&Path> = inputs.iter().map(PathBuf::as_path).collect();
        std::fs::write(&list, concat_list(&refs))
            .map_err(|e| format!("write {}: {e}", list.display()))?;
        temps.push(list.clone());
        ["-f", "concat", "-safe", "0", "-i"]
            .iter()
            .map(|s| s.to_string())
            .chain([list.to_string_lossy().into_owned()])
            .collect()
    };
    let out = output.to_string_lossy().into_owned();
    match format {
        RecordingFormat::Mp4 => ffmpeg(
            "join",
            input
                .iter()
                .map(String::as_str)
                .chain(["-map", "0", "-c", "copy", "-movflags", "+faststart"])
                .chain(["-f", "mp4", &out]),
        ),
        RecordingFormat::Webm => ffmpeg(
            "join",
            input
                .iter()
                .map(String::as_str)
                .chain(["-map", "0", "-c", "copy", "-f", "webm", &out]),
        ),
        RecordingFormat::Gif => {
            let palette = output.with_extension("palette.png");
            temps.push(palette.clone());
            let palette = palette.to_string_lossy().into_owned();
            let scale = format!("scale='min(iw,{GIF_MAX_W})':-2:flags=lanczos");
            ffmpeg(
                "GIF palette",
                input.iter().map(String::as_str).chain([
                    "-vf",
                    &format!("{scale},palettegen"),
                    "-frames:v",
                    "1",
                    "-update",
                    "1",
                    &palette,
                ]),
            )?;
            let graph = format!("[0:v]{scale}[x];[x][1:v]paletteuse=diff_mode=rectangle");
            ffmpeg(
                "GIF encode",
                input
                    .iter()
                    .map(String::as_str)
                    .chain(["-i", &palette, "-lavfi", &graph])
                    .chain(vfr_args())
                    .chain(["-f", "gif", &out]),
            )
        }
    }
}

/// Copy `seg` to `out` with a silent track in `format`'s mic codec.
fn add_silence(seg: &Path, format: RecordingFormat, out: &Path) -> Result<(), String> {
    let audio = audio_args(format).ok_or("a silent track for a format without audio")?;
    let seg = seg.to_string_lossy().into_owned();
    let out = out.to_string_lossy().into_owned();
    ffmpeg(
        "silent track",
        [
            "-i",
            &seg,
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "copy",
        ]
        .into_iter()
        .chain(audio)
        .chain(["-shortest", "-f", "matroska", &out]),
    )
}

/// Run one ffmpeg pass; `what` labels its error.
fn ffmpeg<'a>(what: &str, args: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    let out = Tool::Ffmpeg
        .command()
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| Tool::Ffmpeg.spawn_error(&e))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "ffmpeg {what} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_sit_beside_the_output() {
        let out = Path::new("/rec/2025-01-02_03-04-05.mp4");
        assert_eq!(
            segment_path(out, 3),
            Path::new("/rec/2025-01-02_03-04-05.part3.mkv")
        );
    }

    #[test]
    fn concat_list_quotes_and_escapes_each_segment() {
        let a = Path::new("/rec/a.part0.mkv");
        let b = Path::new("/rec/it's.part1.mkv");
        assert_eq!(
            concat_list(&[a, b]),
            "file '/rec/a.part0.mkv'\nfile '/rec/it'\\''s.part1.mkv'\n"
        );
    }

    #[test]
    fn nothing_to_join_is_an_error_and_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("r.mp4");
        std::fs::write(&out, b"").unwrap();
        assert!(join(&[], RecordingFormat::Mp4, &out).is_err());
        assert!(out.exists());
    }

    #[test]
    fn a_failed_join_keeps_the_segments_and_names_them() {
        // Not Matroska: ffmpeg rejects the input, or is not installed;
        // either way the join fails and must not delete the segment.
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("r.mp4");
        let seg = segment_path(&out, 0);
        std::fs::write(&seg, b"not a video").unwrap();
        let segs = [Segment {
            path: seg.clone(),
            audio: false,
        }];
        let err = join(&segs, RecordingFormat::Mp4, &out).unwrap_err();
        assert!(seg.exists(), "segment deleted on failure");
        assert!(!out.exists(), "partial output left behind");
        assert!(err.contains(&seg.display().to_string()), "{err}");
    }
}
