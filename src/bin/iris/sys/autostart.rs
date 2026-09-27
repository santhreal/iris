//! Start at login: this account's OS entry that runs `iris --daemon`
//! when the account logs in. The entry is the one record of the
//! setting: Settings reads it when it opens and writes it on Save.
//!
//! - Linux: the XDG autostart entry `iris-autostart.desktop` in
//!   `$XDG_CONFIG_HOME/autostart`. It overrides the entry of that name
//!   a deb or rpm installs in `/etc/xdg/autostart`, so off is an entry
//!   with `Hidden=true` there.
//! - macOS: the LaunchAgent `~/Library/LaunchAgents/dev.iris.app.plist`.
//! - Windows: the `iris` value of
//!   `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, which the
//!   installer writes on a first installation.
//!
//! `enabled` is true only for an entry that starts this iris: one that
//! names another copy of iris.exe or the iris binary reads as off, and
//! Save points it at this one.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{enabled, set};
#[cfg(target_os = "macos")]
pub use macos::{enabled, set};
#[cfg(windows)]
pub use windows::{enabled, set};
