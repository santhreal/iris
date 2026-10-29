//! OCR: copy all recognized text in a capture to the clipboard. macOS
//! recognizes text with the Vision framework and Windows with
//! Windows.Media.Ocr. Linux runs tesseract, a runtime dependency
//! resolved by `tools`; its absence is an error that names the install
//! command.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Recognize text in the image at `path`: the toast's Copy Text menu
/// row, over the saved capture.
pub fn recognize_text(source: &Path) -> Result<String, String> {
    if !source.exists() {
        return Err(format!("image does not exist: {}", source.display()));
    }
    let text = recognize(source)?;
    let text = text.trim();
    if text.is_empty() {
        return Err("no text recognized in this image".to_string());
    }
    Ok(text.to_string())
}

/// Recognize text in `png`, an encoded image: the editor's Copy Text
/// reads the image as shown, which no file holds until Done saves it.
/// The bytes reach the recognizer through a file in the cache
/// directory, deleted once read.
pub fn recognize_png(png: &[u8]) -> Result<String, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = crate::dirs::cache_dir().ok_or("no cache directory to hold the image for OCR")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let file = dir.join(format!(
        "ocr-{}-{}.png",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&file, png).map_err(|e| format!("write {}: {e}", file.display()))?;
    let text = recognize_text(&file);
    if let Err(e) = std::fs::remove_file(&file) {
        crate::ilog!("iris: ocr: delete {}: {e}", file.display());
    }
    text
}

#[cfg(target_os = "macos")]
fn recognize(source: &Path) -> Result<String, String> {
    crate::sys::vision::recognize_text(source)
}

#[cfg(windows)]
fn recognize(source: &Path) -> Result<String, String> {
    crate::sys::media_ocr::recognize_text(source)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn recognize(source: &Path) -> Result<String, String> {
    use crate::tools::Tool;
    let out = Tool::Tesseract
        .command()
        .arg(source)
        .arg("stdout")
        .output()
        .map_err(|e| Tool::Tesseract.spawn_error(&e))?;
    if !out.status.success() {
        return Err(crate::tools::failure(
            "tesseract",
            out.status,
            &String::from_utf8_lossy(&out.stderr),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// WHY: the class closed here is "Copy Text depends on a tool the app
// does not ship": macOS reads text through Vision and Windows through
// Windows.Media.Ocr, so a packaged app with nothing else installed
// copies text. These run on the macOS and Windows CI runners, which
// have no tesseract. Not covered: recognition of anything harder than
// clean rendered text, a Windows profile with no OCR language, and
// Linux, where tesseract does the recognition.
#[cfg(all(test, any(windows, target_os = "macos")))]
mod tests {
    use super::{recognize_png, recognize_text};
    use std::path::{Path, PathBuf};

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ocr-two-lines.png")
    }

    const LINES: &str = "Iris copies this text\nSecond line 2048";

    #[test]
    fn the_system_ocr_reads_each_line_in_order() {
        assert_eq!(recognize_text(&fixture()).as_deref(), Ok(LINES));
    }

    /// The editor's image as shown has no file: the bytes pass through
    /// the cache, which keeps none of them. The cache follows
    /// IRIS_HOME, which other tests set, so this one runs serially in
    /// a home of its own.
    #[test]
    #[serial_test::serial]
    fn the_system_ocr_reads_an_image_no_file_holds() {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::dirs::HOME_ENV, home.path());
        let png = std::fs::read(fixture()).unwrap();
        assert_eq!(recognize_png(&png).as_deref(), Ok(LINES));
        let ours = format!("ocr-{}-", std::process::id());
        let left: Vec<_> = std::fs::read_dir(home.path().join("cache"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .filter(|n| n.to_string_lossy().starts_with(&ours))
            .collect();
        assert!(left.is_empty(), "the cache still holds {left:?}");
    }

    #[test]
    fn an_image_without_text_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let blank = dir.path().join("blank.png");
        image::GrayImage::from_pixel(320, 120, image::Luma([255]))
            .save(&blank)
            .unwrap();
        assert_eq!(
            recognize_text(&blank),
            Err("no text recognized in this image".to_string())
        );
    }

    #[test]
    fn a_file_that_is_not_an_image_reports_the_recognition_failure() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.png");
        std::fs::write(&file, b"plain text, not a PNG").unwrap();
        let err = recognize_text(&file).unwrap_err();
        assert!(err.starts_with("text recognition failed: "), "{err}");
    }
}
