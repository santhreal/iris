//! Display servers and daemons, each private to one case.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Lavapipe, where installed: a daemon on a private server renders the
/// same on every host.
const LAVAPIPE: &str = "/usr/share/vulkan/icd.d/lvp_icd.json";

/// Whether the cases run: `IRIS_X11_TEST_DISPLAY` is set on a host that
/// runs the daemon's windows.
pub fn enabled() -> bool {
    std::env::var_os("IRIS_X11_TEST_DISPLAY").is_some()
}

/// A display server a case started. Killed on drop.
pub struct Server(Child);

impl Server {
    /// `None` when the server's executable is not installed.
    pub fn spawn(cmd: &mut Command) -> Option<Server> {
        match cmd.stdin(Stdio::null()).spawn() {
            Ok(child) => Some(Server(child)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => panic!("start {:?}: {e}", cmd.get_program()),
        }
    }

    pub fn kill(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }

    /// Stop the server with SIGSTOP: it keeps its sockets open and
    /// answers nothing, so a client's round trip never returns.
    pub fn stop(&self) {
        let pid = self.0.id();
        // SAFETY: kill takes no pointers; the pid is this case's child.
        let sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) };
        assert_eq!(sent, 0, "SIGSTOP {pid}");
        let stat = format!("/proc/{pid}/stat");
        until("the server to stop", || {
            let state = std::fs::read_to_string(&stat).unwrap();
            (state.rsplit_once(')')?.1.split_whitespace().next()? == "T").then_some(())
        });
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A private Xvfb and its display name, or `None`, printed, without
/// Xvfb on PATH. -displayfd: Xvfb takes the first free display number
/// and writes it to stdout once it accepts clients.
pub fn xvfb(case: &str) -> Option<(Server, String)> {
    let Some(mut xvfb) = Server::spawn(
        Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "640x480x24",
                "-nolisten",
                "tcp",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    ) else {
        eprintln!("{case} did not run: no Xvfb on PATH");
        return None;
    };
    let mut announced = BufReader::new(xvfb.0.stdout.take().unwrap());
    let mut number = String::new();
    announced.read_line(&mut number).unwrap();
    Some((xvfb, format!(":{}", number.trim())))
}

/// A headless sway whose socket is in `run`, and the socket's name for
/// `WAYLAND_DISPLAY`, or `None`, printed, without sway on PATH. sway runs
/// with no XWayland.
pub fn sway(case: &str, run: &Path) -> Option<(Server, std::ffi::OsString)> {
    let config = run.join("sway.conf");
    std::fs::write(&config, "xwayland disable\n").unwrap();
    // The headless backend with pixman: no GPU, no input devices, no
    // output scanned out. sway refuses to start while the NVIDIA module is
    // loaded unless told otherwise, even headless.
    let Some(sway) = Server::spawn(
        Command::new("sway")
            .arg("--unsupported-gpu")
            .arg("-c")
            .arg(config)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env("XDG_RUNTIME_DIR", run)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .env("WLR_RENDERER", "pixman")
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    ) else {
        eprintln!("{case} did not run: no sway on PATH");
        return None;
    };
    let socket = until("sway to listen", || {
        std::fs::read_dir(run)
            .ok()?
            .filter_map(|entry| Some(entry.ok()?.file_name()))
            .find(|name| {
                name.to_str()
                    .is_some_and(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))
            })
    });
    Some((sway, socket))
}

/// An X display name no server uses, and the number of connections made
/// to it. An X client on Linux connects to display `:N` through the
/// abstract socket `/tmp/.X11-unix/XN` first, which names no file; a
/// thread accepts each connection there, counts it, and closes it.
pub fn counted_x_display() -> (String, Arc<AtomicUsize>) {
    use std::os::linux::net::SocketAddrExt as _;
    use std::os::unix::net::{SocketAddr, UnixListener};
    for n in 200..400 {
        let name = format!("/tmp/.X11-unix/X{n}");
        // A server listening on the path would answer a client that
        // found no abstract socket.
        if Path::new(&name).exists() {
            continue;
        }
        let address = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let Ok(listener) = UnixListener::bind_addr(&address) else {
            continue;
        };
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = connections.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                drop(stream);
                counted.fetch_add(1, Ordering::SeqCst);
            }
        });
        return (format!(":{n}"), connections);
    }
    panic!("every X display number from :200 to :399 is in use");
}

/// `root/run`, private to this user as a session's runtime directory is.
pub fn runtime_dir(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let run = root.join("run");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).unwrap();
    run
}

/// `iris` with every iris location, its socket, and its captures under
/// `dir`, which it prepares: the capture folders, a config that names
/// them, and a private runtime directory. Its session bus address leads
/// nowhere, so the tray of a daemon it starts never registers on a
/// desktop's panel.
pub fn iris(dir: &Path) -> Command {
    for sub in ["config", "shots", "vids"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    let run = runtime_dir(dir);
    let quoted = |p: &Path| toml::Value::from(p.to_str().unwrap()).to_string();
    let config = dir.join("config").join("config.toml");
    let text = format!(
        "screenshots_dir = {}\nrecordings_dir = {}\n",
        quoted(&dir.join("shots")),
        quoted(&dir.join("vids")),
    );
    // A daemon of `dir` may read the config while a case starts another
    // process: a write would truncate it under that read.
    if std::fs::read_to_string(&config).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&config, text).unwrap();
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_iris"));
    cmd.env_remove("IRIS_SLOWMO")
        .env("IRIS_HOME", dir)
        .env("XDG_RUNTIME_DIR", &run)
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", dir.join("no-bus").display()),
        );
    if Path::new(LAVAPIPE).exists() {
        cmd.env("VK_ICD_FILENAMES", LAVAPIPE);
    }
    cmd
}

/// A daemon with every iris location, its socket, and its captures under
/// one directory (see [`iris`]). Killed on drop.
pub struct Daemon {
    child: Child,
    dir: PathBuf,
}

impl Daemon {
    /// Start the daemon with `session` setting its display variables, and
    /// wait for it to bind its socket.
    pub fn start(dir: &Path, session: impl FnOnce(&mut Command)) -> Daemon {
        let mut daemon = Daemon::spawn(dir, session);
        daemon.bound();
        daemon
    }

    /// Start the daemon with `session` setting its display variables, and
    /// return without waiting for its socket; see [`Daemon::bound`].
    pub fn spawn(dir: &Path, session: impl FnOnce(&mut Command)) -> Daemon {
        let mut cmd = iris(dir);
        let out = std::fs::File::create(dir.join("daemon.log")).unwrap();
        session(&mut cmd);
        let child = cmd
            .stdin(Stdio::null())
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .unwrap();
        Daemon {
            child,
            dir: dir.to_path_buf(),
        }
    }

    /// Wait for the daemon to bind its socket, for at most 10 s. A daemon
    /// that exits first, or binds nothing in time, fails with its log.
    /// The socket is polled every millisecond, so a case that acts once
    /// it returns acts within a millisecond of the bind.
    pub fn bound(&mut self) {
        let socket = self.dir.join("run").join("iris.sock");
        let bound = poll_every(Duration::from_millis(1), || {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!(
                    "the daemon exited {status} before it bound its socket:\n{}",
                    self.log()
                );
            }
            socket.exists().then_some(())
        });
        if bound.is_none() {
            panic!(
                "timed out waiting for the daemon to bind its socket; daemon log:\n{}",
                self.log()
            );
        }
    }

    /// Run `iris <args>` as a client of this daemon, which hands them over
    /// the socket and exits. With no display variables, a client that
    /// found no daemon could not start one in its place.
    pub fn send(&self, args: &[&str]) {
        let out = Command::new(env!("CARGO_BIN_EXE_iris"))
            .args(args)
            .env("IRIS_HOME", &self.dir)
            .env("XDG_RUNTIME_DIR", self.dir.join("run"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "iris {args:?} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Assert the daemon exits within `bound` of `event`. For a daemon
    /// still running, the failure lists the CPU each busy thread uses.
    pub fn exits(&mut self, bound: Duration, event: &str) {
        let deadline = Instant::now() + bound;
        while Instant::now() < deadline {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let before = self.threads();
        std::thread::sleep(Duration::from_millis(500));
        let mut busy: Vec<(String, Duration)> = self
            .threads()
            .into_iter()
            .map(|(tid, (name, cpu))| {
                let was = before.get(&tid).map_or(Duration::ZERO, |t| t.1);
                (format!("{name} {tid}"), cpu.saturating_sub(was) * 2)
            })
            .filter(|t| !t.1.is_zero())
            .collect();
        busy.sort_by_key(|t| std::cmp::Reverse(t.1));
        let total: Duration = busy.iter().map(|t| t.1).sum();
        panic!(
            "the daemon still runs {bound:?} after {event}, using {total:?} of CPU a second, \
             by thread {busy:?}:\n{}",
            self.log()
        );
    }

    /// Each thread of the daemon by id: its name and CPU time so far.
    fn threads(&self) -> BTreeMap<u32, (String, Duration)> {
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{}/task", self.child.id())) else {
            return BTreeMap::new();
        };
        tasks
            .filter_map(|task| {
                let path = task.ok()?.path();
                let tid = path.file_name()?.to_str()?.parse().ok()?;
                let name = std::fs::read_to_string(path.join("comm")).ok()?;
                let stat = std::fs::read_to_string(path.join("stat")).ok()?;
                Some((tid, (name.trim().to_string(), cpu(&stat))))
            })
            .collect()
    }

    /// Wait until the daemon is idle: under 20 ms of CPU in 250 ms. A
    /// server stopped while the daemon's main thread waits on a reply
    /// would hold that thread before it reads any command.
    pub fn settle(&self) {
        let stat = format!("/proc/{}/stat", self.child.id());
        let used = || cpu(&std::fs::read_to_string(&stat).unwrap());
        let mut last = used();
        self.until("the daemon to go idle", || {
            std::thread::sleep(Duration::from_millis(250));
            let now = used();
            let idle = now.saturating_sub(last) < Duration::from_millis(20);
            last = now;
            idle.then_some(())
        });
    }

    /// Poll `f` until it returns a value, for at most 10 s. A timeout
    /// fails with the daemon's log.
    pub fn until<T>(&self, what: &str, f: impl FnMut() -> Option<T>) -> T {
        poll(f)
            .unwrap_or_else(|| panic!("timed out waiting for {what}; daemon log:\n{}", self.log()))
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The user and system CPU time in a `/proc` `stat` line.
fn cpu(stat: &str) -> Duration {
    // Fields after the parenthesized command name; utime and stime are
    // the 12th and 13th of them, in clock ticks.
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .map_or(Vec::new(), |(_, rest)| rest.split_whitespace().collect());
    let ticks = |i: usize| {
        fields
            .get(i)
            .and_then(|f| f.parse::<u64>().ok())
            .unwrap_or(0)
    };
    // Linux reports these in USER_HZ: 100 on x86 and ARM.
    Duration::from_millis((ticks(11) + ticks(12)) * 10)
}

/// Poll `f` until it returns a value, for at most 10 s.
pub fn until<T>(what: &str, f: impl FnMut() -> Option<T>) -> T {
    poll(f).unwrap_or_else(|| panic!("timed out waiting for {what}"))
}

/// `f`'s first value, polled every 20 ms for at most 10 s.
fn poll<T>(f: impl FnMut() -> Option<T>) -> Option<T> {
    poll_every(Duration::from_millis(20), f)
}

/// `f`'s first value, polled every `interval` for at most 10 s.
fn poll_every<T>(interval: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = f() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(interval);
    }
}
