//! macOS: `open -R` shows the file selected in a Finder window. A path
//! that no longer exists opens its folder instead.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

fn open(args: &[&OsStr]) -> bool {
    Command::new("/usr/bin/open")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn reveal(path: &Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        if !open(&[OsStr::new("-R"), path.as_os_str()]) {
            if let Some(parent) = path.parent() {
                open(&[parent.as_os_str()]);
            }
        }
    });
}
