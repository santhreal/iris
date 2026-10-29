//! OCR: copy all recognized text in a capture to the clipboard. macOS
//! recognizes text with the Vision framework. Linux and Windows run
//! tesseract, a runtime dependency resolved by `tools`; its absence is
//! an error that names the install command.

use std::path::Path;

/// Recognize text in the image at `path`. Shared by the editor's
/// Copy Text action and the toast's Copy Text menu row.
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

#[cfg(target_os = "macos")]
fn recognize(source: &Path) -> Result<String, String> {
    crate::sys::vision::recognize_text(source)
}

#[cfg(not(target_os = "macos"))]
fn recognize(source: &Path) -> Result<String, String> {
    use crate::tools::Tool;
    let out = Tool::Tesseract
        .command()
        .arg(source)
        .arg("stdout")
        .output()
        .map_err(|e| Tool::Tesseract.spawn_error(&e))?;
    if !out.status.success() {
        return Err(format!(
            "tesseract exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// WHY: the class closed here is "Copy Text depends on a tool the macOS
// app does not ship": macOS reads text through Vision, so a packaged
// app with nothing else installed copies text. These run on the macOS
// CI runner, which has no tesseract. Not covered: recognition of
// anything harder than clean rendered text, and Linux and Windows,
// where tesseract does the recognition.
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::recognize_text;
    use std::path::Path;

    #[test]
    fn vision_reads_each_line_in_order() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ocr-two-lines.png");
        assert_eq!(
            recognize_text(&fixture).as_deref(),
            Ok("Iris copies this text\nSecond line 2048")
        );
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
    fn a_file_that_is_not_an_image_reports_the_vision_failure() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.png");
        std::fs::write(&file, b"plain text, not a PNG").unwrap();
        let err = recognize_text(&file).unwrap_err();
        assert!(err.starts_with("text recognition failed: "), "{err}");
    }
}
