//! Windows: the synthesized shutter through `PlaySoundW`. Windows ships
//! no camera sound, so `click` renders one once into an in-memory WAV
//! that lives for the rest of the process.

use std::sync::OnceLock;

use windows_sys::Win32::Media::Audio::{
    PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT, SND_SYSTEM,
};

static WAV: OnceLock<Vec<u8>> = OnceLock::new();

/// Queue the shutter and return. `SND_ASYNC` plays it on the winmm
/// thread and a newer call cuts an older one off; `SND_SYSTEM` puts it
/// on the system-sounds volume beside the other UI sounds.
pub fn shutter() {
    let wav = WAV.get_or_init(super::click::wav);
    // SAFETY: with SND_MEMORY the first argument points at a complete
    // RIFF image, read asynchronously; the static buffer outlives the
    // playback. The module handle is unused without SND_RESOURCE.
    unsafe {
        PlaySoundW(
            wav.as_ptr().cast(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM,
        );
    }
}
