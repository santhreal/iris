//! Linux: one inotify instance holds a watch per directory, and an
//! eventfd is the waker. A wait polls both.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::unix::{c_path, owned, timespec};
use super::{Entries, Woke};

pub(super) struct Watch {
    inotify: OwnedFd,
    wake: Arc<OwnedFd>,
}

/// The eventfd a wait polls beside the inotify instance. It is never
/// read, so once written it stays readable.
#[derive(Clone)]
pub(super) struct Waker(Arc<OwnedFd>);

impl Watch {
    pub(super) fn new(dirs: &[PathBuf], entries: Entries) -> io::Result<Self> {
        // SAFETY: plain syscalls on descriptors this function owns.
        let inotify = owned(unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) })?;
        let kinds = match entries {
            Entries::Made => libc::IN_CREATE | libc::IN_MOVED_TO,
            Entries::Removed => {
                libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_DELETE_SELF | libc::IN_MOVE_SELF
            }
        };
        for dir in dirs {
            let path = c_path(dir)?;
            // SAFETY: a descriptor this function owns and a NUL-terminated path.
            let added = unsafe {
                libc::inotify_add_watch(
                    inotify.as_raw_fd(),
                    path.as_ptr(),
                    kinds | libc::IN_ONLYDIR,
                )
            };
            if added < 0 {
                let e = io::Error::last_os_error();
                if !matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) {
                    return Err(io::Error::new(e.kind(), format!("{}: {e}", dir.display())));
                }
            }
        }
        // SAFETY: a plain syscall; the descriptor goes straight to an owner.
        let wake = owned(unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) })?;
        Ok(Self {
            inotify,
            wake: Arc::new(wake),
        })
    }

    pub(super) fn waker(&self) -> Waker {
        Waker(Arc::clone(&self.wake))
    }

    pub(super) fn wait(&self, limit: Option<Duration>) -> Woke {
        let mut fds = [self.wake.as_raw_fd(), self.inotify.as_raw_fd()].map(|fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        });
        let ts = limit.map(timespec);
        let ts = ts.as_ref().map_or(std::ptr::null(), std::ptr::from_ref);
        // SAFETY: two valid pollfds. A null timeout waits with no limit
        // and a null sigmask leaves signals as they are.
        unsafe {
            libc::ppoll(
                fds.as_mut_ptr(),
                fds.len() as libc::nfds_t,
                ts,
                std::ptr::null(),
            )
        };
        if fds[0].revents & libc::POLLIN != 0 {
            return Woke::Woken;
        }
        if fds[1].revents & libc::POLLIN == 0 {
            return Woke::Limit;
        }
        // Drain the queued events, so the next wait blocks until a new one.
        let mut events = [0u8; 4096];
        // SAFETY: reads into a buffer of the stated length on a
        // non-blocking descriptor; it stops at EAGAIN.
        while unsafe {
            libc::read(
                self.inotify.as_raw_fd(),
                events.as_mut_ptr().cast(),
                events.len(),
            )
        } > 0
        {}
        Woke::Changed
    }
}

impl Waker {
    pub(super) fn wake(&self) {
        let one = 1u64.to_ne_bytes();
        // SAFETY: an 8-byte write from a live buffer to an eventfd this
        // handle keeps open. A counter at its maximum refuses the write
        // with EAGAIN and is readable already.
        unsafe { libc::write(self.0.as_raw_fd(), one.as_ptr().cast(), one.len()) };
    }
}
