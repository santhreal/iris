//! Shared capture pipeline for the GPUI surfaces: grab, crop, save,
//! clipboard, library registration. Everything a surface needs after the
//! user commits a region or a window, plus the copy variants the editor
//! and library share (image, file, path, OCR text).

use std::path::{Path, PathBuf};

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
            .map_err(|e| eprintln!("iris: clipboard unavailable: {e}"))
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

/// Crop a region out of a frozen frame.
pub fn crop(frame: &Frame, region: Region) -> Result<image::RgbaImage, String> {
    if region.width == 0 || region.height == 0 {
        return Err("empty capture region".to_string());
    }
    if region.x + region.width > frame.width || region.y + region.height > frame.height {
        return Err(format!(
            "region {}x{}+{}+{} outside frame {}x{}",
            region.width, region.height, region.x, region.y, frame.width, frame.height
        ));
    }
    let mut out = image::RgbaImage::new(region.width, region.height);
    let raw = out.as_mut();
    let raw: &mut [u8] = &mut *raw;
    for row in 0..region.height {
        let src = ((region.y + row) * frame.width + region.x) as usize * 4;
        let dst = (row * region.width) as usize * 4;
        let len = region.width as usize * 4;
        raw[dst..dst + len].copy_from_slice(&frame.rgba[src..src + len]);
    }
    Ok(out)
}

/// Save an RGBA image to the screenshots dir, put it on the clipboard and
/// register it in the library. The capture is durable and paste-able
/// before any surface shows it.
pub fn finalize(img: &image::RgbaImage) -> Result<(PathBuf, library::CaptureEntry), String> {
    let cfg = Config::load();
    std::fs::create_dir_all(&cfg.screenshots_dir)
        .map_err(|e| format!("create screenshots dir: {e}"))?;
    let path = unique_path(&cfg);
    image::save_buffer(
        &path,
        img.as_raw(),
        img.width(),
        img.height(),
        image::ColorType::Rgba8,
    )
    .map_err(|e| format!("save screenshot: {e}"))?;
    // The file is the product; a clipboard failure degrades to a
    // log line, never a lost capture.
    if let Err(e) = copy_image(img) {
        eprintln!("iris: clipboard: {e}");
    }
    let entry = library::add(&path, img.width(), img.height())?;
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

