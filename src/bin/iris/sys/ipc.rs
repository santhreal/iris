//! Single-instance IPC over a local socket.
//!
//! The first process to bind the well-known name becomes the daemon;
//! every later invocation forwards its argv and exits. `interprocess`
//! maps the namespaced name to a Unix domain socket on Unix and a named
//! pipe (`\\.\pipe\iris`) on Windows, so this one file serves every OS.
//!
//! The listener runs a dedicated blocking-accept thread that pushes each
//! connection's argv onto a channel. That replaces the old design that
//! `poll()`ed the listener fd inside the GPUI executor: a blocking
//! thread is the same shape on every platform and needs no raw fd.

use std::io::{Read, Write};
use std::time::{Duration, Instant};
use futures::channel::mpsc::{self, UnboundedReceiver};
use interprocess::local_socket::{
    prelude::*, GenericFilePath, ListenerOptions, Name,
};

/// The well-known socket name. A filesystem path on Unix (the runtime
/// dir holds `iris.sock`, mode 0600), a `\\.\pipe\` name on Windows —
/// `GenericFilePath` maps each to the platform's local socket.
#[cfg(unix)]
fn socket_name() -> Name<'static> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir());
    dir.join("iris.sock")
        .into_os_string()
        .to_fs_name::<GenericFilePath>()
        .expect("iris.sock is a valid local socket path")
}

/// Windows: the named pipe `\\.\pipe\iris`.
#[cfg(windows)]
fn socket_name() -> Name<'static> {
    r"\\.\pipe\iris"
        .to_fs_name::<GenericFilePath>()
        .expect("iris is a valid named pipe name")
}

/// If a daemon is already running, forward these args and exit the
/// process. Returns true when the caller must exit. A bare invocation
/// (no flags) surfaces the home window in the running daemon.
///
/// When no daemon answers, a flagged invocation still must not hold the
/// caller's shell: spawn a detached daemon, wait for its socket, forward,
/// and exit. Only a bare `iris` (no args) becomes the daemon in the
/// foreground.
pub fn forward_if_running(args: &[String]) -> bool {
    if let Ok(mut stream) = LocalSocketStream::connect(socket_name()) {
        let effective: &[String] = if args.is_empty() {
            &["--home".to_string()]
        } else {
            args
        };
        let payload = effective.join("\n");
        return stream.write_all(payload.as_bytes()).is_ok();
    }
    if args.is_empty() {
        return false;
    }
    spawn_detached_daemon() && forward_when_ready(args)
}

/// Spawn the daemon as a detached child: stdio to null, no console
/// window on Windows, so it survives this process's exit and never
/// holds the caller's terminal. The child runs `main` with no args,
/// fails its own forward probe, and becomes the daemon.
fn spawn_detached_daemon() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Unix: own process group so the daemon is not killed with the
    // caller's session. Windows: detached, no new console.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    cmd.spawn().is_ok()
}

/// Forward `args` once the freshly spawned daemon's socket accepts. The
/// child binds early in `start`, but not synchronously, so poll the
/// connect for a few seconds rather than assume readiness.
fn forward_when_ready(args: &[String]) -> bool {
    let payload = args.join("\n");
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if let Ok(mut stream) = LocalSocketStream::connect(socket_name()) {
            return stream.write_all(payload.as_bytes()).is_ok();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Bind the local socket and spawn the accept thread. Each connection's
/// argv (newline-separated) is pushed onto the returned receiver as a
/// `Vec<String>`. The daemon drains it on the command pump.
///
/// `reclaim_name` + `try_overwrite` let a fresh daemon take over the
/// socket file a SIGKILLed predecessor left behind; on Windows the name
/// is a kernel object that dies with the process, so there is nothing
/// stale to reclaim. On Unix the socket file gets mode 0600: it drives
/// screen capture, so only this user may connect.
pub fn spawn_listener() -> Result<UnboundedReceiver<Vec<String>>, String> {
    let opts = ListenerOptions::new()
        .name(socket_name())
        .reclaim_name(true)
        .try_overwrite(true);
    #[cfg(unix)]
    let opts = {
        use interprocess::os::unix::local_socket::ListenerOptionsExt;
        opts.mode(0o600)
    };
    let listener = opts
        .create_sync()
        .map_err(|e| format!("bind local socket: {e}"))?;

    let (tx, rx) = mpsc::unbounded();
    std::thread::Builder::new()
        .name("iris-ipc".into())
        .spawn(move || accept_loop(listener, tx))
        .map_err(|e| format!("spawn ipc thread: {e}"))?;
    Ok(rx)
}

/// Blocking accept loop: one connection at a time, each read to EOF
/// with a bounded drip deadline, then the decoded argv goes to the
/// daemon. A client that connects but never writes cannot stall later
/// commands: the read deadline caps it.
fn accept_loop(
    listener: LocalSocketListener,
    tx: futures::channel::mpsc::UnboundedSender<Vec<String>>,
) {
    for conn in listener.incoming() {
        let Ok(mut stream) = conn else {
            continue;
        };
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        let started = Instant::now();
        // Read until the peer closes (forward_if_running drops its
        // stream after the write) or the drip deadline passes. 1MiB caps
        // a hostile flood; 5s caps a slow drip.
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.len() > (1 << 20) || started.elapsed() > Duration::from_secs(5) {
                        break;
                    }
                }
            }
        }
        // Decode once: a multi-byte char split across the 4KiB read
        // boundary would corrupt into U+FFFD under per-chunk lossy.
        let text = String::from_utf8_lossy(&buf);
        let args: Vec<String> = text
            .lines()
            .map(|l| l.to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if !args.is_empty() {
            let _ = tx.unbounded_send(args);
        }
    }
}
