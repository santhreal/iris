//! Atomic destination reservation for screenshots and recordings, and
//! whole-file replacement for the files iris rewrites.
//!
//! Reserving the destination with `create_new` closes the check-then-write
//! race between concurrent saves. An unused reservation removes its empty
//! file; `keep` transfers that cleanup responsibility to the output writer.
//! `replace` writes a sibling and renames it over the file, so no reader
//! and no failure sees part of a file.

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

/// Replace the file at `path` with `bytes` in one rename: a reader, an
/// exit of iris, or a full disk finds the old file or the new one, never
/// part of either. The bytes go to a hidden sibling first, which a failed
/// write deletes. A symlink at `path` keeps naming the file it names, and
/// that file is replaced; the replacement takes the old file's
/// permissions. A missing file is created.
pub fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    replace_with(path, |file| file.write_all(bytes))
}

/// `replace`, with `write` filling the staged sibling.
fn replace_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = target.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "the path names no file")
    })?;
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staged = std::ffi::OsString::from(".");
    staged.push(name);
    staged.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let staged = dir.join(staged);
    let permissions = std::fs::metadata(&target).ok().map(|m| m.permissions());
    let swapped = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .and_then(|mut file| {
            write(&mut file)?;
            if let Some(permissions) = permissions {
                file.set_permissions(permissions)?;
            }
            // Windows renames no file that is still open.
            drop(file);
            std::fs::rename(&staged, &target)
        });
    if swapped.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    swapped
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

// WHY: the class closed here is "a rewrite of a file iris keeps leaves
// part of a file": an editor save, library.json, or config.toml written
// in place is empty or cut short to a reader during the write, and for
// good after a full disk or an exit midway; library.json then reads as
// corrupt and the library starts empty. Also closed: a replacement that
// leaves its staged sibling, turns a symlink into a file, or widens the
// old file's permissions. Not covered: a power cut before the file
// system writes the new file's blocks out, which needs an fsync this does
// not run.
#[cfg(test)]
mod replace_tests {
    use super::*;

    /// The names in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn replace_swaps_the_whole_file_and_leaves_no_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.json");
        replace(&path, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        replace(&path, b"second, longer").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second, longer");
        replace(&path, b"3").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"3");
        assert_eq!(names(dir.path()), ["library.json"]);
    }

    #[test]
    fn a_failed_replace_keeps_the_old_file_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, b"the old capture").unwrap();
        let failed = replace_with(&path, |file| {
            file.write_all(b"half")?;
            Err(std::io::Error::other("no space left on device"))
        });
        assert_eq!(failed.unwrap_err().to_string(), "no space left on device");
        assert_eq!(std::fs::read(&path).unwrap(), b"the old capture");
        assert_eq!(names(dir.path()), ["shot.png"]);
    }

    #[test]
    fn replacing_a_directory_fails_and_leaves_no_sibling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("taken");
        std::fs::create_dir(&path).unwrap();
        assert!(replace(&path, b"bytes").is_err());
        assert!(path.is_dir());
        assert_eq!(names(dir.path()), ["taken"]);
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_keeps_naming_the_file_it_names_with_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.png");
        let link = dir.path().join("link.png");
        std::fs::write(&real, b"old").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        replace(&link, b"new").unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&real).unwrap(), b"new");
        let mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(names(dir.path()), ["link.png", "real.png"]);
    }
}
