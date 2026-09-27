//! The system-tray icon behind one facade.
//!
//! `MENU` is the tray menu on every platform: each row is a label and
//! the `Command` it pushes onto the daemon channel. Linux draws it
//! through the freedesktop StatusNotifierItem protocol via `ksni`,
//! Windows through `Shell_NotifyIcon`, macOS through `NSStatusItem`.
//! A platform module draws the rows from `rows` and reports a click
//! with `pick`.

use std::sync::OnceLock;

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

#[cfg(test)]
mod tests;

/// One row of the tray menu.
enum Row {
    Item {
        label: &'static str,
        command: Command,
    },
    Separator,
}

/// The tray menu, top to bottom.
const MENU: &[Row] = &[
    Row::Item {
        label: "Capture",
        command: Command::Capture,
    },
    Row::Item {
        label: "Record window",
        command: Command::RecordToggle,
    },
    Row::Item {
        label: "Library",
        command: Command::Library,
    },
    Row::Item {
        label: "Settings",
        command: Command::Settings,
    },
    Row::Separator,
    Row::Item {
        label: "Quit",
        command: Command::Quit,
    },
];

/// The daemon channel. A Win32 window procedure and an Objective-C
/// action are plain functions that cannot capture it.
static TX: OnceLock<UnboundedSender<Command>> = OnceLock::new();

/// Every row with its id: the row's index plus one. Win32 menus and
/// AppKit tags use 0 for "no item".
fn rows() -> impl Iterator<Item = (usize, &'static Row)> {
    MENU.iter().enumerate().map(|(i, row)| (i + 1, row))
}

/// The command behind row `id`. `None` for 0, a separator, or an id
/// past the end.
fn command(id: usize) -> Option<Command> {
    match MENU.get(id.checked_sub(1)?)? {
        Row::Item { command, .. } => Some(command.clone()),
        Row::Separator => None,
    }
}

/// Push the command behind row `id` onto the daemon channel.
fn pick(id: usize) {
    if let (Some(cmd), Some(tx)) = (command(id), TX.get()) {
        let _ = tx.unbounded_send(cmd);
    }
}

/// Spawn the tray. `tx` receives the menu's command. On platforms
/// without an implementation this is a no-op: the daemon still answers
/// CLI forwards and hotkeys.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub fn spawn(tx: UnboundedSender<Command>) {
    let _ = TX.set(tx);
    imp::spawn();
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn spawn(_tx: UnboundedSender<Command>) {}
