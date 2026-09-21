//! iris backend library: capture, recording, library, OCR, config,
//! clipboard and file drag-out. The UI lives in the `gpui` binary.

pub mod capture;
pub mod config;
pub mod dragcopy;
pub mod history;
pub mod library;
pub mod log;
pub mod ocr;
pub mod par;
pub mod thumb;
pub mod record;

pub use config::Config;
