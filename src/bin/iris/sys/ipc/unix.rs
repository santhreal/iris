//! Unix: the daemon's socket file.
//!
//! The socket's directory is its access control. A connect needs search
//! permission on the directory that holds the socket, and the directory
//! holding `iris.sock` admits only this user, on Linux and macOS alike.
//! The socket file's own mode is not used: setting it takes an fchmod on
//! the socket before bind, which macOS rejects with EINVAL.

use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

use interprocess::local_socket::{prelude::*, GenericFilePath, ListenerOptions, Name, ToFsName};

/// `iris.sock` in the socket directory of `$XDG_RUNTIME_DIR`, or of the
/// temp dir when that is unset.
pub(super) fn socket_name() -> Result<Name<'static>, String> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|v| !v.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from);
    let path = socket_dir(&base)?.join("iris.sock");
    path.clone()
        .into_os_string()
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| format!("iris: socket path {}: {e}", path.display()))
}

/// Bind the socket. `reclaim_name` and `try_overwrite` let a fresh
/// daemon take over the socket file a SIGKILLed predecessor left behind.
pub(super) fn bind(name: Name<'static>) -> Result<LocalSocketListener, String> {
    ListenerOptions::new()
        .name(name)
        .reclaim_name(true)
        .try_overwrite(true)
        .create_sync()
        .map_err(|e| format!("bind local socket: {e}"))
}

/// Connect to the daemon's socket. Only this user can enter the
/// directory that holds it, so the daemon that answers is this user's.
pub(super) fn connect(name: Name<'_>) -> io::Result<LocalSocketStream> {
    LocalSocketStream::connect(name)
}

/// The directory that holds the socket: `base` itself when only this
/// user can enter it, as with a session's `$XDG_RUNTIME_DIR` or the
/// macOS per-user temp dir; otherwise `base/iris-<uid>`, made with mode
/// 0700. A directory of that name that is a symlink, that another user
/// owns, or that others may enter fails: whoever controls it could read
/// the paths a client forwards or answer in the daemon's place.
fn socket_dir(base: &Path) -> Result<PathBuf, String> {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if private(base, uid) {
        return Ok(base.to_path_buf());
    }
    let dir = base.join(format!("iris-{uid}"));
    if let Err(e) = std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        if e.kind() != io::ErrorKind::AlreadyExists {
            return Err(format!(
                "iris: create the socket directory {}: {e}",
                dir.display()
            ));
        }
    }
    if private(&dir, uid) {
        Ok(dir)
    } else {
        Err(format!(
            "iris: {} is not a directory of uid {uid} with mode 0700; remove it",
            dir.display()
        ))
    }
}

/// `dir` is a directory, not a symlink to one, that `uid` owns and no
/// other user may read, write, or enter.
fn private(dir: &Path, uid: u32) -> bool {
    std::fs::symlink_metadata(dir)
        .is_ok_and(|m| m.is_dir() && m.uid() == uid && m.mode() & 0o077 == 0)
}

/// A socket name no daemon uses, in a fresh private directory that
/// lives as long as the returned guard.
#[cfg(test)]
pub(super) fn scratch_name() -> (Name<'static>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = socket_dir(dir.path()).unwrap().join("iris.sock");
    let name = path.into_os_string().to_fs_name::<GenericFilePath>().unwrap();
    (name, dir)
}

// WHY: the class closed here is "a socket another user can reach". The
// directory is the socket's only access control on every Unix, so each
// directory `socket_dir` returns must admit only this user, and one it
// cannot vouch for fails instead of holding the socket. Not covered: a
// directory another user owns, which a test without root cannot make.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn uid() -> u32 {
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    fn chmod(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn a_private_base_holds_the_socket_itself() {
        let base = tempfile::tempdir().unwrap();
        chmod(base.path(), 0o700);
        assert_eq!(socket_dir(base.path()).unwrap(), base.path());
    }

    #[test]
    fn a_shared_base_gets_a_directory_only_this_user_can_enter() {
        // World-writable and sticky as /tmp, then readable by all, by
        // the group, and enterable by others.
        for mode in [0o1777, 0o755, 0o750, 0o701] {
            let base = tempfile::tempdir().unwrap();
            chmod(base.path(), mode);
            let dir = socket_dir(base.path()).unwrap();
            assert_eq!(dir, base.path().join(format!("iris-{}", uid())), "{mode:o}");
            let meta = std::fs::symlink_metadata(&dir).unwrap();
            assert!(meta.is_dir(), "{mode:o}");
            assert_eq!(meta.uid(), uid(), "{mode:o}");
            assert_eq!(meta.mode() & 0o7777, 0o700, "{mode:o}");
            // The next client and the daemon find the same directory.
            assert_eq!(socket_dir(base.path()).unwrap(), dir, "{mode:o}");
            chmod(base.path(), 0o700);
        }
    }

    #[test]
    fn a_directory_it_cannot_vouch_for_fails() {
        fn check(what: &str, make: impl Fn(&Path)) {
            let base = tempfile::tempdir().unwrap();
            chmod(base.path(), 0o755);
            make(&base.path().join(format!("iris-{}", uid())));
            let got = socket_dir(base.path());
            chmod(base.path(), 0o700);
            assert!(got.is_err(), "iris-<uid> is {what}: {got:?}");
        }
        check("mode 0755", |dir| {
            std::fs::DirBuilder::new().mode(0o755).create(dir).unwrap();
            chmod(dir, 0o755);
        });
        check("a symlink to a private directory", |dir| {
            let target = dir.with_file_name("elsewhere");
            std::fs::DirBuilder::new().mode(0o700).create(&target).unwrap();
            std::os::unix::fs::symlink(&target, dir).unwrap();
        });
        check("a regular file", |dir| std::fs::write(dir, b"").unwrap());
    }

    #[test]
    fn a_base_that_does_not_exist_fails() {
        let base = tempfile::tempdir().unwrap();
        let missing = base.path().join("missing");
        assert!(socket_dir(&missing).is_err());
        assert!(!missing.exists());
    }
}
