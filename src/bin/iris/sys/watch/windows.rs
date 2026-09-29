//! Windows: a change notification on each directory, and a
//! manual-reset event that is the waker. A wait waits on all of them.

use std::cell::RefCell;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForMultipleObjects, INFINITE,
};

use super::{Entries, Woke};

/// The most handles one WaitForMultipleObjects takes
/// (MAXIMUM_WAIT_OBJECTS); the waker's event holds one.
const WAIT_SLOTS: usize = 64;

pub(super) struct Watch {
    wake: Arc<Event>,
    /// The waker's event, then a change notification per directory. A
    /// notification whose directory is gone is dropped from the list.
    handles: RefCell<Vec<HANDLE>>,
}

// SAFETY: the handles are kernel object handles, usable from any
// thread; the watch closes its notifications on drop.
unsafe impl Send for Watch {}

/// A manual-reset event: once set it stays set, as no wait resets it.
struct Event(HANDLE);

// SAFETY: an event handle is usable from any thread, and SetEvent is
// safe to call concurrently.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: an event this value opened and no other closes.
        unsafe { CloseHandle(self.0) };
    }
}

#[derive(Clone)]
pub(super) struct Waker(Arc<Event>);

impl Watch {
    /// Both entry kinds report as a file or folder name change: a
    /// notification cannot tell a made entry from a removed one.
    pub(super) fn new(dirs: &[PathBuf], _: Entries) -> io::Result<Self> {
        if dirs.len() >= WAIT_SLOTS {
            return Err(io::Error::other(format!(
                "{} directories to watch; one wait takes at most {}",
                dirs.len(),
                WAIT_SLOTS - 1
            )));
        }
        // SAFETY: no name and no security attributes; a manual-reset
        // event that starts unset.
        let wake = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if wake.is_null() {
            return Err(io::Error::last_os_error());
        }
        let watch = Self {
            wake: Arc::new(Event(wake)),
            handles: RefCell::new(vec![wake]),
        };
        for dir in dirs {
            let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
            let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME;
            // SAFETY: a NUL-terminated wide path that outlives the call.
            let note = unsafe { FindFirstChangeNotificationW(wide.as_ptr(), 0, filter) };
            if note == INVALID_HANDLE_VALUE {
                let e = io::Error::last_os_error();
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) {
                    continue;
                }
                return Err(io::Error::new(e.kind(), format!("{}: {e}", dir.display())));
            }
            watch.handles.borrow_mut().push(note);
        }
        Ok(watch)
    }

    pub(super) fn waker(&self) -> Waker {
        Waker(Arc::clone(&self.wake))
    }

    pub(super) fn wait(&self, limit: Option<Duration>) -> Woke {
        let ms = limit.map_or(INFINITE, |d| {
            // Rounded up: a limit under a millisecond still blocks.
            u32::try_from(d.as_nanos().div_ceil(1_000_000))
                .map_or(INFINITE - 1, |ms| ms.min(INFINITE - 1))
        });
        let mut handles = self.handles.borrow_mut();
        // SAFETY: at most WAIT_SLOTS open handles; any one ends the wait.
        let woke = unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, ms) };
        let index = woke.wrapping_sub(WAIT_OBJECT_0) as usize;
        if index == 0 {
            return Woke::Woken;
        }
        if index >= handles.len() {
            return Woke::Limit;
        }
        // Re-arm the notification. One that cannot re-arm has lost its
        // directory and would end every later wait at once: drop it.
        // SAFETY: a notification this watch opened.
        if unsafe { FindNextChangeNotification(handles[index]) } == 0 {
            let dead = handles.swap_remove(index);
            // SAFETY: a notification this watch opened, now out of the list.
            unsafe { FindCloseChangeNotification(dead) };
        }
        Woke::Changed
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // The event at index 0 closes with the last Arc of it.
        for &note in &self.handles.get_mut()[1..] {
            // SAFETY: a notification this watch opened and no other closes.
            unsafe { FindCloseChangeNotification(note) };
        }
    }
}

impl Waker {
    pub(super) fn wake(&self) {
        // SAFETY: an event this handle keeps open.
        unsafe { SetEvent(self.0 .0) };
    }
}
