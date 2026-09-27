//! The clipboard: one `arboard::Clipboard` for the life of the process.
//!
//! On X11 the selection is served from the instance that set it, so the
//! process keeps one instead of opening one per copy. On Wayland,
//! arboard sets the selection through the compositor's data-control
//! protocol (`ext-data-control-v1` or `wlr-data-control-unstable-v1`),
//! which needs no focused window, and uses the X server in `DISPLAY`
//! (XWayland) on a compositor without one. The instance opens on the
//! first copy: a daemon that never copies connects to neither.
//!
//! Each call waits on the X server or the compositor, and the first
//! can start an on-demand XWayland: call them off the UI thread.

use std::borrow::Cow;
use std::sync::LazyLock;

use parking_lot::Mutex;

/// None when no clipboard could be opened: a capture still saves.
static CLIPBOARD: LazyLock<Option<Mutex<arboard::Clipboard>>> = LazyLock::new(|| {
    arboard::Clipboard::new()
        .map(Mutex::new)
        .map_err(|e| crate::ilog!("iris: clipboard unavailable: {e}"))
        .ok()
});

/// Run `f` on the process's clipboard; `what` prefixes its error.
pub(crate) fn with(
    what: &str,
    f: impl FnOnce(&mut arboard::Clipboard) -> Result<(), arboard::Error>,
) -> Result<(), String> {
    let clipboard = CLIPBOARD.as_ref().ok_or("clipboard unavailable")?;
    f(&mut clipboard.lock()).map_err(|e| format!("{what}: {e}"))
}

/// Put RGBA pixels on the clipboard. On Linux arboard offers them as
/// `image/png`.
pub fn set_image(img: &image::RgbaImage) -> Result<(), String> {
    with("set clipboard image", |c| {
        c.set_image(arboard::ImageData {
            width: img.width() as usize,
            height: img.height() as usize,
            bytes: Cow::Borrowed(img.as_raw()),
        })
    })
}

pub fn set_text(text: &str) -> Result<(), String> {
    with("set clipboard text", |c| c.set_text(text))
}
