//! The system-tray icon behind one facade.
//!
//! `MENU` is the tray menu on every platform: each row is a label and
//! the `Command` it pushes onto the daemon channel. Linux draws it
//! through the freedesktop StatusNotifierItem protocol via `ksni`,
//! Windows through `Shell_NotifyIcon`, macOS through `NSStatusItem`.
//! A platform module draws the rows from `drawn` and reports a click
//! with `pick`. The install row is drawn only while the daemon has an
//! update to offer (`set_offer`), the record row only while no recording
//! runs, and the stop row only while one does (`set_recording`).

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
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
        when: When,
    },
    /// `Install iris {version}` while an update is offered, sending
    /// `Command::InstallUpdate`; not drawn otherwise.
    Update,
    Separator,
}

/// When an item row is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
enum When {
    Always,
    /// While no recording runs.
    Idle,
    /// While a recording runs.
    Recording,
}

/// The tray menu, top to bottom.
const MENU: &[Row] = &[
    Row::Item {
        label: "New Screenshot",
        command: Command::Capture,
        when: When::Always,
    },
    Row::Item {
        label: "Record Window",
        command: Command::RecordToggle,
        when: When::Idle,
    },
    Row::Item {
        label: "Stop Recording",
        command: Command::RecordStop,
        when: When::Recording,
    },
    Row::Item {
        label: "Library",
        command: Command::Library,
        when: When::Always,
    },
    Row::Item {
        label: "Settings",
        command: Command::Settings,
        when: When::Always,
    },
    Row::Update,
    Row::Separator,
    Row::Item {
        label: "Quit",
        command: Command::Quit,
        when: When::Always,
    },
];

/// The install row's label before the version it offers.
const UPDATE_LABEL: &str = "Install iris";

/// The label of the row that sends `command`, matched by its `Debug`
/// name, for the tests that hold another surface to the same name.
#[cfg(test)]
pub(crate) fn label_of(command: &Command) -> Option<&'static str> {
    let name = format!("{command:?}");
    MENU.iter().find_map(|row| match row {
        Row::Item { label, command, .. } if format!("{command:?}") == name => Some(*label),
        _ => None,
    })
}

/// The version the install row offers; `None` hides the row.
static OFFER: parking_lot::Mutex<Option<String>> = parking_lot::Mutex::new(None);

/// Whether a recording runs: it selects the record or the stop row.
static RECORDING: AtomicBool = AtomicBool::new(false);

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
/// install row while no update is offered and the item rows whose
/// `When` does not hold.
fn drawn() -> Vec<(usize, Drawn)> {
    let offer = OFFER.lock().clone();
    let recording = RECORDING.load(Ordering::Acquire);
    rows()
        .filter_map(|(id, row)| {
            let drawn = match row {
                Row::Item { label, when, .. } => {
                    let shown = match when {
                        When::Always => true,
                        When::Idle => !recording,
                        When::Recording => recording,
                    };
                    shown.then_some(Drawn::Item(Cow::Borrowed(*label)))?
                }
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

/// Draw the stop row while a recording runs and the record row while
/// none does. Call it on the daemon's main thread: macOS draws the menu
/// there.
pub fn set_recording(on: bool) {
    let changed = RECORDING.swap(on, Ordering::AcqRel) != on;
    // Windows draws the menu on each click and reads `drawn` then.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if changed {
        imp::redraw();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = changed;
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn spawn(_tx: UnboundedSender<Command>) {}
