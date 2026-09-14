//! OCR: copy all recognized text in a capture to the clipboard.
//! tesseract is a runtime dependency discovered on PATH; its absence is
//! an honest error naming the install command.

use std::path::Path;

/// Recognize text in the image at `path`. Shared by the editor's
/// Copy text action and any future OCR surface.
pub fn recognize_text(source: &Path) -> Result<String, String> {
    if !source.exists() {
        return Err(format!("image does not exist: {}", source.display()));
    }
    let out = std::process::Command::new("tesseract")
        .arg(source)
        .arg("stdout")
        .output()
        .map_err(|e| {
            format!("tesseract failed to start (install: sudo apt install tesseract-ocr): {e}")
        })?;
    if !out.status.success() {
        return Err(format!(
            "tesseract exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return Err("no text recognized in this image".to_string());
    }
    Ok(text)
}
