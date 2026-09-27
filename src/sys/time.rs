//! The C library's local-time conversion. POSIX `localtime_r` and
//! Windows `localtime_s` differ in argument order and return convention.

/// Epoch `secs` as local calendar fields, via the C library's timezone
/// database, or None when the conversion fails (no zone database).
pub(crate) fn localtime(secs: i64) -> Option<libc::tm> {
    // SAFETY: `libc::tm` is plain integers (and on glibc a nullable
    // zone-name pointer), so all-zero is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call and do not alias.
    #[cfg(unix)]
    let ok = unsafe { !libc::localtime_r(&secs, &mut tm).is_null() };
    // SAFETY: as above; `localtime_s` returns an errno, 0 on success.
    #[cfg(windows)]
    let ok = unsafe { libc::localtime_s(&mut tm, &secs) == 0 };
    ok.then_some(tm)
}
