//! Linux: the freedesktop `camera-shutter` sound through the first
//! player that plays it.

use std::process::{Command, Stdio};

const SOUND: &str = "/usr/share/sounds/freedesktop/stereo/camera-shutter.oga";

/// Play the shutter on a detached thread. The thread waits on each
/// player, so none is left a zombie, and a player that exits nonzero
/// (no sound server, no such file) falls through to the next; the last
/// resolves the sound by theme name instead of by path.
pub fn shutter() {
    std::thread::spawn(|| {
        let attempts: [(&str, &[&str]); 3] = [
            ("pw-play", &[SOUND]),
            ("paplay", &[SOUND]),
            ("canberra-gtk-play", &["-i", "camera-shutter"]),
        ];
        for (bin, args) in attempts {
            let played = Command::new(bin)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if played {
                return;
            }
        }
    });
}
