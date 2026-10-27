//! The iris binary this process runs, and the AppImage that contains it.
//!
//! The backend library resolves the installed executable after a package
//! upgrade. This module adds the AppImage path used by updates and
//! autostart entries.

#[cfg(target_os = "linux")]
mod appimage;
#[cfg(target_os = "linux")]
pub use appimage::appimage;

pub use iris_lib::sys::exe::this;
