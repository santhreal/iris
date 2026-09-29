//! macOS: one kqueue holds a vnode filter on each directory, open for
//! events only, and a user filter that is the waker. A write to a
//! directory is an entry made or removed in it.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::unix::{c_path, owned, timespec};
use super::{Entries, Woke};

pub(super) struct Watch {
    queue: Arc<OwnedFd>,
    /// The directories' descriptors: a vnode filter ends when its
    /// descriptor closes.
    _dirs: Vec<OwnedFd>,
}

/// The kqueue, to trigger its user filter. The filter has no EV_CLEAR,
/// so once triggered it stays active.
#[derive(Clone)]
pub(super) struct Waker(Arc<OwnedFd>);

/// The ident of the waker's user filter. A filter is keyed by its ident
/// and its filter type together, so this cannot meet a directory's
/// descriptor.
const WAKE: libc::uintptr_t = 0;

fn event(ident: libc::uintptr_t, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
    libc::kevent {
        ident,
        filter,
        flags,
        fflags,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}

/// Register `change` on `queue`.
fn apply(queue: &OwnedFd, change: &libc::kevent) -> io::Result<()> {
    // SAFETY: registers one change and waits for no events.
    let applied = unsafe {
        libc::kevent(
            queue.as_raw_fd(),
            change,
            1,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    };
    if applied < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl Watch {
    pub(super) fn new(dirs: &[PathBuf], entries: Entries) -> io::Result<Self> {
        // SAFETY: a plain syscall; the descriptor goes straight to an owner.
        let queue = owned(unsafe { libc::kqueue() })?;
        let kinds = match entries {
            Entries::Made => libc::NOTE_WRITE,
            Entries::Removed => libc::NOTE_WRITE | libc::NOTE_DELETE | libc::NOTE_RENAME,
        };
        let mut open = Vec::with_capacity(dirs.len());
        for dir in dirs {
            let path = c_path(dir)?;
            let flags = libc::O_EVTONLY | libc::O_CLOEXEC | libc::O_DIRECTORY;
            // SAFETY: a NUL-terminated path; the descriptor goes straight
            // to an owner.
            let fd = match owned(unsafe { libc::open(path.as_ptr(), flags) }) {
                Ok(fd) => fd,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) =>
                {
                    continue
                }
                Err(e) => return Err(io::Error::new(e.kind(), format!("{}: {e}", dir.display()))),
            };
            let ident = fd.as_raw_fd() as libc::uintptr_t;
            apply(
                &queue,
                &event(
                    ident,
                    libc::EVFILT_VNODE,
                    libc::EV_ADD | libc::EV_CLEAR,
                    kinds,
                ),
            )?;
            open.push(fd);
        }
        apply(&queue, &event(WAKE, libc::EVFILT_USER, libc::EV_ADD, 0))?;
        Ok(Self {
            queue: Arc::new(queue),
            _dirs: open,
        })
    }

    pub(super) fn waker(&self) -> Waker {
        Waker(Arc::clone(&self.queue))
    }

    pub(super) fn wait(&self, limit: Option<Duration>) -> Woke {
        let ts = limit.map(timespec);
        let ts = ts.as_ref().map_or(std::ptr::null(), std::ptr::from_ref);
        // SAFETY: kevent is plain data; zero is a valid value.
        let mut events: [libc::kevent; 8] = unsafe { std::mem::zeroed() };
        // SAFETY: room for the stated number of events. EV_CLEAR resets
        // a vnode filter once it is reported, so the next wait blocks
        // until a new write; a null timeout waits with no limit.
        let n = unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                std::ptr::null(),
                0,
                events.as_mut_ptr(),
                events.len() as libc::c_int,
                ts,
            )
        };
        let got = &events[..usize::try_from(n).unwrap_or(0)];
        // kevent is packed: the field is copied out, never borrowed.
        let woken = got.iter().any(|e| {
            let filter = e.filter;
            filter == libc::EVFILT_USER
        });
        if woken {
            Woke::Woken
        } else if got.is_empty() {
            Woke::Limit
        } else {
            Woke::Changed
        }
    }
}

impl Waker {
    pub(super) fn wake(&self) {
        let trigger = event(WAKE, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER);
        // The trigger goes to a kqueue this handle keeps open, on the
        // filter the watch registered: it has no failure to report.
        let _ = apply(&self.0, &trigger);
    }
}
