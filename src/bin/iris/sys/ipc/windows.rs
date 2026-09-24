//! Windows: the daemon's named pipe, one per account.
//!
//! Every account on the machine shares one pipe namespace, so the
//! pipe's name holds the account's SID. The pipe's security descriptor
//! makes the account its owner and grants no other account access. A
//! client reads the owner of the pipe it opened and writes nothing to a
//! pipe another account owns, such as one that account created under
//! the name before the daemon started.

use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null_mut;
use std::sync::OnceLock;

use interprocess::local_socket::{prelude::*, GenericFilePath, ListenerOptions, Name, ToFsName};
use interprocess::os::windows::local_socket::ListenerOptionsExt;
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use widestring::{U16CStr, U16CString};
use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetSecurityInfo, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The account's pipe, `\\.\pipe\iris-<SID>`.
pub(super) fn socket_name() -> Result<Name<'static>, String> {
    format!(r"\\.\pipe\iris-{}", account()?)
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| format!("iris: pipe name: {e}"))
}

/// Bind the pipe with the account as its owner and the only account
/// with access to it.
pub(super) fn bind(name: Name<'static>) -> Result<LocalSocketListener, String> {
    let sid = account()?;
    let descriptor = U16CString::from_str(format!("O:{sid}D:P(A;;GA;;;{sid})"))
        .map_err(io::Error::other)
        .and_then(|sddl| SecurityDescriptor::deserialize(&sddl))
        .map_err(|e| format!("bind local socket: security descriptor: {e}"))?;
    ListenerOptions::new()
        .name(name)
        .security_descriptor(descriptor)
        .create_sync()
        .map_err(|e| format!("bind local socket: {e}"))
}

/// Connect to the account's daemon. A pipe another account owns fails
/// with `PermissionDenied` before a byte is written to it.
pub(super) fn connect(name: Name<'_>) -> io::Result<LocalSocketStream> {
    let stream = LocalSocketStream::connect(name)?;
    let LocalSocketStream::NamedPipe(pipe) = &stream;
    let owner = owner(pipe.inner().as_handle().as_raw_handle())?;
    let sid = account().map_err(io::Error::other)?;
    if owner != sid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("the pipe belongs to {owner}, not to this account, {sid}"),
        ));
    }
    Ok(stream)
}

/// The string SID of the account this process runs as, read once.
fn account() -> Result<String, String> {
    static SID: OnceLock<Result<String, String>> = OnceLock::new();
    SID.get_or_init(|| token_user().map_err(|e| format!("iris: read this process's account: {e}")))
        .clone()
}

/// The string SID of the user of this process's token.
fn token_user() -> io::Result<String> {
    let mut raw: HANDLE = null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo handle that needs no
    // close, and `raw` is writable.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success `raw` is an open token handle nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut len = 0u32;
    // SAFETY: a null buffer of length 0 asks for the length alone.
    unsafe { GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut len) };
    // u64 words: TOKEN_USER holds a pointer, which a byte buffer would
    // not align.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` holds at least `len` writable bytes.
    let read = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buf.as_mut_ptr().cast(),
            len,
            &mut len,
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: GetTokenInformation wrote a TOKEN_USER at the start of
    // `buf`, and its SID points into `buf`, which outlives the call.
    unsafe { sid_text((*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid) }
}

/// The string SID of the owner of the kernel object `handle`.
fn owner(handle: HANDLE) -> io::Result<String> {
    let mut sid: PSID = null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: `handle` is open for the call, and each out pointer is
    // writable or null.
    let err = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut sid,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if err != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(err as i32));
    }
    // SAFETY: `sid` points into `descriptor`, which GetSecurityInfo
    // allocated with LocalAlloc and which is freed once the SID is read.
    unsafe {
        let text = sid_text(sid);
        LocalFree(descriptor);
        text
    }
}

/// `sid` in its string form, `S-1-5-…`.
///
/// # Safety
/// `sid` points to a valid SID.
unsafe fn sid_text(sid: PSID) -> io::Result<String> {
    let mut text: *mut u16 = null_mut();
    // SAFETY: the caller passes a valid SID, and `text` is writable.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success `text` is a NUL-terminated string allocated
    // with LocalAlloc, freed once copied.
    unsafe {
        let string = U16CStr::from_ptr_str(text).to_string_lossy();
        LocalFree(text.cast());
        Ok(string)
    }
}

/// A pipe name no daemon uses.
#[cfg(test)]
pub(super) fn scratch_name() -> (Name<'static>, ()) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        r"\\.\pipe\iris-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    (name.to_fs_name::<GenericFilePath>().unwrap(), ())
}

// WHY: the class closed here is "a command line written to a pipe this
// account does not own". Every account shares the pipe namespace, so
// another account can create the daemon's pipe before the daemon does.
// The test makes a pipe the Administrators group owns, which only an
// elevated token may do, and connects as every client does. Not
// covered: another account opening the daemon's pipe, which takes a
// second account to run as.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    #[test]
    fn a_pipe_another_account_owns_is_refused() {
        let (name, ()) = scratch_name();
        let sddl = U16CString::from_str("O:BAD:(A;;GA;;;WD)").unwrap();
        let descriptor = SecurityDescriptor::deserialize(&sddl).unwrap();
        let listener = ListenerOptions::new()
            .name(name.clone())
            .security_descriptor(descriptor)
            .create_sync();
        let Ok(_listener) = listener else {
            eprintln!("skipped: only an elevated token may make Administrators a pipe's owner");
            return;
        };
        let err = connect(name.borrow()).expect_err("connected to a pipe Administrators own");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err}");
    }
}
