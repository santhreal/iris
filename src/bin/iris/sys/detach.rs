//! Starting a process that outlives this one and holds nothing of it.
//!
//! A daemon that a command line starts must not keep that command
//! line's streams: a caller that reads the command's output to its end,
//! as a script or a test harness does, would otherwise wait for the
//! daemon to exit. On Unix the child gets /dev/null for its standard
//! streams and a process group of its own, and every other descriptor
//! iris opens is close-on-exec. On Windows the child is created with no
//! console and bInheritHandles FALSE, so it receives no handle.

use std::io;
use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

/// Start `program` with `args`, detached from this process. On Windows
/// an argument that would need quotes on the command line, one that is
/// empty or holds whitespace or '"', fails with `InvalidInput`.
pub fn spawn(program: &Path, args: &[&str]) -> io::Result<()> {
    imp::spawn(program, args)
}
