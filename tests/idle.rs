//! WHY: an idle daemon does not wake on a timer. The classes closed here:
//!
//! - A `blocking` pool thread that outlives its work. The pool serves
//!   zbus (the tray and the portals) and async-fs, and blocking 1.7 keeps
//!   every pool thread it starts for the life of the process, waking it
//!   every 500 ms: each daemon woke twice a second at idle. Covered
//!   whichever blocking release the lock file resolves to.
//! - A window that runs frames with nothing to draw. The X11 platform ran
//!   a timer at the display refresh rate for every open window, so a
//!   daemon with its home window open woke its main thread 60 times a
//!   second. Every option of `tests/support/options.rs` that opens a
//!   window opens it here, each in a daemon of its own, and `ALLOWED`
//!   requires a decision on the main-thread wakeups of each, so a window
//!   added to `iris --help` fails the case until one is recorded.
//! - A window that polls the disk. The library read its capture store
//!   every 1.5 s, which woke the main thread twice in each `WATCH` span.
//!   Each daemon here holds a capture in its store, so the library
//!   watches the capture's folder.
//! - A watch that starts before the window's start ends. The start (the
//!   first frames, the editor's image decode, the driver's shader
//!   compiles) ends about a second after the map on an unloaded host and
//!   seconds later on a loaded one, and waits on the X server and the
//!   disk use no CPU in the daemon: a single watch after a CPU-quiet
//!   point caught the end of the editor's start on a loaded host. The
//!   daemon is watched in back-to-back spans of `WATCH` from the map
//!   until a span is within the allowance, for at most `DEADLINE`; a
//!   timer wakes the main thread in every span.
//!
//! Not covered: a timer with a period longer than the watch, work that
//! stops within `DEADLINE` of the map (an entrance animation), a timer
//! on a thread other than the main thread (the X11 window fixup thread
//! rescans every 50 ms while a fixup waits for a window manager to
//! manage its window, for at most 6 s, and the test server has no window
//! manager), a window that animates while the pointer is over it or
//! while it has the focus, Wayland, Windows, and macOS. The window case
//! runs only with `IRIS_X11_TEST_DISPLAY` set and needs `Xvfb`; without
//! it the case prints that it did not run.

#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "support/options.rs"]
mod options;
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

#[cfg(target_os = "linux")]
#[test]
fn an_idle_blocking_pool_thread_exits() {
    use std::time::{Duration, Instant};

    fn pool_threads() -> usize {
        std::fs::read_dir("/proc/self/task")
            .expect("list this process's threads")
            .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
            .filter(|comm| comm.starts_with("blocking-"))
            .count()
    }

    let ran_on = futures::executor::block_on(blocking::unblock(|| {
        std::thread::current().name().map(str::to_owned)
    }));
    assert!(
        ran_on
            .as_deref()
            .is_some_and(|n| n.starts_with("blocking-")),
        "the task ran on {ran_on:?}, not a pool thread"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while pool_threads() > 0 {
        assert!(
            Instant::now() < deadline,
            "a blocking pool thread is still up 5 s after its last task"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn an_idle_window_does_not_wake_the_main_thread() {
    use std::time::{Duration, Instant};

    use daemon::{enabled, store, xvfb, Daemon};
    use options::{open_args, single_png, Kind, EDITOR, OPTIONS};
    use x11::windows_of;
    use x11rb::connection::Connection as _;

    /// How long each span of the watch lasts.
    const WATCH: Duration = Duration::from_secs(2);

    /// How long after the map a window has to reach a span of `WATCH`
    /// within its allowance.
    const DEADLINE: Duration = Duration::from_secs(12);

    /// The main-thread wakeups each window is allowed in `WATCH` at idle,
    /// by title. A frame timer at the display refresh rate makes 120.
    const ALLOWED: &[(&str, u64)] = &[
        ("Library - iris", 0),
        ("Settings - iris", 0),
        ("iris", 0),
        (EDITOR, 0),
    ];

    if !enabled() {
        return;
    }
    let case = "an_idle_window_does_not_wake_the_main_thread";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    for &(option, kind) in OPTIONS {
        let Kind::Single(title) = kind else {
            continue;
        };
        let allowed = ALLOWED
            .iter()
            .find_map(|&(t, n)| (t == title).then_some(n))
            .unwrap_or_else(|| {
                panic!("record the idle main-thread wakeups of the {option} window in ALLOWED")
            });
        let dir = tempfile::tempdir().unwrap();
        let daemon = Daemon::start(dir.path(), |cmd| {
            cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
        });
        let shot = single_png(&dir.path().join("shots"));
        store(dir.path(), std::slice::from_ref(&shot));
        daemon.send(&open_args(option, &shot));
        daemon.until(
            &format!("{option} to map a window titled {title:?}"),
            || (windows_of(&conn, root, title).len() == 1).then_some(()),
        );
        let mapped = Instant::now();
        let (main, threads) = loop {
            let threads = daemon.wakeups(WATCH);
            let main = threads.get(&daemon.pid()).map_or(0, |t| t.1);
            if main <= allowed || mapped.elapsed() >= DEADLINE {
                break (main, threads);
            }
        };
        let woke: Vec<_> = threads.iter().filter(|(_, t)| t.1 > 0).collect();
        assert!(
            main <= allowed,
            "with the window {option} open, the daemon's main thread woke more than the \
             {allowed} allowed in every {WATCH:?} for {:?} after the map, {main} times in the \
             last; the threads that woke in the last: {woke:?}\n{}",
            mapped.elapsed(),
            daemon.log()
        );
    }
}
