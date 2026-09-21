//! Global hotkeys behind one facade.
//!
//! The daemon registers the configured capture/record chords and pushes
//! a `Command` onto the channel when one fires. Each OS grabs keys its
//! own way: X11 key grabs on Linux, `RegisterHotKey` on Windows, a
//! Carbon/`NSEvent` monitor on macOS. The per-OS module owns the detail;
//! this facade fixes the call shape.

use futures::channel::mpsc::UnboundedSender;

use crate::daemon::Command;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

/// Spawn the hotkey listener. `tx` receives the command bound to each
/// chord. On platforms without an implementation yet this is a no-op:
/// the daemon still answers CLI forwards and the tray.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub fn spawn(tx: UnboundedSender<Command>) {
    imp::spawn(tx);
}

/// Signal a settings change: ungrab everything, reload config, re-grab.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub fn request_regrab() {
    imp::request_regrab();
}

// A platform with no global-hotkey backend still runs the daemon and
// answers its socket; only the chords are unbound.
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn spawn(_tx: UnboundedSender<Command>) {}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn request_regrab() {}
