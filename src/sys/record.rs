//! The live recording source for each platform. On Linux the source
//! grabs frames itself (X11 SHM or the Wayland screencast portal) and
//! hands them to a `recorder::Recorder`, which streams them through
//! `encoder` segments framed by `mkv`. On Windows and macOS, `desktop`
//! runs ffmpeg's own screen capture per segment.

#[cfg(target_os = "linux")]
pub mod encoder;
#[cfg(target_os = "linux")]
pub mod mkv;
#[cfg(target_os = "linux")]
pub mod recorder;
#[cfg(target_os = "linux")]
mod wake;

#[cfg(target_os = "linux")]
pub mod x11;

#[cfg(target_os = "linux")]
pub mod wayland;

#[cfg(any(windows, target_os = "macos"))]
pub mod desktop;

#[cfg(any(windows, target_os = "macos", test))]
mod devices;
