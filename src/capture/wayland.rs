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
        if !crate::session::wayland() {
            return Err(
                "WAYLAND_DISPLAY is not set; the Wayland backend needs a Wayland session"
                    .to_string(),
            );
        }
        Ok(Self)
    }
}

async fn portal_screenshot() -> Result<Frame, String> {
    // A cold portal backend can lose the first request: dbus activation
    // starts xdpw on demand and the frontend's dispatch can land before
    // the backend registers its objects, or a screencopy frame can
    // outlive the request on an idle output. One retry covers the cold
    // start; a user cancel is never retried.
    let mut last_err = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        let request = match ashpd::desktop::screenshot::Screenshot::request()
            .interactive(false)
            .send()
            .await
        {
            Ok(request) => request,
            Err(e) => {
                last_err = format!(
                    "portal Screenshot request failed (is xdg-desktop-portal running?): {e}"
                );
                continue;
            }
        };
        match request.response() {
            Ok(shot) => return frame_from_uri(shot.uri().as_str()),
            Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => {
                return Err("portal Screenshot cancelled".to_string());
            }
            Err(e) => {
                last_err = format!("portal Screenshot denied or failed: {e}");
            }
        }
    }
    Err(last_err)
}

fn frame_from_uri(uri: &str) -> Result<Frame, String> {
    let encoded = uri
        .strip_prefix("file://")
        .ok_or_else(|| format!("portal returned a non-file uri: {uri}"))?;
    let path = crate::dragcopy::uri_decode_path(encoded);
    let img = image::open(&path)
        .map_err(|e| format!("read portal screenshot {}: {e}", path.display()))?
        .into_rgba8();
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
