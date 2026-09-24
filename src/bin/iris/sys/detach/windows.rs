//! Windows: CreateProcessW with bInheritHandles FALSE.
//!
//! `std::process::Command` always passes TRUE, which hands the child
//! every inheritable handle of this process, among them the standard
//! handles the caller passed this process even when the child's own are
//! set to NUL. A caller that read this process's output to its end then
//! waited for the child to exit.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DETACHED_PROCESS, PROCESS_INFORMATION, STARTUPINFOW,
};

/// `program` with `args` and no console.
pub(super) fn spawn(program: &Path, args: &[&str]) -> io::Result<()> {
    let mut line = command_line(program, args)?;
    create(program, &mut line).map(drop)
}

/// The NUL-terminated command line of `program` and `args`: the program
/// in double quotes, then each argument after a space. A path holds no
/// '"', and the quotes of the first argument end at the next '"'
/// whatever backslashes precede it. Each program parses the rest of its
/// command line itself, and the quoting rules differ between them: NSIS
/// reads `"/S"` as no /S. An argument that would need quotes, one that
/// is empty or holds whitespace or '"', fails with `InvalidInput`.
fn command_line(program: &Path, args: &[&str]) -> io::Result<Vec<u16>> {
    let mut line: Vec<u16> = std::iter::once(u16::from(b'"'))
        .chain(program.as_os_str().encode_wide())
        .chain([u16::from(b'"')])
        .collect();
    for arg in args {
        if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || c == '"') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("argument {arg:?} needs quotes on a Windows command line"),
            ));
        }
        line.push(u16::from(b' '));
        line.extend(arg.encode_utf16());
    }
    line.push(0);
    Ok(line)
}

/// Start `program` with the NUL-terminated command line `line`, no
/// console, and none of this process's handles. Returns the child's
/// process handle.
fn create(program: &Path, line: &mut [u16]) -> io::Result<OwnedHandle> {
    let app: Vec<u16> = program.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: STARTUPINFOW and PROCESS_INFORMATION are plain C structs
    // for which all zeroes is a valid value.
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `app` and `line` are NUL-terminated UTF-16 that outlive
    // the call, `line` is writable as CreateProcessW requires, and each
    // null pointer selects a default: no security attributes, and this
    // process's environment and working directory.
    let created = unsafe {
        CreateProcessW(
            app.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            DETACHED_PROCESS,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut info,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success both handles are open, and this call owns them.
    unsafe {
        CloseHandle(info.hThread);
        Ok(OwnedHandle::from_raw_handle(info.hProcess))
    }
}

// WHY: the classes closed here are "a detached child that holds its
// parent's handles" and "an argument the child reads differently".
// A client run with piped output started the daemon with
// bInheritHandles TRUE, and the caller's read of the client's output
// waited for the daemon to exit. The first test hands the parent an
// inheritable pipe end, as a caller's standard handle is, starts a
// child as `spawn` does, and reads the pipe to its end while the child
// runs. The second pins the command line the updater hands the NSIS
// installer, which reads a quoted "/S" as no /S, and the arguments
// that are rejected rather than quoted. Not covered: the working
// directory, which the child shares with this process.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;
    use std::io::Read;
    use std::os::windows::io::AsRawHandle;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE_FLAG_INHERIT, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};

    #[test]
    fn a_detached_child_holds_no_handle_of_its_parent() {
        let (mut reader, writer) = std::io::pipe().unwrap();
        // SAFETY: `writer` is an open pipe handle this test owns.
        let marked = unsafe {
            SetHandleInformation(writer.as_raw_handle(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)
        };
        assert_ne!(marked, 0, "{}", io::Error::last_os_error());
        // PING.EXE counting six echoes of the loopback address: a
        // console program that runs about five seconds.
        let ping = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
            .join(r"System32\PING.EXE");
        let mut line = command_line(&ping, &["-n", "6", "127.0.0.1"]).unwrap();
        let child = create(&ping, &mut line).unwrap();
        drop(writer);

        let started = Instant::now();
        reader.read_to_end(&mut Vec::new()).unwrap();
        let waited = started.elapsed();
        // SAFETY: `child` is a process handle this test owns.
        let running = unsafe { WaitForSingleObject(child.as_raw_handle(), 0) } == WAIT_TIMEOUT;
        unsafe { TerminateProcess(child.as_raw_handle(), 0) };
        assert!(
            running && waited < Duration::from_secs(2),
            "the pipe stayed open {waited:?} after this process closed its end \
             (child running: {running}): the child inherited it"
        );
    }

    #[test]
    fn arguments_reach_the_child_as_bare_tokens() {
        let setup = Path::new(r"C:\Users\a b\AppData\Local\iris\update\iris-setup.exe");
        let line = command_line(setup, &["/S", "/RUN"]).unwrap();
        assert_eq!(
            String::from_utf16(&line).unwrap(),
            "\"C:\\Users\\a b\\AppData\\Local\\iris\\update\\iris-setup.exe\" /S /RUN\0"
        );
        for arg in ["", "a b", "a\tb", "\"/S\"", r"/D=C:\Program Files\iris"] {
            let err = command_line(setup, &["/S", arg]).expect_err(arg);
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{arg:?}");
        }
    }
}
