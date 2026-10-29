//! WHY: the class closed here is "a panic leaves no trail in the log". A
//! daemon started by the desktop or at login has no terminal, so a panic
//! printed only to stderr was lost, and the log file showed a daemon that
//! stopped mid-line. The hook writes every panic, on any thread and with
//! either payload type, to the log file; it must also return when the
//! panicking thread holds the log's own lock.
//!
//! Not covered: a panic raised before `init` runs, which has no file to
//! write to; an abort that never runs the hook, such as a stack overflow.

use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

use super::{install_panic_hook, LOG_FILE};

/// The cases share the log's lock and the process's panic hook: one runs
/// at a time, or a case holding the lock delays the other's line past
/// the hook's wait.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Panic on a thread named `name` with `raise`, and wait for the thread
/// to end, for at most 5 s. A thread that does not end may hold the log's
/// lock inside the hook, where this thread's own panic would wait for it
/// too, so the timeout ends the test process instead of panicking.
fn panic_on(name: &str, raise: impl FnOnce() + Send + 'static) {
    let (done, ended) = mpsc::channel();
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(raise));
            let _ = done.send(result.is_err());
        })
        .unwrap();
    let Ok(panicked) = ended.recv_timeout(Duration::from_secs(5)) else {
        eprintln!("the thread '{name}' did not end within 5 s of its panic");
        std::process::exit(101);
    };
    assert!(panicked, "the thread '{name}' did not panic");
}

#[test]
fn a_panic_on_any_thread_is_written_to_the_log() {
    let _serial = serial();
    install_panic_hook();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("iris.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap();
    let previous = LOG_FILE.lock().replace((file, 0));

    panic_on("journal-str", || panic!("a static\nmessage"));
    let n = 7;
    panic_on("journal-string", move || panic!("a formatted message {n}"));

    *LOG_FILE.lock() = previous;
    let mut log = String::new();
    std::fs::File::open(&path)
        .unwrap()
        .read_to_string(&mut log)
        .unwrap();
    for (thread, message) in [
        ("journal-str", "a static message"),
        ("journal-string", "a formatted message 7"),
    ] {
        let line = log
            .lines()
            .find(|l| l.contains(&format!("panic on thread '{thread}'")))
            .unwrap_or_else(|| panic!("no line for the panic on '{thread}' in:\n{log}"));
        // `file!()` is the path the panic's location reports, with the
        // platform's separators.
        let at = format!("at {}:", file!());
        assert!(
            line.contains(&at) && line.ends_with(message),
            "the panic on '{thread}' is written as {line:?}"
        );
    }
}

#[test]
fn a_panic_while_holding_the_log_lock_still_ends_its_thread() {
    let _serial = serial();
    install_panic_hook();
    panic_on("journal-held", || {
        let _held = LOG_FILE.lock();
        panic!("raised with the log locked");
    });
}
