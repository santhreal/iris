//! Single-instance IPC over a local socket.
//!
//! One process per socket is the daemon: the one that holds the daemon
//! claim, which it takes before it connects to the display and holds
//! until it exits. Only the claim's holder binds the socket's
//! well-known name; every other invocation forwards its argv to it and
//! exits. `interprocess` maps the name to a Unix domain socket on Unix
//! and a named pipe on Windows. The per-OS module makes the claim,
//! names, binds, and connects to the socket, and is its access control;
//! this file is the protocol, the same on every OS.
//!
//! A blocking accept thread hands each connection to a reader thread of
//! its own, which pushes the connection's argv onto a channel. A client
//! that connects and sends nothing, or sends slowly, holds only its own
//! reader and delays no other command.

use futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use interprocess::local_socket::{prelude::*, Name};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(test)]
mod tests;

/// How long a client waits for a daemon that is starting to bind the
/// socket.
const READY_TIMEOUT: Duration = Duration::from_secs(8);

/// What one connection may cost the daemon.
#[derive(Clone, Copy)]
struct Limits {
    /// Connections read at once. The accept thread closes a connection
    /// past this unread, so clients that never close cannot pile up
    /// reader threads.
    readers: usize,
    /// Longest command line, in bytes.
    line: usize,
    /// Time from the accept to the client's close. A command line still
    /// arriving after it is dropped when the next read returns. Named
    /// pipes take no read timeout, so on every OS a client that sends
    /// nothing holds its reader until it closes.
    drip: Duration,
}

const LIMITS: Limits = Limits {
    readers: 16,
    line: 1 << 20,
    drip: Duration::from_secs(5),
};

/// Proof that this process holds the daemon claim (see `claim`): the
/// socket binds only with it.
pub struct Claimed(());

/// Take the daemon claim, held until this process exits, however it
/// exits: the OS drops it with the process. Of any number of processes
/// that start at once, one takes it, and only that one binds the
/// socket, so the socket file a bind replaces on Unix is a dead
/// daemon's. `Ok(None)`: another process holds the claim, a daemon that
/// runs or one that is starting.
pub fn claim() -> Result<Option<Claimed>, String> {
    let Some(held) = imp::claim(&imp::claim_name()?)? else {
        return Ok(None);
    };
    std::mem::forget(held);
    Ok(Some(Claimed(())))
}

/// Deliver `args` to the daemon, or make this process the daemon.
/// `Ok(None)`: delivered, the caller exits. `Ok(Some(_))`: no daemon
/// runs and this process holds the claim: it becomes the daemon in the
/// foreground and runs `args` itself. A bare invocation (no flags)
/// surfaces the home window in a running daemon.
///
/// When no daemon answers, a flagged invocation still must not hold the
/// caller's shell: it starts a detached `iris --daemon`, waits for the
/// socket, and forwards. A spawned daemon that never binds is an error,
/// not a cue to start a second daemon in the foreground: the first may
/// still bind. A process that finds the claim held, by a daemon that is
/// starting, forwards to that daemon once it binds.
pub fn forward_if_running(args: &[String]) -> Result<Option<Claimed>, String> {
    let name = imp::socket_name()?;
    let effective: &[String] = if args.is_empty() {
        &["--home".to_string()]
    } else {
        args
    };
    match imp::connect(name.borrow()) {
        Ok(mut stream) => {
            return stream
                .write_all(effective.join("\n").as_bytes())
                .map(|()| None)
                .map_err(|e| format!("iris: send to the running daemon: {e}"));
        }
        // The name is bound and this account may not use it, so no
        // daemon this process starts could bind it either.
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            return Err(format!("iris: connect to the daemon: {e}"));
        }
        Err(_) => {}
    }
    // A bare `iris` becomes the daemon itself, as does a flagged one
    // that could not start a detached daemon.
    if args.is_empty() || !spawn_daemon() {
        if let Some(claimed) = claim()? {
            return Ok(Some(claimed));
        }
    }
    if forward_when_ready(&name, effective) {
        return Ok(None);
    }
    Err(format!(
        "iris: the daemon did not start within {}s; see {}",
        READY_TIMEOUT.as_secs(),
        iris_lib::dirs::log_file().display()
    ))
}

/// Send `args` to a running daemon without spawning one. Returns true
/// only when a daemon accepted the payload. Unlike
/// `forward_if_running`, a missing daemon is a no-op — used by the
/// updater to stop the old daemon before replacing the binary.
pub fn send_to_daemon(args: &[String]) -> bool {
    imp::socket_name().is_ok_and(|name| send(name, args))
}

/// Write `args` to the daemon listening on `name`, one per line.
fn send(name: Name<'_>, args: &[String]) -> bool {
    imp::connect(name).is_ok_and(|mut stream| stream.write_all(args.join("\n").as_bytes()).is_ok())
}

/// Send `--quit` to the running daemon and wait until it exits, for at
/// most `within`. `true` once no daemon answers the socket: it exited,
/// or none ran. A quitting daemon saves its recording first and
/// answers until it exits. The updater runs this before the file swap,
/// so the binary is free to replace.
pub fn quit_daemon(within: Duration) -> bool {
    let Ok(name) = imp::socket_name() else {
        return true;
    };
    quit(name.borrow(), within)
}

/// Send `--quit` to `name`, then poll until nothing answers it, for at
/// most `within`. A `--quit` that does not land leaves the daemon
/// answering, and the wait fails.
fn quit(name: Name<'_>, within: Duration) -> bool {
    send(name.borrow(), &["--quit".to_string()]);
    let deadline = Instant::now() + within;
    loop {
        if imp::connect(name.borrow()).is_err() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Start this executable as a detached `iris --daemon` (see
/// `sys::detach`), which survives this process's exit and holds none of
/// its streams. It takes the claim and becomes the daemon, or exits when
/// another process holds the claim.
fn spawn_daemon() -> bool {
    match std::env::current_exe().and_then(|exe| crate::sys::detach::spawn(&exe, &["--daemon"])) {
        Ok(()) => true,
        Err(e) => {
            iris_lib::ilog!("iris: start the daemon: {e}");
            false
        }
    }
}

/// Forward `args` once the daemon that is starting binds the socket. It
/// binds only after it has connected to the display and grabbed its
/// hotkeys, so poll the connect for a few seconds rather than assume
/// readiness.
fn forward_when_ready(name: &Name<'_>, args: &[String]) -> bool {
    let payload = args.join("\n");
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(mut stream) = imp::connect(name.borrow()) {
            return stream.write_all(payload.as_bytes()).is_ok();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Bind the daemon's socket and spawn the accept thread. Each
/// connection's argv (newline-separated) is pushed onto the returned
/// receiver as a `Vec<String>`. The daemon drains it on the command
/// pump. The bind takes the claim: only its holder may replace the
/// socket file.
pub fn spawn_listener(_: Claimed) -> Result<UnboundedReceiver<Vec<String>>, String> {
    listen(imp::socket_name()?, LIMITS)
}

/// Bind `name` and spawn the accept thread.
fn listen(name: Name<'static>, limits: Limits) -> Result<UnboundedReceiver<Vec<String>>, String> {
    let listener = imp::bind(name)?;
    let (tx, rx) = mpsc::unbounded();
    std::thread::Builder::new()
        .name("iris-ipc".into())
        .spawn(move || accept_loop(&listener, &tx, limits))
        .map_err(|e| format!("spawn ipc thread: {e}"))?;
    Ok(rx)
}

/// Accept connections for the life of the daemon, each read on a thread
/// of its own, at most `limits.readers` at once.
fn accept_loop(listener: &LocalSocketListener, tx: &UnboundedSender<Vec<String>>, limits: Limits) {
    let reading = Arc::new(AtomicUsize::new(0));
    for conn in listener.incoming() {
        let Ok(stream) = conn else {
            continue;
        };
        let accepted = Instant::now();
        if reading.fetch_add(1, Ordering::AcqRel) >= limits.readers {
            reading.fetch_sub(1, Ordering::AcqRel);
            iris_lib::ilog!(
                "iris: {} clients are still sending; closed a new connection unread",
                limits.readers
            );
            continue;
        }
        let (tx, slot) = (tx.clone(), Arc::clone(&reading));
        let reader = std::thread::Builder::new()
            .name("iris-ipc-read".into())
            .spawn(move || {
                let args = read_command(stream, accepted, limits);
                // Before the send: whoever sees the command line land
                // sees this reader's slot free.
                slot.fetch_sub(1, Ordering::AcqRel);
                if !args.is_empty() {
                    let _ = tx.unbounded_send(args);
                }
            });
        if let Err(e) = reader {
            reading.fetch_sub(1, Ordering::AcqRel);
            iris_lib::ilog!("iris: spawn an ipc reader: {e}");
        }
    }
}

/// The lines a client sends before it closes, the empty ones dropped.
/// A command line that runs past `limits.line` bytes, is still arriving
/// `limits.drip` after `accepted`, or ends in a failed read is dropped
/// whole: running the part that arrived would run a command no client
/// sent.
fn read_command(mut stream: LocalSocketStream, accepted: Instant, limits: Limits) -> Vec<String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = match stream.read(&mut chunk) {
            Ok(n) => n,
            Err(e) => {
                iris_lib::ilog!("iris: dropped a forwarded command line: read: {e}");
                return Vec::new();
            }
        };
        if n == 0 && buf.is_empty() {
            return Vec::new();
        }
        if accepted.elapsed() > limits.drip {
            iris_lib::ilog!(
                "iris: dropped a forwarded command line still arriving after {:?}",
                limits.drip
            );
            return Vec::new();
        }
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > limits.line {
            iris_lib::ilog!(
                "iris: dropped a forwarded command line over {} bytes",
                limits.line
            );
            return Vec::new();
        }
    }
    // Decode once: a multi-byte char split across the 4KiB read
    // boundary would corrupt into U+FFFD under per-chunk lossy.
    String::from_utf8_lossy(&buf)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}
