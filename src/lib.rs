//! iris backend library: capture, recording, library, OCR, config,
//! clipboard and file drag-out. The UI lives in the `gpui` binary.

pub mod capture;
pub mod config;
pub mod dirs;
pub mod dragcopy;
pub mod history;
pub mod library;
pub mod log;
#[cfg(target_os = "macos")]
pub mod objc;
pub mod ocr;
pub mod par;
pub mod pixel;
pub mod record;
pub mod session;
pub mod thumb;
pub mod time;
pub mod tools;

pub use config::Config;
