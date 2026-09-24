//! macOS: the system screenshot sound through `afplay`.

use std::path::Path;
use std::process::{Command, Stdio};

/// The sound `screencapture` plays, newest name first.
const SOUNDS: [&str; 2] = [
    "/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/system/Screen Capture.aif",
    "/System/Library/Components/CoreAudio.component/Contents/SharedSupport/SystemSounds/system/Grab.aif",
];

fn sound() -> Option<&'static str> {
    SOUNDS.into_iter().find(|p| Path::new(p).exists())
}

/// Play the shutter on a detached thread that waits on `afplay`, so the
/// player is never left a zombie.
pub fn shutter() {
    let Some(path) = sound() else {
        return;
    };
    std::thread::spawn(move || {
        let _ = Command::new("/usr/bin/afplay")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

#[cfg(test)]
mod tests {
    /// The screenshot sound ships with the OS; a release that moves it
    /// silences every capture, so the move has to fail here first.
    #[test]
    fn system_screenshot_sound_exists() {
        assert!(
            super::sound().is_some(),
            "none of {:?} exists",
            super::SOUNDS
        );
    }
}
