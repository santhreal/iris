//! Descriptor and path conversions the Linux and macOS watches share.

use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::Path;
use std::time::Duration;

pub(super) fn timespec(d: Duration) -> libc::timespec {
    // SAFETY: timespec is plain integers; some targets add padding
    // fields, so it is zeroed and then assigned, not built literally.
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    ts.tv_sec = libc::time_t::try_from(d.as_secs()).unwrap_or(libc::time_t::MAX);
    ts.tv_nsec = d.subsec_nanos() as _;
    ts
}

pub(super) fn c_path(dir: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    Ok(std::ffi::CString::new(dir.as_os_str().as_bytes())?)
}

pub(super) fn owned(fd: libc::c_int) -> io::Result<OwnedFd> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` was just returned open by the kernel and has no other owner.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}
