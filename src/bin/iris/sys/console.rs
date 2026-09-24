//! The terminal a command line runs from.
//!
//! On Windows iris.exe is a GUI program (`windows_subsystem` in
//! main.rs), so it starts with no console and none opens: not for the
//! daemon at login, a Start Menu launch, or a hotkey tool running
//! `iris --capture`. A command line typed in a terminal attaches to that
//! terminal's console to print its output and errors. A Unix process
//! inherits its terminal's streams, so there is nothing to attach.

/// Attach standard output and error to the parent process's console
/// when the parent has one and passed no streams of its own: output
/// redirected to a file or a pipe stays there. Call before the first
/// print.
pub fn attach() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Console::{
            AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        };
        // SAFETY: GetStdHandle and AttachConsole take no pointers and
        // only read or set this process's standard handles; no other
        // thread prints yet.
        unsafe {
            let passed = |which| {
                let handle = GetStdHandle(which);
                !handle.is_null() && handle != INVALID_HANDLE_VALUE
            };
            if !passed(STD_OUTPUT_HANDLE) && !passed(STD_ERROR_HANDLE) {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
    }
}
