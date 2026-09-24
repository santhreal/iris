//! Unix: a child with /dev/null for its standard streams, in a process
//! group of its own, so a signal to the caller's terminal does not
//! reach it.

use std::io;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

pub(super) fn spawn(program: &Path, args: &[&str]) -> io::Result<()> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
}
