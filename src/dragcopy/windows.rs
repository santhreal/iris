//! File copy on Windows: a `CF_HDROP` clipboard entry, the format
//! Explorer's own Copy produces. Explorer pastes the files themselves.
//!
//! The payload is a `DROPFILES` header followed by the paths as UTF-16,
//! each NUL-terminated, with one extra NUL ending the list. The layout
//! is built by the pure [`hdrop_payload`] so it is testable on any host.

/// `sizeof(DROPFILES)`: `pFiles: u32`, `pt: POINT` (two `i32`),
/// `fNC: BOOL`, `fWide: BOOL`.
const DROPFILES_LEN: usize = 20;

/// `DROPFILES` plus the double-NUL-terminated wide path list.
pub(super) fn hdrop_payload(paths: &[Vec<u16>]) -> Vec<u8> {
    let chars: usize = paths.iter().map(|p| p.len() + 1).sum::<usize>() + 1;
    let mut out = Vec::with_capacity(DROPFILES_LEN + chars * 2);
    out.extend_from_slice(&(DROPFILES_LEN as u32).to_le_bytes()); // pFiles
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    out.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    out.extend_from_slice(&0i32.to_le_bytes()); // fNC
    out.extend_from_slice(&1i32.to_le_bytes()); // fWide: UTF-16 paths
    for p in paths {
        for u in p {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// Replace the clipboard contents with `paths` (absolute) as `CF_HDROP`.
#[cfg(windows)]
pub(super) fn copy_abs_paths(paths: &[std::path::PathBuf]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows_sys::Win32::System::Ole::CF_HDROP;

    let wide: Vec<Vec<u16>> = paths
        .iter()
        .map(|p| p.as_os_str().encode_wide().collect())
        .collect();
    let payload = hdrop_payload(&wide);

    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, payload.len());
        if mem.is_null() {
            return Err("GlobalAlloc failed for the clipboard file list".to_string());
        }
        let dst = GlobalLock(mem);
        if dst.is_null() {
            GlobalFree(mem);
            return Err("GlobalLock failed for the clipboard file list".to_string());
        }
        std::ptr::copy_nonoverlapping(payload.as_ptr(), dst as *mut u8, payload.len());
        GlobalUnlock(mem);

        if OpenClipboard(std::ptr::null_mut()) == 0 {
            GlobalFree(mem);
            return Err("clipboard is held by another application; retry".to_string());
        }
        EmptyClipboard();
        // On success the clipboard owns `mem`; on failure it stays ours.
        let set = SetClipboardData(CF_HDROP as u32, mem);
        CloseClipboard();
        if set.is_null() {
            GlobalFree(mem);
            return Err("SetClipboardData(CF_HDROP) failed".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // WHY: the class closed here is "Explorer reads a malformed file
    // list": a wrong pFiles offset, a missing fWide flag, or a missing
    // terminator makes Explorer paste nothing or read past the buffer.
    // `clipboard_round_trip_reads_back_as_file_list` covers ownership and
    // Explorer's reader on a real Windows clipboard; it is ignored by
    // default because it replaces the clipboard contents.
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "replaces the clipboard; run with --ignored on Windows"]
    fn clipboard_round_trip_reads_back_as_file_list() {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, GetClipboardData, OpenClipboard,
        };
        use windows_sys::Win32::System::Ole::CF_HDROP;
        use windows_sys::Win32::UI::Shell::DragQueryFileW;

        let dir = tempfile::tempdir().unwrap();
        let files = [dir.path().join("shot one.png"), dir.path().join("é.png")];
        for f in &files {
            std::fs::write(f, b"x").unwrap();
        }
        crate::dragcopy::copy_file_paths(&files).unwrap();

        let mut read = Vec::new();
        unsafe {
            assert_ne!(OpenClipboard(std::ptr::null_mut()), 0);
            let h = GetClipboardData(CF_HDROP as u32);
            assert!(!h.is_null(), "CF_HDROP absent after copy");
            let n = DragQueryFileW(h, u32::MAX, std::ptr::null_mut(), 0);
            for i in 0..n {
                let len = DragQueryFileW(h, i, std::ptr::null_mut(), 0);
                let mut buf = vec![0u16; len as usize + 1];
                DragQueryFileW(h, i, buf.as_mut_ptr(), buf.len() as u32);
                buf.truncate(len as usize);
                read.push(std::path::PathBuf::from(std::ffi::OsString::from_wide(
                    &buf,
                )));
            }
            CloseClipboard();
        }
        let expect: Vec<_> = files
            .iter()
            .map(|f| std::path::absolute(f).unwrap())
            .collect();
        assert_eq!(read, expect);
    }

    #[test]
    fn header_points_past_itself_and_marks_wide() {
        let p = hdrop_payload(&[wide("C:\\a.png")]);
        assert_eq!(u32::from_le_bytes(p[0..4].try_into().unwrap()), 20);
        assert_eq!(i32::from_le_bytes(p[16..20].try_into().unwrap()), 1);
    }

    #[test]
    fn paths_are_nul_separated_and_double_nul_terminated() {
        let p = hdrop_payload(&[wide("C:\\a"), wide("D:\\é")]);
        let units: Vec<u16> = p[20..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let mut expect = wide("C:\\a");
        expect.push(0);
        expect.extend(wide("D:\\é"));
        expect.extend([0, 0]);
        assert_eq!(units, expect);
    }

    #[test]
    fn empty_list_is_header_plus_terminator() {
        assert_eq!(hdrop_payload(&[]).len(), 22);
    }
}
