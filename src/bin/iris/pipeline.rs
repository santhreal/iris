//! Shared capture pipeline for the GPUI surfaces: grab, crop, save,
//! clipboard, library registration. Everything a surface needs after the
//! user commits a region or a window, plus the copy variants the editor
//! and library share (image, file, path, OCR text).

use std::path::{Path, PathBuf};

use image::ImageEncoder;

use iris_lib::{
    capture::{self, Frame},
    config::Config,
    dragcopy, library, ocr,
};

/// The process-global clipboard owner: the X11 selection is served only
/// while a Clipboard instance lives, so the app holds one for its whole
/// lifetime instead of dropping it after each capture. None when the
/// clipboard could not be opened: a capture must still save to disk.
pub static CLIPBOARD: std::sync::LazyLock<Option<parking_lot::Mutex<arboard::Clipboard>>> =
    std::sync::LazyLock::new(|| {
        arboard::Clipboard::new()
            .map(parking_lot::Mutex::new)
            .map_err(|e| iris_lib::ilog!("iris: clipboard unavailable: {e}"))
            .ok()
    });

/// Run `f` against the shared clipboard. An unavailable clipboard is
/// an honest error, never a panic mid-capture.
pub fn with_clipboard(
    f: impl FnOnce(&mut arboard::Clipboard),
) -> Result<(), String> {
    let Some(clipboard) = CLIPBOARD.as_ref() else {
        return Err("clipboard unavailable".into());
    };
    f(&mut clipboard.lock());
    Ok(())
}

/// Camera shutter at the grab moment: the freedesktop sound through
/// the first available player. Best effort; silence is not an error.
pub fn play_shutter_sound() {
    if !Config::load().sound_on_capture {
        return;
    }
    std::thread::spawn(|| {
        let path = "/usr/share/sounds/freedesktop/stereo/camera-shutter.oga";
        let attempts: [(&str, &[&str]); 3] = [
            ("pw-play", &[path]),
            ("paplay", &[path]),
            ("canberra-gtk-play", &["-i", "camera-shutter"]),
        ];
        for (bin, args) in attempts {
            if std::process::Command::new(bin).args(args).spawn().is_ok() {
                return;
            }
        }
    });
}

/// Save a frame under the config's screenshot template: placeholders
/// {date} and {time}, numeric suffix on collision.
fn unique_path(cfg: &Config) -> PathBuf {
    let now = chrono::Local::now();
    let name = cfg
        .screenshot_template
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H-%M-%S").to_string());
    let path = cfg.screenshots_dir.join(format!("{name}.png"));
    if !path.exists() {
        return path;
    }
    for n in 2.. {
        let candidate = cfg.screenshots_dir.join(format!("{name}-{n}.png"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

/// Grab the whole virtual screen once. The overlay freezes this frame and
/// every later crop comes out of it, so the screen cannot change under
/// the selection.
pub fn grab_frame() -> Result<Frame, String> {
    capture::backend()?.grab_screen()
}

/// A region in frame pixels, clamped to the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn from_corners(a: (f32, f32), b: (f32, f32), max_w: u32, max_h: u32) -> Self {
        let x = a.0.min(b.0).clamp(0.0, max_w as f32);
        let y = a.1.min(b.1).clamp(0.0, max_h as f32);
        let r = a.0.max(b.0).clamp(0.0, max_w as f32);
        let bot = a.1.max(b.1).clamp(0.0, max_h as f32);
        Region {
            x: x as u32,
            y: y as u32,
            width: (r - x) as u32,
            height: (bot - y) as u32,
        }
    }
}


/// Crop a region out of a BGRA buffer (the overlay's RenderImage
/// holds the frame's only CPU copy, already swizzled for the GPU).
/// The swizzle is symmetric: BGRA in, RGBA out.
pub fn crop_bgra(bgra: &[u8], width: u32, height: u32, region: Region) -> Result<image::RgbaImage, String> {
    if region.width == 0 || region.height == 0 {
        return Err("empty capture region".to_string());
    }
    if region.x + region.width > width || region.y + region.height > height {
        return Err(format!(
            "region {}x{}+{}+{} outside frame {}x{}",
            region.width, region.height, region.x, region.y, width, height
        ));
    }
    // Uninit capacity, not a zeroed image: the banded fill writes
    // every byte, and a 33MB memset before a 33MB fill is a wasted
    // pass. from_raw validates the size after the fill.
    let mut buf: Vec<u8> = Vec::with_capacity(region.width as usize * region.height as usize * 4);
    #[allow(clippy::uninit_vec)] // the banded fill writes every byte
    unsafe { buf.set_len(buf.capacity()) };
    let raw: &mut [u8] = &mut buf;
    let row_len = region.width as usize * 4;
    // Fused copy+swizzle per row, banded across threads once the crop
    // is big enough to matter (a 4K crop is 33MB).
    iris_lib::par::par_bands_mut(raw, row_len, |dst, start| {
        let row0 = (start / row_len) as u32;
        for (r, dst_row) in dst.chunks_exact_mut(row_len).enumerate() {
            let src = ((region.y + row0 + r as u32) * width + region.x) as usize * 4;
            for (d, s) in dst_row
                .chunks_exact_mut(4)
                .zip(bgra[src..src + row_len].chunks_exact(4))
            {
                let v = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                let rgb = (v & 0xFF00_FF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF);
                d.copy_from_slice(&rgb.to_le_bytes());
            }
        }
    });
    image::RgbaImage::from_raw(region.width, region.height, buf)
        .ok_or_else(|| "crop buffer size mismatch".to_string())
}


/// Save an RGBA image to the screenshots dir, put it on the clipboard and
/// register it in the library. The capture is durable and paste-able
/// before any surface shows it.
pub fn finalize(img: image::RgbaImage) -> Result<(PathBuf, library::CaptureEntry), String> {
    let t_png = std::time::Instant::now();
    let cfg = Config::load();
    std::fs::create_dir_all(&cfg.screenshots_dir)
        .map_err(|e| format!("create screenshots dir: {e}"))?;
    let path = unique_path(&cfg);
    // Fast PNG: the default Balanced deflate is ~3x slower than Fast on
    // a 4K frame, and a screenshot's redundancy means Fast still
    // compresses well. The capture is durable sooner and the toast's
    // thumbnail source exists earlier.
    let mut png = std::io::Cursor::new(Vec::new());
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| format!("encode screenshot: {e}"))?;
    std::fs::write(&path, png.into_inner())
        .map_err(|e| format!("save screenshot: {e}"))?;
    iris_lib::ilog!("iris: capture: png written in {:?}", t_png.elapsed());
    // The file is the product; a clipboard failure degrades to a
    // log line, never a lost capture. copy_to_clipboard gates whether
    // the capture lands on the clipboard at all. The clipboard set
    // re-encodes the pixels to PNG at arboard's default compression,
    // which is slower than the file encode above: it runs on its own
    // thread so the toast does not wait on a second encode.
    let img = std::sync::Arc::new(img);
    if cfg.copy_to_clipboard {
        let img = img.clone();
        std::thread::Builder::new()
            .name("iris-clipboard".into())
            .spawn(move || {
                if let Err(e) = copy_image(&img) {
                    iris_lib::ilog!("iris: clipboard: {e}");
                }
            })
            .map_err(|e| format!("spawn clipboard thread: {e}"))?;
    }
    let entry = library::add(&path, &img)?;
    Ok((path, entry))
}

pub fn copy_image(img: &image::RgbaImage) -> Result<(), String> {
    let mut out = Ok(());
    with_clipboard(|c| {
        out = c
            .set_image(arboard::ImageData {
                width: img.width() as usize,
                height: img.height() as usize,
                bytes: std::borrow::Cow::Borrowed(img.as_raw()),
            })
            .map_err(|e| format!("set clipboard image: {e}"));
    })?;
    out
}

pub fn copy_image_file(path: &Path) -> Result<(), String> {
    let mut out = Ok(());
    with_clipboard(|c| out = dragcopy::clipboard_set_image(c, path))?;
    out
}

pub fn copy_path_text(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("path does not exist: {}", path.display()));
    }
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = abs.to_string_lossy().into_owned();
    let mut out = Ok(());
    with_clipboard(|c| out = dragcopy::clipboard_set_text(c, &text))?;
    out
}

/// Copy as file (text/uri-list): file managers paste the file itself.
pub fn copy_file(path: &Path) -> Result<(), String> {
    dragcopy::copy_file_path(path)
}

pub fn copy_files(paths: &[PathBuf]) -> Result<(), String> {
    dragcopy::copy_file_paths(paths)
}

/// OCR an image to the clipboard. Returns the recognized text so the
/// caller can report the character count.
pub fn copy_ocr_text(path: &Path) -> Result<String, String> {
    let text = ocr::recognize_text(path)?;
    let mut out = Ok(());
    with_clipboard(|c| out = dragcopy::clipboard_set_text(c, &text))?;
    out?;
    Ok(text)
}


// WHY: the class closed here is "captures land in the wrong place or
// clobber each other": a template that stops substituting, a suffix
// collision that overwrites, or a region that crops out of bounds all
// lose the user's shot. Not covered: the X11/Wayland grab itself.
#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_at(dir: &Path) -> Config {
        Config {
            screenshots_dir: dir.to_path_buf(),
            ..Config::default()
        }
    }

    #[test]
    fn unique_path_substitutes_template_and_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = cfg_at(dir.path());
        let first = unique_path(&cfg);
        assert!(first.to_string_lossy().ends_with(".png"));
        assert!(!first.to_string_lossy().contains('{'));
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(&cfg);
        assert_ne!(first, second);
    }

    #[test]
    fn region_from_corners_normalizes_and_clamps() {
        // Reversed drag direction.
        let r = Region::from_corners((50.0, 40.0), (10.0, 5.0), 100, 100);
        assert_eq!(r, Region { x: 10, y: 5, width: 40, height: 35 });
        // Drag past the frame edges clamps to the frame.
        let r = Region::from_corners((-20.0, -10.0), (150.0, 120.0), 100, 100);
        assert_eq!(r, Region { x: 0, y: 0, width: 100, height: 100 });
    }

    #[test]
    fn crop_rejects_empty_and_out_of_bounds() {
        let bgra = vec![0u8; 64];
        assert!(crop_bgra(&bgra, 4, 4, Region { x: 0, y: 0, width: 0, height: 2 }).is_err());
        assert!(crop_bgra(&bgra, 4, 4, Region { x: 3, y: 0, width: 2, height: 2 }).is_err());
    }

    #[test]
    fn crop_copies_exact_pixels() {
        // BGRA source: build it by swizzling an RGBA ramp so the
        // expected output is the ramp itself.
        let mut rgba = vec![0u8; 4 * 4 * 4];
        for i in 0..rgba.len() {
            rgba[i] = i as u8;
        }
        let mut bgra = rgba.clone();
        crate::widgets::swizzle_rgba_bgra(&mut bgra);
        let out = crop_bgra(&bgra, 4, 4, Region { x: 1, y: 1, width: 2, height: 2 }).unwrap();
        assert_eq!(out.width(), 2);
        // Top-left of the crop is frame pixel (1,1) = byte offset 20.
        assert_eq!(&out.as_raw()[..4], &[20, 21, 22, 23]);
    }

    #[test]
    fn parallel_crop_matches_serial() {
        // Over the 1MP parallel threshold: every band must land its
        // rows at the right offset with the same BGRA->RGBA swap.
        let (w, h) = (1600u32, 1000u32);
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for i in 0..(w * h) as usize {
            bgra[i * 4] = (i % 251) as u8;
            bgra[i * 4 + 1] = (i % 253) as u8;
            bgra[i * 4 + 2] = (i % 255) as u8;
            bgra[i * 4 + 3] = 255;
        }
        let region = Region { x: 300, y: 200, width: 600, height: 450 };
        let out = crop_bgra(&bgra, w, h, region).unwrap();
        assert_eq!(out.dimensions(), (600, 450));
        // Spot-check first, middle and last pixels of the crop.
        for (dx, dy) in [(0u32, 0u32), (300, 225), (599, 449)] {
            let src = (((200 + dy) * w + 300 + dx) * 4) as usize;
            let dst = ((dy * 600 + dx) * 4) as usize;
            assert_eq!(out.as_raw()[dst], bgra[src + 2], "R at {dx},{dy}");
            assert_eq!(out.as_raw()[dst + 1], bgra[src + 1], "G at {dx},{dy}");
            assert_eq!(out.as_raw()[dst + 2], bgra[src], "B at {dx},{dy}");
            assert_eq!(out.as_raw()[dst + 3], 255, "A at {dx},{dy}");
        }
    }
}
