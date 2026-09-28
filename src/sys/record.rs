//! The live recording source for each platform. On Linux the source
//! grabs frames itself (X11 SHM or the Wayland screencast portal) and
//! hands them to a `recorder::Recorder`, which streams them through
//! `encoder` segments framed by `mkv`. On Windows and macOS, `desktop`
//! runs ffmpeg's own screen capture per segment.

#[cfg(target_os = "linux")]
pub mod encoder;
#[cfg(target_os = "linux")]
pub mod mkv;
#[cfg(target_os = "linux")]
pub mod recorder;
#[cfg(target_os = "linux")]
mod wake;

#[cfg(target_os = "linux")]
pub mod x11;

#[cfg(target_os = "linux")]
pub mod wayland;

#[cfg(any(windows, target_os = "macos"))]
pub mod desktop;

#[cfg(any(windows, target_os = "macos", test))]
mod devices;

/// Whether no other process holds `path` open. A Windows file stays
/// undeletable while another process holds it without delete sharing,
/// as ffmpeg holds its output; a POSIX file can be deleted open.
pub(crate) fn released(path: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Share mode 0 fails with a sharing violation while any other
        // handle is open.
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        true
    }
}

/// `path` open for writing, shared as ffmpeg's C runtime shares its
/// output: read and write, and on Windows not delete.
#[cfg(test)]
pub(crate) fn hold(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut open = std::fs::OpenOptions::new();
    open.write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        const FILE_SHARE_WRITE: u32 = 2;
        open.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    open.open(path)
}

// WHY: `released` gates sending a segment before ffmpeg exits. ffmpeg
// before 6.0 closes its output after it reports the output written;
// on Windows a join would then leave the segment file behind, unable to
// delete it. The test holds the file as ffmpeg's C runtime does, and
// checks that `released` agrees with whether the delete succeeds.
#[cfg(test)]
mod tests {
    #[test]
    fn released_means_the_file_can_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seg0.mkv");
        std::fs::write(&path, b"segment").unwrap();
        let held = super::hold(&path).unwrap();
        let released = super::released(&path);
        assert_eq!(released, std::fs::remove_file(&path).is_ok());
        drop(held);
        let _ = std::fs::write(&path, b"segment");
        assert!(super::released(&path));
        std::fs::remove_file(&path).unwrap();
    }
}
