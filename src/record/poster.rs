//! A recording's poster frame: its first frame, scaled to cover a box
//! and cropped to it, for the saved-recording notice.

use std::path::Path;
use std::process::Stdio;

use crate::tools::Tool;

/// The first frame of the video at `path`, scaled to cover a `w`x`h`
/// box and centre-cropped to it, as RGBA. ffmpeg decodes one frame and
/// writes the box's raw pixels, so no image is encoded or decoded here.
pub fn poster(path: &Path, w: u32, h: u32) -> Result<image::RgbaImage, String> {
    if w == 0 || h == 0 {
        return Err(format!("poster box {w}x{h} has no area"));
    }
    let filter =
        format!("scale={w}:{h}:force_original_aspect_ratio=increase:flags=area,crop={w}:{h}");
    let out = Tool::Ffmpeg
        .command()
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &filter])
        .args(["-pix_fmt", "rgba", "-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| Tool::Ffmpeg.spawn_error(&e))?;
    if !out.status.success() {
        return Err(crate::tools::failure(
            "ffmpeg poster",
            out.status,
            &String::from_utf8_lossy(&out.stderr),
        ));
    }
    let len = out.stdout.len();
    let want = w as usize * h as usize * 4;
    if len != want {
        return Err(format!(
            "ffmpeg poster: {len} bytes for a {w}x{h} frame of {}",
            path.display()
        ));
    }
    image::RgbaImage::from_raw(w, h, out.stdout)
        .ok_or_else(|| format!("ffmpeg poster: no {w}x{h} frame in {len} bytes"))
}

#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    /// A `w`x`h` clip, red on its left half and blue on its right, in
    /// `dir`; None where no ffmpeg is installed.
    fn split_clip(dir: &Path, w: u32, h: u32) -> Option<std::path::PathBuf> {
        Tool::Ffmpeg.find()?;
        let out = dir.join("clip.mkv");
        let half = format!("{}x{h}", w / 2);
        let status = Tool::Ffmpeg
            .command()
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", &format!("color=c=red:s={half}:d=1")])
            .args(["-f", "lavfi", "-i", &format!("color=c=blue:s={half}:d=1")])
            .args(["-filter_complex", "hstack", "-c:v", "ffv1"])
            .arg(&out)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "ffmpeg made no clip");
        Some(out)
    }

    fn is(px: &image::Rgba<u8>, want: [u8; 3]) -> bool {
        px.0[3] == 255 && (0..3).all(|i| px.0[i].abs_diff(want[i]) < 24)
    }

    /// WHY: the class closed is "the poster shows the wrong part of the
    /// recording, or a frame of the wrong size": a wide clip in a square
    /// box must fill the box and keep its centre, so both halves meet in
    /// the middle, and a tall box keeps the centre column of a wide clip.
    /// Not covered: which frame of a long clip is first; every recording
    /// starts on the screen it shows.
    #[test]
    fn the_poster_covers_its_box_and_keeps_the_centre() {
        let dir = tempfile::tempdir().unwrap();
        let Some(clip) = split_clip(dir.path(), 320, 100) else {
            eprintln!("skipped: no ffmpeg");
            return;
        };
        for (w, h) in [(40, 40), (64, 40), (20, 60)] {
            let img = poster(&clip, w, h).unwrap();
            assert_eq!(img.dimensions(), (w, h));
            for y in [0, h / 2, h - 1] {
                let left = img.get_pixel(1, y);
                let right = img.get_pixel(w - 2, y);
                assert!(is(left, [255, 0, 0]), "{w}x{h} left {left:?}");
                assert!(is(right, [0, 0, 255]), "{w}x{h} right {right:?}");
            }
        }
    }

    #[test]
    fn a_file_that_is_no_video_fails_with_ffmpegs_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("r.mp4");
        std::fs::write(&path, b"not a video").unwrap();
        let err = poster(&path, 64, 40).unwrap_err();
        assert!(
            err.starts_with("ffmpeg poster failed") || err.contains("not installed"),
            "{err}"
        );
        assert!(poster(&path, 0, 40).unwrap_err().contains("no area"));
    }
}
