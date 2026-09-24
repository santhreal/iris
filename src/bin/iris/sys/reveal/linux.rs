//! Linux: `org.freedesktop.FileManager1.ShowItems` opens the folder with
//! the file selected (Nautilus, Dolphin, Nemo, Thunar, Caja). A session
//! without that service opens the folder through `xdg-open`.

use std::path::Path;
use std::process::{Command, Stdio};

pub fn reveal(path: &Path) {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let uri = format!(
        "file://{}",
        iris_lib::dragcopy::uri_encode_path(&abs.to_string_lossy())
    );
    let parent = abs.parent().unwrap_or(&abs).to_path_buf();
    std::thread::spawn(move || {
        // --print-reply makes dbus-send wait for the reply: without it
        // the call is fire-and-forget and exits 0 even when no file
        // manager owns the name, so the fallback would never run.
        let shown = Command::new("dbus-send")
            .args([
                "--session",
                "--print-reply",
                "--reply-timeout=5000",
                "--dest=org.freedesktop.FileManager1",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !shown {
            let _ = Command::new("xdg-open")
                .arg(&parent)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    });
}
