//! Windows: `SHOpenFolderAndSelectItems` opens an Explorer window on the
//! containing folder with the file selected. A path the shell cannot
//! resolve opens its folder through `explorer.exe` instead.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use windows_sys::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows_sys::Win32::UI::Shell::{ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems};

pub fn reveal(path: &Path) {
    // absolute(), not canonicalize(): the shell parser rejects the
    // `\\?\` verbatim prefix canonicalize adds.
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    std::thread::spawn(move || {
        let wide: Vec<u16> = abs.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: `wide` is NUL-terminated and outlives the call; the
        // PIDL is freed on the path that created it; COM is initialized
        // on this thread for the shell call and released after it.
        let shown = unsafe {
            let com = OleInitialize(std::ptr::null());
            let pidl = ILCreateFromPathW(wide.as_ptr());
            let shown = !pidl.is_null() && {
                let hr = SHOpenFolderAndSelectItems(pidl, 0, std::ptr::null(), 0);
                ILFree(pidl);
                hr >= 0
            };
            if com >= 0 {
                OleUninitialize();
            }
            shown
        };
        if !shown {
            if let Some(parent) = abs.parent() {
                // explorer.exe exits 1 on success; its status says nothing.
                let _ = Command::new("explorer.exe").arg(parent).status();
            }
        }
    });
}
