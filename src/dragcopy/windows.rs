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

/// Entry `slot` of a COM object's vtable.
#[cfg(windows)]
unsafe fn vtable(obj: *mut core::ffi::c_void, slot: usize) -> *const core::ffi::c_void {
    *(*(obj as *const *const *const core::ffi::c_void)).add(slot)
}

/// Release a COM object: `IUnknown::Release` is vtable slot 2.
#[cfg(windows)]
unsafe fn release(obj: *mut core::ffi::c_void) {
    let unref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32 =
        std::mem::transmute(vtable(obj, 2));
    unref(obj);
}

/// The shell's data object for files (absolute paths), as Explorer
/// builds it for a selection of those files. Created and dropped on a
/// thread that called `OleInitialize`.
#[cfg(windows)]
struct ShellData {
    obj: *mut core::ffi::c_void,
}

#[cfg(windows)]
impl ShellData {
    fn new(paths: &[std::path::PathBuf]) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::{ILCreateFromPathW, ILFree};

        let mut pidls = Vec::with_capacity(paths.len());
        let mut made = Ok(());
        for p in paths {
            let w: Vec<u16> = p
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let pidl = unsafe { ILCreateFromPathW(w.as_ptr()) };
            if pidl.is_null() {
                made = Err(format!("drag: {} has no shell item", p.display()));
                break;
            }
            pidls.push(pidl);
        }
        let data = made.and_then(|()| unsafe { Self::from_id_lists(&pidls) });
        for p in pidls {
            unsafe { ILFree(p) };
        }
        data
    }

    /// An item array takes absolute ID lists from any folders, and its
    /// data object is the one Explorer drags. `SHCreateDataObject` with
    /// no parent folder returns an object with no formats at all.
    unsafe fn from_id_lists(
        pidls: &[*mut windows_sys::Win32::UI::Shell::Common::ITEMIDLIST],
    ) -> Result<Self, String> {
        use windows_sys::core::GUID;
        use windows_sys::Win32::UI::Shell::{BHID_DataObject, SHCreateShellItemArrayFromIDLists};

        // IID_IDataObject {0000010e-0000-0000-C000-000000000046}.
        const IID_IDATAOBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
        type Obj = *mut core::ffi::c_void;

        let mut array: Obj = std::ptr::null_mut();
        let hr = SHCreateShellItemArrayFromIDLists(
            pidls.len() as u32,
            pidls.as_ptr() as *const *const _,
            &mut array,
        );
        if hr < 0 || array.is_null() {
            return Err(format!("drag: no shell item array for the files: {hr:#x}"));
        }
        // IShellItemArray::BindToHandler is vtable slot 3.
        let bind: unsafe extern "system" fn(Obj, Obj, *const GUID, *const GUID, *mut Obj) -> i32 =
            std::mem::transmute(vtable(array, 3));
        let mut obj: Obj = std::ptr::null_mut();
        let hr = bind(
            array,
            std::ptr::null_mut(),
            &BHID_DataObject,
            &IID_IDATAOBJECT,
            &mut obj,
        );
        release(array);
        if hr < 0 || obj.is_null() {
            return Err(format!("drag: no data object for the files: {hr:#x}"));
        }
        Ok(ShellData { obj })
    }
}

#[cfg(windows)]
impl Drop for ShellData {
    fn drop(&mut self) {
        unsafe { release(self.obj) };
    }
}

/// Drag `paths` (absolute) out as files, from the pointer, until the
/// button is released. The shell builds the data object (`CF_HDROP`
/// plus shell IDs, so Explorer, browsers, and chat apps all accept it)
/// and supplies the drop source and drag image.
///
/// Runs on its own STA thread: `SHDoDragDrop` pumps a modal loop, and
/// pumping it inside a UI event handler re-enters the UI's window
/// procedure while its state is borrowed. The thread attaches to the
/// caller's input queue so the drag reads the same button state and
/// mouse capture as the window the press began in.
#[cfg(windows)]
pub(super) fn drag_abs_paths(paths: Vec<std::path::PathBuf>) -> Result<(), String> {
    use windows_sys::Win32::System::Ole::{OleInitialize, OleUninitialize, DROPEFFECT_COPY};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows_sys::Win32::UI::Shell::SHDoDragDrop;

    if paths.is_empty() {
        return Err("drag: no files".to_string());
    }
    let ui_tid = unsafe { GetCurrentThreadId() };
    std::thread::Builder::new()
        .name("iris-ole-drag".into())
        .spawn(move || unsafe {
            if OleInitialize(std::ptr::null()) < 0 {
                crate::ilog!("drag: OleInitialize failed");
                return;
            }
            let me = GetCurrentThreadId();
            let attached = AttachThreadInput(me, ui_tid, 1) != 0;
            match ShellData::new(&paths) {
                // The press may already be over (a flick shorter than a
                // thread spawn): a drag started then would follow the
                // pointer with no button held.
                Ok(data) => {
                    if GetAsyncKeyState(VK_LBUTTON as i32) < 0 {
                        let mut effect = 0u32;
                        SHDoDragDrop(
                            std::ptr::null_mut(),
                            data.obj,
                            std::ptr::null_mut(),
                            DROPEFFECT_COPY,
                            &mut effect,
                        );
                    }
                }
                Err(e) => crate::ilog!("{e}"),
            }
            if attached {
                AttachThreadInput(me, ui_tid, 0);
            }
            OleUninitialize();
        })
        .map_err(|e| format!("spawn drag thread: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests;
