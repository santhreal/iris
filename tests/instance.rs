//! One daemon per socket. Of the processes that start at once with no
//! daemon running, one becomes the daemon and every command line reaches
//! it; a daemon killed with no chance to clean up gives way to the next
//! start.
//!
//! WHY: the class closed here is "two daemons". A process that found no
//! daemon on the socket became one, a bare `iris` in the foreground and
//! a flagged one through a detached child, and nothing kept two such
//! processes apart: each found no daemon, and on Unix each bind replaced
//! the socket file of the one before, so all of them ran on, each with a
//! tray icon and windows of its own. A child that started after another
//! had bound forwarded `--home`, as a bare `iris` does, and opened the
//! home window for a command that asked for another window. The first
//! case starts four `iris --home` and a bare `iris` at once, over rounds,
//! and requires one daemon and one home window. The second starts four
//! `iris --library` at once, then an `iris --daemon` beside the daemon,
//! and requires one daemon, one library window, and no home window. The
//! third kills the daemon with SIGKILL, which leaves its socket file, and
//! requires the next `iris --home` to start a daemon in its place. The
//! cases run only with `IRIS_X11_TEST_DISPLAY` set and need `Xvfb`;
//! without it a case prints that it did not run. Not covered: a start
//! racing a daemon that quits, Wayland, Windows, and macOS.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::{Child, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use daemon::{enabled, iris, xvfb, Daemon};
use x11::windows_of;
use x11rb::connection::Connection as _;

/// Clients started at once.
const CLIENTS: usize = 4;

/// Rounds of the first case, each from no daemon.
const ROUNDS: usize = 3;

/// How long a process or window has to reach the state a case waits
/// for. A client waits up to 8 s for a daemon that is starting.
const BOUND: Duration = Duration::from_secs(12);

/// How long a window or a process that should not exist has to appear.
const SETTLE: Duration = Duration::from_millis(1500);

/// The home window's title.
const HOME: &str = "iris";

/// The library window's title.
const LIBRARY: &str = "Library - iris";

#[test]
fn processes_that_start_at_once_leave_one_daemon() {
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb("processes_that_start_at_once_leave_one_daemon") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let home = tempfile::tempdir().unwrap();
    let dir = home.path();
    let _reaper = Reaper(dir);
    for round in 0..ROUNDS {
        let clients: Vec<Child> = (0..CLIENTS)
            .map(|_| start(dir, &display, &["--home"]))
            .collect();
        let mut bare = start(dir, &display, &[]);
        for mut client in clients {
            succeeds(dir, &mut client, "iris --home");
        }
        let daemon = one_process(dir, round);
        // A bare `iris` that did not become the daemon raised home in
        // the one that did.
        if bare.id() != daemon {
            succeeds(dir, &mut bare, "a bare iris");
        }
        windows(dir, &conn, root, HOME, 1, round);
        std::thread::sleep(SETTLE);
        assert_eq!(
            windows_of(&conn, root, HOME).len(),
            1,
            "round {round}: a second home window opened"
        );
        assert_eq!(
            processes(dir),
            [daemon],
            "round {round}: a second daemon started"
        );
        quit(dir, &display);
        let _ = bare.wait();
    }
}

#[test]
fn a_start_that_finds_a_daemon_opens_no_window_of_its_own() {
    if !enabled() {
        return;
    }
    let case = "a_start_that_finds_a_daemon_opens_no_window_of_its_own";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let home = tempfile::tempdir().unwrap();
    let dir = home.path();
    let _reaper = Reaper(dir);
    let clients: Vec<Child> = (0..CLIENTS)
        .map(|_| start(dir, &display, &["--library"]))
        .collect();
    for mut client in clients {
        succeeds(dir, &mut client, "iris --library");
    }
    let daemon = one_process(dir, 0);
    windows(dir, &conn, root, LIBRARY, 1, 0);
    let mut late = start(dir, &display, &["--daemon"]);
    succeeds(dir, &mut late, "iris --daemon beside a daemon");
    std::thread::sleep(SETTLE);
    assert_eq!(
        windows_of(&conn, root, LIBRARY).len(),
        1,
        "a second library window opened"
    );
    assert_eq!(
        windows_of(&conn, root, HOME).len(),
        0,
        "a home window opened"
    );
    assert_eq!(processes(dir), [daemon], "a second daemon started");
    quit(dir, &display);
}

#[test]
fn a_start_replaces_a_killed_daemon() {
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb("a_start_replaces_a_killed_daemon") else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let home = tempfile::tempdir().unwrap();
    let dir = home.path();
    let _reaper = Reaper(dir);
    let daemon = Daemon::start(dir, |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let killed = one_process(dir, 0);
    // SIGKILL: the daemon removes nothing on its way out.
    drop(daemon);
    assert!(
        dir.join("run").join("iris.sock").exists(),
        "the killed daemon's socket file is gone"
    );
    let mut client = start(dir, &display, &["--home"]);
    succeeds(dir, &mut client, "iris --home after a killed daemon");
    let started = one_process(dir, 0);
    assert_ne!(started, killed);
    windows(dir, &conn, root, HOME, 1, 0);
    quit(dir, &display);
}

/// Start `iris args` on `display`, with every iris location under `dir`.
/// Its standard error is piped, except for a bare `iris`, which may
/// become the daemon and log to it for as long as it runs.
fn start(dir: &Path, display: &str, args: &[&str]) -> Child {
    let stderr = if args.is_empty() {
        Stdio::null()
    } else {
        Stdio::piped()
    };
    iris(dir)
        .args(args)
        .env("DISPLAY", display)
        .env_remove("WAYLAND_DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .unwrap()
}

/// Require `child` to exit with status 0 within `BOUND`.
fn succeeds(dir: &Path, child: &mut Child, what: &str) {
    let status: ExitStatus = within(dir, &format!("{what} to exit"), || {
        child.try_wait().unwrap().ok_or("it still runs".to_string())
    });
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    assert!(status.success(), "{what} exited {status}: {stderr}");
}

/// The one iris process of `dir` once every other has exited.
fn one_process(dir: &Path, round: usize) -> u32 {
    within(dir, "one iris process", || {
        match processes(dir).as_slice() {
            [pid] => Ok(*pid),
            all => Err(format!("round {round}: {} run: {all:?}", all.len())),
        }
    })
}

/// Wait until `count` windows titled `title` are mapped.
fn windows(
    dir: &Path,
    conn: &impl x11rb::connection::Connection,
    root: u32,
    title: &str,
    count: usize,
    round: usize,
) {
    within(
        dir,
        &format!("{count} windows titled {title:?}"),
        || match windows_of(conn, root, title).len() {
            n if n == count => Ok(()),
            n => Err(format!("round {round}: {n} are mapped")),
        },
    );
}

/// Quit `dir`'s daemon and wait until no iris process of `dir` is left.
fn quit(dir: &Path, display: &str) {
    let mut client = start(dir, display, &["--quit"]);
    succeeds(dir, &mut client, "iris --quit");
    within(dir, "every iris process to exit", || {
        match processes(dir).as_slice() {
            [] => Ok(()),
            left => Err(format!("{left:?} still run")),
        }
    });
}

/// The live iris processes whose `IRIS_HOME` is `dir`: its daemon, and
/// any process still starting or about to exit.
fn processes(dir: &Path) -> Vec<u32> {
    let exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_iris")).unwrap();
    let home = [b"IRIS_HOME=".as_slice(), dir.as_os_str().as_bytes()].concat();
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|entry| {
            let pid: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
            // A process that has exited reads as no executable.
            if std::fs::read_link(format!("/proc/{pid}/exe")).ok()? != exe {
                return None;
            }
            let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
            environ.split(|&b| b == 0).any(|v| v == home).then_some(pid)
        })
        .collect()
}

/// Poll `f` every 20 ms until it returns a value; after `BOUND`, fail
/// with `what`, the last state `f` reported, and the log every iris
/// process of `dir` writes to.
fn within<T>(dir: &Path, what: &str, mut f: impl FnMut() -> Result<T, String>) -> T {
    let deadline = Instant::now() + BOUND;
    loop {
        match f() {
            Ok(value) => return value,
            Err(state) if Instant::now() >= deadline => {
                let log = std::fs::read_to_string(dir.join("state").join("iris.log"));
                panic!(
                    "timed out waiting for {what}: {state}\n{}",
                    log.unwrap_or_default()
                );
            }
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// Kills every iris process of its directory on drop, the detached
/// daemons of a case that failed included.
struct Reaper<'a>(&'a Path);

impl Drop for Reaper<'_> {
    fn drop(&mut self) {
        for pid in processes(self.0) {
            // SAFETY: kill takes no pointers; the pid is an iris process
            // of this case.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
    }
}
