//! Wakes a client that waits for a starting daemon's bind.
//!
//! A bind makes the socket file, so a watch on the socket's directory
//! (`sys::watch`) reports the bind as it happens. A timed sleep alone
//! lands late on a loaded host: on a macOS CI runner a 2 ms sleep
//! returned after 8 to 18 ms. The watch reports every entry made in the
//! directory, so a wake is a cue to retry the connect, not proof of a
//! bind.

use std::path::Path;
use std::time::Duration;

use crate::sys::watch::{DirWatch, Entries};

/// A watch on the directory that holds the socket, or none where the
/// watch could not be made; `wait` is then a timed sleep.
pub(in crate::sys::ipc) struct BindWait(Option<DirWatch>);

impl BindWait {
    /// A watch that cannot be made, as when this user has used up
    /// fs.inotify.max_user_instances, leaves the client to retry its
    /// connect on a timer. The command still completes, so the failure
    /// goes to the log file and not to the command's stderr.
    pub(in crate::sys::ipc) fn new(dir: &Path) -> Self {
        match DirWatch::new(&[dir.to_path_buf()], Entries::Made) {
            Ok(watch) => Self(Some(watch)),
            Err(e) => {
                // The error states the directory.
                iris_lib::log::file_line(&format!(
                    "iris: watch the daemon's socket directory: {e}; retrying the connect every {} ms",
                    crate::sys::ipc::READY_POLL.as_millis()
                ));
                Self(None)
            }
        }
    }

    /// Return once an entry is made in the directory, or once `limit`
    /// passes.
    pub(in crate::sys::ipc) fn wait(&self, limit: Duration) {
        match &self.0 {
            Some(watch) => {
                watch.wait(Some(limit));
            }
            None => std::thread::sleep(limit),
        }
    }
}
