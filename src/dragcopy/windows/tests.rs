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

/// The paths in a `CF_HDROP` handle, as a drop target reads them.
#[cfg(windows)]
unsafe fn hdrop_paths(h: *mut core::ffi::c_void) -> Vec<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::UI::Shell::DragQueryFileW;

    let n = DragQueryFileW(h, u32::MAX, std::ptr::null_mut(), 0);
    (0..n)
        .map(|i| {
            let len = DragQueryFileW(h, i, std::ptr::null_mut(), 0);
            let mut buf = vec![0u16; len as usize + 1];
            DragQueryFileW(h, i, buf.as_mut_ptr(), buf.len() as u32);
            buf.truncate(len as usize);
            std::ffi::OsString::from_wide(&buf).into()
        })
        .collect()
}

/// WHY: the class closed here is "a drag payload a drop target reads
/// nothing from": ID lists that resolve to the wrong items, or a data
/// object without the formats drop targets read, so Explorer refuses
/// the drop and browsers and chat apps receive no files. The object
/// `drag_abs_paths` drags, for files in two folders, must list them
/// in `CF_HDROP`, which browsers and chat apps read, and the drop
/// target of an Explorer folder, the one a drop onto that folder's
/// window reaches, must copy them in. Not covered: the pointer loop
/// inside `SHDoDragDrop` and the input attach, which need a live
/// desktop.
#[cfg(windows)]
#[test]
fn dragged_files_reach_hdrop_readers_and_explorer_folders() {
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use windows_sys::core::GUID;
    use windows_sys::Win32::Foundation::{GlobalFree, POINTL};
    use windows_sys::Win32::System::Ole::{
        OleInitialize, OleUninitialize, CF_HDROP, DROPEFFECT_COPY,
    };
    use windows_sys::Win32::UI::Shell::{ILFree, SHBindToObject, SHParseDisplayName};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };

    // IID_IShellFolder {000214E6-0000-0000-C000-000000000046}.
    const IID_ISHELLFOLDER: GUID = GUID::from_u128(0x000214e6_0000_0000_c000_000000000046);
    // IID_IDropTarget {00000122-0000-0000-C000-000000000046}.
    const IID_IDROPTARGET: GUID = GUID::from_u128(0x00000122_0000_0000_c000_000000000046);
    const MK_LBUTTON: u32 = 0x0001;
    const DVASPECT_CONTENT: u32 = 1;
    const TYMED_HGLOBAL: u32 = 1;
    type Obj = *mut core::ffi::c_void;
    type DropFn = unsafe extern "system" fn(Obj, Obj, u32, POINTL, *mut u32) -> i32;
    #[repr(C)]
    struct FormatEtc {
        cf: u16,
        ptd: Obj,
        aspect: u32,
        lindex: i32,
        tymed: u32,
    }
    #[repr(C)]
    struct StgMedium {
        tymed: u32,
        handle: Obj,
        owner: Obj,
    }

    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let dst = tempfile::tempdir().unwrap();
    let files = [a.path().join("shot one.png"), b.path().join("é.png")];
    for (i, f) in files.iter().enumerate() {
        std::fs::write(f, format!("capture {i}")).unwrap();
    }
    // One spelling per file: the temp directory may be named with
    // 8.3 short names, and the shell reports long ones.
    fn canonical(paths: &[PathBuf]) -> Vec<PathBuf> {
        paths
            .iter()
            .map(|p| std::fs::canonicalize(p).unwrap())
            .collect()
    }
    let arrived = || {
        files.iter().enumerate().all(|(i, f)| {
            std::fs::read(dst.path().join(f.file_name().unwrap())).ok()
                == Some(format!("capture {i}").into_bytes())
        })
    };
    unsafe {
        assert!(OleInitialize(std::ptr::null()) >= 0);
        let data = ShellData::new(&files).unwrap();

        // IDataObject::GetData is vtable slot 3.
        let get: unsafe extern "system" fn(Obj, *const FormatEtc, *mut StgMedium) -> i32 =
            std::mem::transmute(vtable(data.obj, 3));
        let want = FormatEtc {
            cf: CF_HDROP,
            ptd: std::ptr::null_mut(),
            aspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL,
        };
        let mut medium = StgMedium {
            tymed: 0,
            handle: std::ptr::null_mut(),
            owner: std::ptr::null_mut(),
        };
        let hr = get(data.obj, &want, &mut medium);
        assert!(
            hr >= 0 && medium.tymed == TYMED_HGLOBAL,
            "CF_HDROP: {hr:#x}"
        );
        let listed = hdrop_paths(medium.handle);
        // What ReleaseStgMedium does for an HGLOBAL: the owner
        // releases it, or the receiver frees it.
        if medium.owner.is_null() {
            GlobalFree(medium.handle);
        } else {
            release(medium.owner);
        }
        assert_eq!(canonical(&listed), canonical(&files));

        let dir: Vec<u16> = dst
            .path()
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut pidl = std::ptr::null_mut();
        let hr = SHParseDisplayName(
            dir.as_ptr(),
            std::ptr::null_mut(),
            &mut pidl,
            0,
            std::ptr::null_mut(),
        );
        assert!(hr >= 0, "parse {}: {hr:#x}", dst.path().display());
        let mut folder: Obj = std::ptr::null_mut();
        let hr = SHBindToObject(
            std::ptr::null_mut(),
            pidl,
            std::ptr::null_mut(),
            &IID_ISHELLFOLDER,
            &mut folder,
        );
        ILFree(pidl);
        assert!(hr >= 0, "bind folder: {hr:#x}");
        // IShellFolder::CreateViewObject (slot 8) with IID_IDropTarget:
        // the target of a drop onto the folder's window.
        let create: unsafe extern "system" fn(Obj, Obj, *const GUID, *mut Obj) -> i32 =
            std::mem::transmute(vtable(folder, 8));
        let mut target: Obj = std::ptr::null_mut();
        let hr = create(folder, std::ptr::null_mut(), &IID_IDROPTARGET, &mut target);
        assert!(hr >= 0, "folder drop target: {hr:#x}");

        // IDropTarget::DragEnter is slot 3, IDropTarget::Drop slot 6.
        // The drag allows copy only, as `drag_abs_paths` does.
        let enter: DropFn = std::mem::transmute(vtable(target, 3));
        let drop_on: DropFn = std::mem::transmute(vtable(target, 6));
        let pt = POINTL { x: 0, y: 0 };
        let mut effect = DROPEFFECT_COPY;
        let hr = enter(target, data.obj, MK_LBUTTON, pt, &mut effect);
        assert!(hr >= 0, "drag enter: {hr:#x}");
        assert_eq!(effect, DROPEFFECT_COPY, "the folder rejects the drag");
        let mut effect = DROPEFFECT_COPY;
        let hr = drop_on(target, data.obj, MK_LBUTTON, pt, &mut effect);
        assert!(hr >= 0, "drop: {hr:#x}");

        // The shell may copy on a worker that calls back into this
        // apartment, so the wait pumps its messages. Bounded.
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut msg: MSG = std::mem::zeroed();
        while !arrived() && Instant::now() < deadline {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        release(target);
        release(folder);
        drop(data);
        OleUninitialize();
    }
    assert!(
        arrived(),
        "the drop left {:?}",
        std::fs::read_dir(dst.path()).unwrap().collect::<Vec<_>>()
    );
    // A copy, not a move: the captures stay where they were saved.
    assert!(files.iter().all(|f| f.exists()));
}

#[cfg(windows)]
#[test]
#[ignore = "replaces the clipboard; run with --ignored on Windows"]
fn clipboard_round_trip_reads_back_as_file_list() {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, OpenClipboard,
    };
    use windows_sys::Win32::System::Ole::CF_HDROP;

    let dir = tempfile::tempdir().unwrap();
    let files = [dir.path().join("shot one.png"), dir.path().join("é.png")];
    for f in &files {
        std::fs::write(f, b"x").unwrap();
    }
    crate::dragcopy::copy_file_paths(&files).unwrap();

    let read = unsafe {
        assert_ne!(OpenClipboard(std::ptr::null_mut()), 0);
        let h = GetClipboardData(CF_HDROP as u32);
        assert!(!h.is_null(), "CF_HDROP absent after copy");
        let read = hdrop_paths(h);
        CloseClipboard();
        read
    };
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
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
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
