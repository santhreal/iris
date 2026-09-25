//! Wakes a client that waits for a starting daemon's bind.
//!
//! A bind makes the socket file, so a watch on the socket's directory
//! reports the bind as it happens: inotify on Linux, a kqueue vnode
//! filter on macOS. A timed sleep alone lands late on a loaded host: on
//! a macOS CI runner a 2 ms sleep returned after 8 to 18 ms. The watch
//! reports every entry made in the directory, so a wake is a cue to
//! retry the connect, not proof of a bind.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::time::Duration;

/// A watch on the directory that holds the socket, or none where the
/// watch could not be made; `wait` is then a timed sleep.
pub(in crate::sys::ipc) struct BindWait(Option<Watch>);

impl BindWait {
    pub(in crate::sys::ipc) fn new(dir: &Path) -> Self {
        match Watch::new(dir) {
            Ok(watch) => Self(Some(watch)),
            Err(e) => {
                iris_lib::ilog!("iris: watch {} for the daemon's socket: {e}", dir.display());
                Self(None)
            }
        }
    }

    /// Return once an entry is made in the directory, or once `limit`
    /// passes.
    pub(in crate::sys::ipc) fn wait(&self, limit: Duration) {
        match &self.0 {
            Some(watch) => watch.wait(limit),
            None => std::thread::sleep(limit),
        }
    }
}

fn timespec(d: Duration) -> libc::timespec {
    // SAFETY: timespec is plain integers; some targets add padding
    // fields, so it is zeroed and then assigned, not built literally.
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    ts.tv_sec = libc::time_t::try_from(d.as_secs()).unwrap_or(libc::time_t::MAX);
    ts.tv_nsec = d.subsec_nanos() as _;
    ts
}

fn c_path(dir: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    Ok(std::ffi::CString::new(dir.as_os_str().as_bytes())?)
}

fn owned(fd: libc::c_int) -> io::Result<OwnedFd> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just returned open by the kernel and has no other owner.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// An inotify instance watching the directory for new entries.
#[cfg(target_os = "linux")]
struct Watch(OwnedFd);

#[cfg(target_os = "linux")]
impl Watch {
    fn new(dir: &Path) -> io::Result<Self> {
        // SAFETY: plain syscalls on a descriptor this function owns.
        let fd = owned(unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) })?;
        let path = c_path(dir)?;
        let mask = libc::IN_CREATE | libc::IN_MOVED_TO;
        if unsafe { libc::inotify_add_watch(fd.as_raw_fd(), path.as_ptr(), mask) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(fd))
    }

    fn wait(&self, limit: Duration) {
        let fd = self.0.as_raw_fd();
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ts = timespec(limit);
        // SAFETY: one valid pollfd; a null sigmask leaves signals as they are.
        unsafe { libc::ppoll(&mut poll, 1, &ts, std::ptr::null()) };
        // Drain the queued events, so the next wait blocks until a new one.
        let mut events = [0u8; 4096];
        // SAFETY: reads into a buffer of the stated length on a
        // non-blocking descriptor; it stops at EAGAIN.
        while unsafe { libc::read(fd, events.as_mut_ptr().cast(), events.len()) } > 0 {}
    }
}

/// A kqueue with a vnode filter on the open directory: a write to a
/// directory is an entry made or removed in it.
#[cfg(target_os = "macos")]
struct Watch {
    queue: OwnedFd,
    _dir: OwnedFd,
}

#[cfg(target_os = "macos")]
impl Watch {
    fn new(dir: &Path) -> io::Result<Self> {
        let path = c_path(dir)?;
        // SAFETY: plain syscalls on descriptors this function owns.
        let dir = owned(unsafe { libc::open(path.as_ptr(), libc::O_EVTONLY | libc::O_CLOEXEC) })?;
        let queue = owned(unsafe { libc::kqueue() })?;
        let change = libc::kevent {
            ident: dir.as_raw_fd() as libc::uintptr_t,
            filter: libc::EVFILT_VNODE,
            flags: libc::EV_ADD | libc::EV_CLEAR,
            fflags: libc::NOTE_WRITE,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        // SAFETY: registers one change and waits for no events.
        let added = unsafe {
            libc::kevent(queue.as_raw_fd(), &change, 1, std::ptr::null_mut(), 0, std::ptr::null())
        };
        if added < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { queue, _dir: dir })
    }

    fn wait(&self, limit: Duration) {
        let ts = timespec(limit);
        // SAFETY: kevent is plain data; zero is a valid value.
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        // SAFETY: room for one event. EV_CLEAR resets the filter once it
        // is reported, so the next wait blocks until a new write.
        unsafe { libc::kevent(self.queue.as_raw_fd(), std::ptr::null(), 0, &mut event, 1, &ts) };
    }
}

/// No directory watch on this OS: `BindWait` sleeps.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
struct Watch;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl Watch {
    fn new(_: &Path) -> io::Result<Self> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn wait(&self, limit: Duration) {
        std::thread::sleep(limit);
    }
}
