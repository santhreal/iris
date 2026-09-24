//! OCR: copy all recognized text in a capture to the clipboard.
//! tesseract is a runtime dependency, resolved by `tools`; its absence
//! is an error that names the install command.

use std::path::Path;

use crate::tools::Tool;

/// Recognize text in the image at `path`. Shared by the editor's
/// Copy text action and the toast's Copy text menu row.
pub fn recognize_text(source: &Path) -> Result<String, String> {
    if !source.exists() {
        return Err(format!("image does not exist: {}", source.display()));
    }
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
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        return Err("no text recognized in this image".to_string());
    }
    Ok(text)
}
