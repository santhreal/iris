//! The system-tray icon behind one facade.
//!
//! The tray owns a menu of `Command`s (capture, record, library,
//! settings, quit) and pushes the chosen one onto the daemon channel.
//! Linux uses the freedesktop StatusNotifierItem protocol via `ksni`;
//! Windows uses `Shell_NotifyIcon`, macOS `NSStatusItem`. The per-OS
//! module owns the detail; this facade fixes the call shape.

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

/// Spawn the tray on its own thread. `tx` receives the menu's command.
/// On platforms without an implementation yet this is a no-op: the
/// daemon still answers CLI forwards and hotkeys.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub fn spawn(tx: UnboundedSender<Command>) {
    imp::spawn(tx);
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn spawn(_tx: UnboundedSender<Command>) {}
