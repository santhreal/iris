//! Platform code: the screen grabs, recording sources, file drag-out,
//! display session, clock, and tool locations of each OS. Every other
//! module of the library compiles the same on Linux, Windows, and macOS.

pub mod capture;
#[cfg(target_os = "linux")]
mod clipboard;
pub mod dragcopy;
#[cfg(target_os = "macos")]
pub mod objc;
pub mod record;
pub mod session;
pub(crate) mod time;
pub(crate) mod tools;
