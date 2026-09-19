//! Wayland still capture via the xdg-desktop-portal Screenshot interface.
//!
//! The portal is the only sanctioned capture path on Wayland compositors
//! without wlr-specific protocols; it works on Hyprland, KDE, and GNOME.
//! Failures name the missing piece (no WAYLAND_DISPLAY, portal not
//! running, user denied) instead of falling back to anything silent.

use super::{CaptureBackend, Frame};

pub struct WaylandBackend;

impl WaylandBackend {
    pub fn new() -> Result<Self, String> {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(
                "WAYLAND_DISPLAY is not set; the Wayland backend needs a Wayland session"
                    .to_string(),
            );
        }
        Ok(Self)
    }
}

async fn portal_screenshot() -> Result<Frame, String> {
    let request = ashpd::desktop::screenshot::Screenshot::request()
        .interactive(false)
        .send()
        .await
        .map_err(|e| {
            format!("portal Screenshot request failed (is xdg-desktop-portal running?): {e}")
        })?;
    let shot = request
        .response()
        .map_err(|e| format!("portal Screenshot denied or failed: {e}"))?;
    let uri = shot.uri().as_str();
    let encoded = uri
        .strip_prefix("file://")
        .ok_or_else(|| format!("portal returned a non-file uri: {uri}"))?;
    let path = percent_decode(encoded);
    let img = image::open(&path)
        .map_err(|e| format!("read portal screenshot {}: {e}", path.display()))?
        .to_rgba8();
    // The portal's file is ours once delivered: leave no per-capture
    // litter in the runtime dir.
    let _ = std::fs::remove_file(&path);
    let (width, height) = img.dimensions();
    Ok(Frame {
        width,
        height,
        rgba: img.into_raw(),
    })
}

impl CaptureBackend for WaylandBackend {
    fn grab_screen(&self) -> Result<Frame, String> {
        futures::executor::block_on(portal_screenshot())
    }
}

/// Percent-decode a file-URI path component (UTF-8).
fn percent_decode(encoded: &str) -> std::path::PathBuf {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &encoded[i + 1..i + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    std::path::PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

// WHY: the class closed here is "portal URIs decode wrong": a path with
// spaces or UTF-8 arrives percent-encoded and a naive strip of file://
// opens a file that does not exist. Not covered: the portal round-trip
// itself, which needs a live session.
#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(
            percent_decode("/home/u/my%20shot%20%231.png"),
            std::path::PathBuf::from("/home/u/my shot #1.png")
        );
    }

    #[test]
    fn decodes_utf8_sequences() {
        assert_eq!(
            percent_decode("/tmp/%C3%A9cran.png"),
            std::path::PathBuf::from("/tmp/écran.png")
        );
    }

    #[test]
    fn leaves_plain_and_malformed_input_alone() {
        assert_eq!(
            percent_decode("/a/b.png"),
            std::path::PathBuf::from("/a/b.png")
        );
        // Trailing % and non-hex escapes pass through literally.
        assert_eq!(
            percent_decode("/a/100%.png"),
            std::path::PathBuf::from("/a/100%.png")
        );
        assert_eq!(
            percent_decode("/a/%zz.png"),
            std::path::PathBuf::from("/a/%zz.png")
        );
    }
}
