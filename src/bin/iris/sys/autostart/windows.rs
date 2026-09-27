//! Windows: the `iris` value of this account's Run key, the value the
//! installer writes and the uninstaller deletes. Task Manager's
//! Startup apps list turns a Run value off without deleting it, in the
//! StartupApproved key beside it.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegGetValueW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_BINARY,
    RRF_RT_REG_SZ,
};

/// The keys under HKEY_CURRENT_USER that start programs at login.
struct Keys<'a> {
    /// Each value a command line run at login.
    run: &'a str,
    /// Task Manager's switch per Run value.
    approved: &'a str,
}

const ACCOUNT: Keys<'static> = Keys {
    run: r"Software\Microsoft\Windows\CurrentVersion\Run",
    approved: r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
};

/// The value's name (packaging/windows/iris.nsi).
const VALUE: &str = "iris";

/// `"<this iris.exe>" --daemon`, the command line the installer writes.
fn command() -> Result<OsString, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("start at login: find this iris: {e}"))?;
    Ok(command_for(&exe))
}

fn command_for(exe: &Path) -> OsString {
    let mut command = OsString::from("\"");
    command.push(exe);
    command.push("\" --daemon");
    command
}

/// Whether the Run value starts this iris at login.
pub fn enabled() -> bool {
    command().is_ok_and(|command| enabled_in(&ACCOUNT, &command))
}

/// Make this iris start at login, or start nothing.
pub fn set(on: bool) -> Result<(), String> {
    set_in(&ACCOUNT, &command()?, on)
}

/// Whether the Run value is `command`, ignoring case as Windows paths
/// do, and Task Manager has not turned it off.
fn enabled_in(keys: &Keys<'_>, command: &OsStr) -> bool {
    let Some(value) = read(keys.run, RRF_RT_REG_SZ) else {
        return false;
    };
    // REG_SZ data ends in a NUL; the command does not.
    let value = OsString::from_wide(value.split(|&u| u == 0).next().unwrap_or_default());
    let same = value.to_string_lossy().to_lowercase() == command.to_string_lossy().to_lowercase();
    same && !turned_off(keys)
}

/// Whether Task Manager turned the value off: the first byte of its
/// StartupApproved data is odd (02 on, 03 off).
fn turned_off(keys: &Keys<'_>) -> bool {
    read(keys.approved, RRF_RT_REG_BINARY).is_some_and(|data| {
        data.first()
            .is_some_and(|&unit| unit.to_le_bytes()[0] & 1 == 1)
    })
}

fn set_in(keys: &Keys<'_>, command: &OsStr, on: bool) -> Result<(), String> {
    // A Task Manager switch left from before would keep a new Run value
    // off, and one left after the value is gone is litter.
    delete(keys.approved)?;
    if !on {
        return delete(keys.run);
    }
    let subkey = wide(OsStr::new(keys.run));
    let mut key: HKEY = null_mut();
    // SAFETY: `subkey` is NUL-terminated; `key` receives the handle,
    // closed below.
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(error("open", keys.run, rc));
    }
    let name = wide(OsStr::new(VALUE));
    let data = wide(command);
    // SAFETY: `key` is open with KEY_SET_VALUE; `data` holds the
    // NUL-terminated string and its byte length counts the NUL, as
    // REG_SZ requires.
    let rc = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    // SAFETY: `key` came from RegCreateKeyExW and is closed once.
    unsafe { RegCloseKey(key) };
    if rc != ERROR_SUCCESS {
        return Err(error("write", keys.run, rc));
    }
    Ok(())
}

/// Delete the value of `subkey`; a value or key that does not exist
/// is already deleted.
fn delete(subkey: &str) -> Result<(), String> {
    let (key, name) = (wide(OsStr::new(subkey)), wide(OsStr::new(VALUE)));
    // SAFETY: both names are NUL-terminated.
    let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) };
    match rc {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        rc => Err(error("delete", subkey, rc)),
    }
}

/// The value of `subkey` as UTF-16 units, when it exists and has a
/// type `flags` allows.
fn read(subkey: &str, flags: u32) -> Option<Vec<u16>> {
    let (key, name) = (wide(OsStr::new(subkey)), wide(OsStr::new(VALUE)));
    let mut bytes = 0u32;
    // SAFETY: the names are NUL-terminated; a null data pointer asks
    // only for the size, written to `bytes`.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            null_mut(),
            &mut bytes,
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let mut data = vec![0u16; (bytes as usize).div_ceil(2)];
    // SAFETY: `data` holds at least `bytes` bytes. A value that grew
    // since the size query fails with ERROR_MORE_DATA instead of
    // overrunning.
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            data.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    data.truncate((bytes as usize).div_ceil(2));
    Some(data)
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

fn error(what: &str, subkey: &str, rc: u32) -> String {
    format!(
        "start at login: {what} HKEY_CURRENT_USER\\{subkey}\\{VALUE}: {}",
        std::io::Error::from_raw_os_error(rc as i32)
    )
}

// WHY: the classes closed here are "Start at login reads on for a Run
// value that starts another iris.exe or that Task Manager turned off",
// "on leaves Task Manager's off switch in place", and "off fails when
// there is nothing to delete". The tests use a key of their own under
// HKEY_CURRENT_USER\Software\iris-tests, never the account's Run key.
// Not covered: Windows running the value at the next login.
#[cfg(test)]
mod tests;
