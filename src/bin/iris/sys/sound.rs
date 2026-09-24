//! The shutter sound at the grab moment. Each OS plays its own sound
//! its own way; `shutter` returns at once and never fails, so a machine
//! with no audio output stays silent.

#[cfg(any(windows, test))]
mod click;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::shutter;
#[cfg(target_os = "macos")]
pub use macos::shutter;
#[cfg(windows)]
pub use windows::shutter;
