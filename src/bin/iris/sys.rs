//! OS-bound primitives behind one facade.
//!
//! Every subsystem that touches the platform — single-instance IPC,
//! global hotkeys, the tray, window placement, capture and record
//! routing — lives here as a free-function API. The platform is fixed
//! at compile time, so the facade is a thin `#[cfg]` re-export over
//! per-OS modules rather than a trait object: no vtable, no runtime
//! dispatch, and a missing backend is a compile error, not a silent
//! no-op.
//!
//! `ipc` is fully cross-platform (interprocess local sockets map to
//! Unix domain sockets on Unix and named pipes on Windows). `hotkeys`,
//! `install`, `record`, `tray`, and `window` have a per-OS implementation each.

pub mod ipc;

pub mod hotkeys;
pub mod install;
pub mod record;
pub mod tray;
pub mod window;
