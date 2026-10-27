//! Atomic destination reservation for screenshots and recordings.
//!
//! Reserving the destination with `create_new` closes the check-then-write
//! race between concurrent saves. An unused reservation removes its empty
//! file; `keep` transfers that cleanup responsibility to the output writer.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// An empty output file reserved against other writers.
///
/// Dropping this before `keep` or a successful `write_all` deletes the
/// placeholder. This covers a recording whose picker or source cannot start.
pub struct ReservedFile {
    path: PathBuf,
    file: Option<File>,
    remove_on_drop: bool,
}

/// Reserve the first available path in `paths`.
///
/// Existing files, directories, and symlinks are left untouched. Errors other
/// than an existing destination stop the search and name the failing path.
pub fn reserve(paths: impl IntoIterator<Item = PathBuf>) -> Result<ReservedFile, String> {
    let mut tried = false;
    for path in paths {
        tried = true;
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create {}: {e}", path.display())),
        };
        return Ok(ReservedFile {
            path,
            file: Some(file),
            remove_on_drop: true,
        });
    }
    Err(if tried {
        "no unused output filename is available".to_string()
    } else {
        "no output filenames were provided".to_string()
    })
}

impl ReservedFile {
    /// The reserved path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write the complete output and keep it. A failed write removes the
    /// partial file before this returns.
    pub fn write_all(mut self, bytes: &[u8]) -> Result<PathBuf, String> {
        if let Err(e) = self
            .file
            .as_mut()
            .expect("a live reservation holds its output file")
            .write_all(bytes)
        {
            return Err(format!("write {}: {e}", self.path.display()));
        }
        Ok(self.keep())
    }

    /// Keep the empty file and return its path. The caller now controls its
    /// lifetime; a recording uses this before handing the path to ffmpeg.
    pub fn keep(mut self) -> PathBuf {
        drop(self.file.take());
        self.remove_on_drop = false;
        self.path.clone()
    }
}

impl Drop for ReservedFile {
    fn drop(&mut self) {
        if self.remove_on_drop {
            // Windows cannot delete the file until its handle is closed.
            drop(self.file.take());
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// WHY: the classes closed here are "two output writers select one unused
// path and overwrite each other" and "a failed startup leaves an empty
// reserved output". Not covered: another process deleting a kept reservation
// before the encoder opens it.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unused_reservation_is_removed_but_a_kept_one_survives() {
        let dir = tempfile::tempdir().unwrap();
        let unused = dir.path().join("unused.png");
        let reserved = reserve([unused.clone()]).unwrap();
        assert_eq!(reserved.path(), unused);
        assert!(unused.is_file());
        drop(reserved);
        assert!(!unused.exists());

        let kept = reserve([unused.clone()]).unwrap().keep();
        assert_eq!(kept, unused);
        assert_eq!(std::fs::read(&kept).unwrap(), b"");
        std::fs::write(&kept, b"encoded").unwrap();
        assert_eq!(std::fs::read(&kept).unwrap(), b"encoded");
    }

    #[test]
    fn reservation_never_truncates_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("capture.png");
        let next = dir.path().join("capture-2.png");
        std::fs::write(&existing, b"old").unwrap();

        let reserved = reserve([existing.clone(), next.clone()]).unwrap();
        assert_eq!(reserved.path(), next);
        assert_eq!(std::fs::read(&existing).unwrap(), b"old");
        let written = reserved.write_all(b"new").unwrap();
        assert_eq!(written, next);
        assert_eq!(std::fs::read(written).unwrap(), b"new");
        assert_eq!(std::fs::read(existing).unwrap(), b"old");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_failed_write_removes_the_partial_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.png");
        std::fs::write(&path, b"reserved").unwrap();
        let file = OpenOptions::new().write(true).open("/dev/full").unwrap();
        let reserved = ReservedFile {
            path: path.clone(),
            file: Some(file),
            remove_on_drop: true,
        };
        let error = reserved.write_all(b"partial").err().unwrap();
        assert!(
            error.starts_with(&format!("write {}:", path.display())),
            "{error}"
        );
        assert!(!path.exists());
    }

    #[test]
    fn an_exhausted_or_empty_candidate_list_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("capture.png");
        std::fs::write(&existing, b"old").unwrap();
        assert_eq!(
            reserve([existing]).err().as_deref(),
            Some("no unused output filename is available")
        );
        assert_eq!(
            reserve(Vec::new()).err().as_deref(),
            Some("no output filenames were provided")
        );
    }
}
