//! Show a file in the platform file manager with the file selected.
//! `reveal` returns at once: the file manager round trip runs on a
//! detached thread that waits on any helper process, so no child is
//! left a zombie and the UI thread never blocks on a slow launch.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::reveal;
#[cfg(target_os = "macos")]
pub use macos::reveal;
#[cfg(windows)]
pub use windows::reveal;
