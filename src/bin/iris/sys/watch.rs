//! A watch on directories. [`DirWatch::wait`] returns once an entry of
//! the kind the watch selects changes in one of its directories, once
//! its [`Waker`] is called, or once a limit passes. A thread blocked in
//! a wait with no limit makes no wakeups until one of these happens.
//!
//! Linux watches with one inotify instance, macOS with one kqueue that
//! holds a vnode filter per directory, and Windows with a change
//! notification per directory. A `Changed` wait is a cue to read the
//! directories again, not a report of what changed: macOS and Windows
//! report both kinds of entry change whichever [`Entries`] selects.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
use windows as imp;

#[cfg(test)]
mod tests;

/// The entry changes a watch reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entries {
    /// An entry made in a directory or moved into it. Windows waits for
    /// the daemon's pipe without a watch, so only its tests make one.
    #[cfg_attr(windows, allow(dead_code))]
    Made,
    /// An entry removed from a directory or moved out of it, and a
    /// directory itself removed or moved.
    Removed,
}

/// What ended a wait.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Woke {
    /// An entry changed in a watched directory.
    Changed,
    /// The watch's [`Waker`] was called.
    Woken,
    /// The limit passed, or a signal interrupted the wait.
    Limit,
}

/// A watch on a set of directories.
pub struct DirWatch(imp::Watch);

/// Ends a wait on its [`DirWatch`] from another thread. Once called,
/// every later wait on the watch returns [`Woke::Woken`] at once.
#[derive(Clone)]
pub struct Waker(imp::Waker);

impl DirWatch {
    /// A watch on each of `dirs` that is a directory. A path that does
    /// not exist, or is not a directory, is skipped: it holds no entry
    /// to remove, and a caller that waits for an entry to be made
    /// creates the directory first. Fails when the OS refuses a watch,
    /// as when this user has used up `fs.inotify.max_user_instances` or
    /// `max_user_watches`, or on Windows past 63 directories.
    pub fn new(dirs: &[PathBuf], entries: Entries) -> io::Result<Self> {
        imp::Watch::new(dirs, entries).map(Self)
    }

    /// The handle that ends this watch's waits.
    pub fn waker(&self) -> Waker {
        Waker(self.0.waker())
    }

    /// Block until a watched entry changes, the waker is called, or
    /// `limit` passes; `None` waits with no limit. Changes reported
    /// before the wait that no wait has returned end it at once.
    pub fn wait(&self, limit: Option<Duration>) -> Woke {
        self.0.wait(limit)
    }
}

impl Waker {
    /// End the wait in progress on the watch, and every later one.
    pub fn wake(&self) {
        self.0.wake();
    }
}
