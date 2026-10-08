//! The system-tray icon behind one facade.
//!
//! `MENU` is the tray menu on every platform: each row is a label and
//! the `Command` it pushes onto the daemon channel. Linux draws it
//! through the freedesktop StatusNotifierItem protocol via `ksni`,
//! Windows through `Shell_NotifyIcon`, macOS through `NSStatusItem`.
//! A platform module draws the rows from `drawn` and reports a click
//! with `pick`. The install row is drawn only while the daemon has an
//! update to offer (`set_offer`).

use std::borrow::Cow;
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
    /// `Install iris {version}` while an update is offered, sending
    /// `Command::InstallUpdate`; not drawn otherwise.
    Update,
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
    Row::Update,
    Row::Separator,
    Row::Item {
        label: "Quit",
        command: Command::Quit,
    },
];

/// The install row's label before the version it offers.
const UPDATE_LABEL: &str = "Install iris";

/// The version the install row offers; `None` hides the row.
static OFFER: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);

/// One row as a platform draws it now.
enum Drawn {
    Item(Cow<'static, str>),
    Separator,
}

/// The daemon channel. A Win32 window procedure and an Objective-C
/// action are plain functions that cannot capture it.
static TX: OnceLock<UnboundedSender<Command>> = OnceLock::new();

/// Every row with its id: the row's index plus one. Win32 menus and
/// AppKit tags use 0 for "no item".
fn rows() -> impl Iterator<Item = (usize, &'static Row)> {
    MENU.iter().enumerate().map(|(i, row)| (i + 1, row))
}

/// The rows to draw now, with their ids: every row of `MENU` but the
/// install row while no update is offered.
fn drawn() -> Vec<(usize, Drawn)> {
    let offer = OFFER.lock().clone();
    rows()
        .filter_map(|(id, row)| {
            let drawn = match row {
                Row::Item { label, .. } => Drawn::Item(Cow::Borrowed(*label)),
                Row::Update => {
                    Drawn::Item(Cow::Owned(format!("{UPDATE_LABEL} {}", offer.as_ref()?)))
                }
                Row::Separator => Drawn::Separator,
            };
            Some((id, drawn))
        })
        .collect()
}

/// The command behind row `id`. `None` for 0, a separator, or an id
/// past the end.
fn command(id: usize) -> Option<Command> {
    match MENU.get(id.checked_sub(1)?)? {
        Row::Item { command, .. } => Some(command.clone()),
        Row::Update => Some(Command::InstallUpdate),
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

/// Show the install row for `version`, or hide it for `None`. Call it
/// on the daemon's main thread: macOS draws the menu there.
pub fn set_offer(version: Option<&semver::Version>) {
    let version = version.map(ToString::to_string);
    {
        let mut offer = OFFER.lock();
        if *offer == version {
            return;
        }
        *offer = version;
    }
    // Windows draws the menu on each click and reads `drawn` then.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    imp::redraw();
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn spawn(_tx: UnboundedSender<Command>) {}
