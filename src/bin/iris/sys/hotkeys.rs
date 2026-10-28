//! Global hotkeys behind one facade.
//!
//! The daemon registers the configured capture/record chords and pushes
//! a `Command` onto the channel when one fires. Each OS grabs keys its
//! own way: X11 key grabs or the GlobalShortcuts portal on Linux,
//! `RegisterHotKey` on Windows, Carbon hotkeys on macOS. The per-OS
//! module owns the detail; this facade fixes the call shape and holds
//! the outcome of each registration, which Home and Settings show.

use futures::channel::mpsc::UnboundedSender;
use iris_lib::config::Config;
use parking_lot::Mutex;

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

/// A configurable global chord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chord {
    Capture,
    Record,
}

impl Chord {
    pub const ALL: [Chord; 2] = [Chord::Capture, Chord::Record];

    /// The configured keys, trimmed; empty when the chord is unset.
    pub fn keys(self, cfg: &Config) -> &str {
        match self {
            Chord::Capture => cfg.capture_hotkey.trim(),
            Chord::Record => cfg.record_hotkey.trim(),
        }
    }

    /// The command a press runs.
    pub fn command(self) -> Command {
        match self {
            Chord::Capture => Command::Capture,
            Chord::Record => Command::RecordToggle,
        }
    }

    /// The portal shortcut id and the description the desktop shows.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn portal(self) -> (&'static str, &'static str) {
        match self {
            Chord::Capture => ("capture", "Capture a region"),
            Chord::Record => ("record", "Start or stop a recording"),
        }
    }

    fn slot(self) -> usize {
        match self {
            Chord::Capture => 0,
            Chord::Record => 1,
        }
    }
}

/// The outcome of registering one chord.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    /// A press runs the chord's command.
    Live,
    /// No keys are configured.
    Unset,
    /// The keys are configured and a press does nothing; the text is
    /// the reason, short enough for a caption.
    Refused(String),
}

/// Per chord: the keys the last registration ran with, and its outcome.
static STATUS: Mutex<[Option<(String, Binding)>; 2]> = Mutex::new([None, None]);
/// The daemon's command channel: `record` wakes the windows that show
/// the status.
static NOTIFY: Mutex<Option<UnboundedSender<Command>>> = Mutex::new(None);

/// Record the registration outcome of `chord` under `keys`. Called by
/// the backend after every registration attempt, the first and each
/// regrab; a refusal is also logged.
pub(crate) fn record(chord: Chord, keys: &str, binding: Binding) {
    if let Binding::Refused(why) = &binding {
        iris_lib::ilog!("iris: hotkey {keys:?} not bound: {why}");
    }
    let changed = {
        let mut status = STATUS.lock();
        let slot = &mut status[chord.slot()];
        let next = Some((keys.to_owned(), binding));
        let changed = *slot != next;
        *slot = next;
        changed
    };
    if changed {
        if let Some(tx) = NOTIFY.lock().as_ref() {
            let _ = tx.unbounded_send(Command::HotkeysBound);
        }
    }
}

/// The state of `chord` configured as `keys`: `Unset` for empty keys,
/// else the outcome the last registration recorded for these keys.
/// `None` while no registration ran for them yet: a regrab after a
/// settings save is in flight, or no daemon registered any.
pub fn binding(chord: Chord, keys: &str) -> Option<Binding> {
    if keys.trim().is_empty() {
        return Some(Binding::Unset);
    }
    STATUS.lock()[chord.slot()]
        .as_ref()
        .filter(|(k, _)| k == keys.trim())
        .map(|(_, b)| b.clone())
}

/// The configured chords with their keys and command, in registration
/// order. Every backend registers from this list.
pub(crate) fn configured(cfg: &Config) -> [(Chord, &str); 2] {
    Chord::ALL.map(|c| (c, c.keys(cfg)))
}

/// Spawn the hotkey listener. `tx` receives the command bound to each
/// chord. The chords are registered when this returns, so a press
/// right after it reaches `tx`, except through the Wayland portal,
/// which binds asynchronously. On platforms without an implementation
/// this records every chord as refused: the daemon still answers CLI
/// forwards and the tray.
pub fn spawn(tx: UnboundedSender<Command>) {
    *NOTIFY.lock() = Some(tx.clone());
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    imp::spawn(tx);
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    refuse_all("No global shortcut support on this platform");
}

/// Signal a settings change: ungrab everything, reload config, re-grab.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub fn request_regrab() {
    imp::request_regrab();
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub fn request_regrab() {
    refuse_all("No global shortcut support on this platform");
}

/// Refusal reasons shared by the backends, short enough for a caption.
pub(crate) const TAKEN: &str = "In use by another app";
pub(crate) const UNKNOWN_KEY: &str = "Not a key iris can bind";
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) const NO_KEY: &str = "Not on this keyboard";

/// Record every configured chord as refused for `why`, and every empty
/// one as unset.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn refuse_all(why: &str) {
    let cfg = Config::load();
    for (chord, keys) in configured(&cfg) {
        let binding = if keys.is_empty() {
            Binding::Unset
        } else {
            Binding::Refused(why.to_owned())
        };
        record(chord, keys, binding);
    }
}

// WHY: the class closed here is "a surface shows a shortcut that does
// nothing". Registration failures were only logged, so Home captioned a
// chord another app held, or one a Wayland session never bound, as if
// it worked. The status is keyed by the keys it was recorded for, so a
// chord changed in Settings reads as pending until its regrab lands
// instead of inheriting the old chord's outcome. Not covered: the OS
// registration calls, exercised by the startup and surfaces suites.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    #[test]
    fn status_follows_the_keys_it_was_recorded_for() {
        record(Chord::Record, "Ctrl+Alt+Q", Binding::Live);
        assert_eq!(binding(Chord::Record, "Ctrl+Alt+Q"), Some(Binding::Live));
        assert_eq!(binding(Chord::Record, " Ctrl+Alt+Q "), Some(Binding::Live));
        // A chord saved after the registration has no outcome yet.
        assert_eq!(binding(Chord::Record, "Ctrl+Alt+W"), None);
        record(
            Chord::Record,
            "Ctrl+Alt+W",
            Binding::Refused("in use by another app".into()),
        );
        assert_eq!(
            binding(Chord::Record, "Ctrl+Alt+W"),
            Some(Binding::Refused("in use by another app".into()))
        );
        assert_eq!(binding(Chord::Record, "Ctrl+Alt+Q"), None);
        // An empty chord is unset whatever was recorded.
        assert_eq!(binding(Chord::Record, "  "), Some(Binding::Unset));
    }

    #[test]
    fn every_chord_runs_its_command_from_its_own_key() {
        let cfg = Config {
            capture_hotkey: " F9 ".into(),
            record_hotkey: "Ctrl+Shift+R".into(),
            ..Config::default()
        };
        let got: Vec<String> = configured(&cfg)
            .iter()
            .map(|(c, k)| format!("{k}={:?}", c.command()))
            .collect();
        assert_eq!(got, ["F9=Capture", "Ctrl+Shift+R=RecordToggle"]);
        let slots: Vec<usize> = Chord::ALL.iter().map(|c| c.slot()).collect();
        assert_eq!(slots, [0, 1]);
    }
}
