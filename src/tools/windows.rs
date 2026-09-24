//! Windows tool locations: the PATH a new login gets, read from the
//! registry because the daemon's own PATH predates any install made
//! while it runs, and the Tesseract installer's default folders, which
//! it does not add to PATH.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::ptr::null_mut;

use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(super) fn hide_console(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// The registry PATH (user, then machine), then the Tesseract folders.
pub(super) fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for (key, sub) in [
        (HKEY_CURRENT_USER, "Environment"),
        (
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ),
    ] {
        if let Some(path) = registry_path(key, sub) {
            dirs.extend(std::env::split_paths(&path));
        }
    }
    for (base, sub) in [
        ("ProgramFiles", "Tesseract-OCR"),
        ("ProgramFiles(x86)", "Tesseract-OCR"),
        ("LOCALAPPDATA", r"Programs\Tesseract-OCR"),
    ] {
        if let Some(base) = std::env::var_os(base) {
            dirs.push(PathBuf::from(base).join(sub));
        }
    }
    dirs
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// The `Path` value under `key\sub` with `%VAR%` references expanded,
/// or None when it is absent or unreadable.
fn registry_path(key: HKEY, sub: &str) -> Option<OsString> {
    let (sub, name) = (wide(sub), wide("Path"));
    // Read the stored string as is, REG_SZ or REG_EXPAND_SZ, and expand
    // it below: one code path for both value types.
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut bytes = 0u32;
    // SAFETY: the names are NUL-terminated; a null data pointer asks
    // only for the size, written to `bytes`.
    let rc = unsafe {
        RegGetValueW(
            key,
            sub.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            null_mut(),
            &mut bytes,
        )
    };
    if rc != 0 || bytes < 2 {
        return None;
    }
    let mut raw = vec![0u16; bytes as usize / 2];
    // SAFETY: `raw` holds `bytes` bytes. A value that grew since the
    // size query fails with ERROR_MORE_DATA instead of overrunning.
    let rc = unsafe {
        RegGetValueW(
            key,
            sub.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            raw.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if rc != 0 {
        return None;
    }
    // SAFETY: `raw` is NUL-terminated (RegGetValueW guarantees it for
    // string types); a null destination asks for the expanded length.
    let need = unsafe { ExpandEnvironmentStringsW(raw.as_ptr(), null_mut(), 0) };
    if need == 0 {
        return None;
    }
    let mut out = vec![0u16; need as usize];
    // SAFETY: `out` holds `need` UTF-16 units, the size just reported.
    let got = unsafe { ExpandEnvironmentStringsW(raw.as_ptr(), out.as_mut_ptr(), need) };
    if got == 0 || got > need {
        return None;
    }
    // `got` counts the terminator.
    out.truncate(got as usize - 1);
    Some(OsString::from_wide(&out))
}
