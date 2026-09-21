use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            directories::BaseDirs::new().and_then(|b| b.runtime_dir().map(|r| r.to_path_buf()))
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("iris.sock")
}

/// If a daemon is already running, forward these args and exit the
/// process. Returns true when the caller must exit. A bare invocation
/// (no flags) surfaces the library in the running daemon.
///
/// When no daemon answers, a flagged invocation still must not hold
/// the caller's shell: spawn a detached daemon, wait for its socket,
/// forward, and exit. Only a bare `iris` (no args) becomes the daemon
/// in the foreground.
pub fn forward_if_running(args: &[String]) -> bool {
    let path = socket_path();
    if let Ok(mut stream) = UnixStream::connect(&path) {
        let effective: &[String] = if args.is_empty() {
            &["--home".to_string()]
        } else {
            args
        };
        let payload = effective.join("\n");
        return stream.write_all(payload.as_bytes()).is_ok();
    }
    // No daemon answered. A bare launch becomes the daemon below; a
    // flagged launch spawns one detached and forwards, so `iris
    // --library` against a crashed or never-started daemon returns
    // instead of blocking on a long-lived process.
    if args.is_empty() {
        return false;
    }
    spawn_detached_daemon() && forward_when_ready(&path, args)
}

/// Spawn the daemon as a detached child: own process group, stdio to
/// null, so it survives this process's exit and never holds the
/// caller's terminal. The child runs `main` with no args, fails its
/// own forward probe, and becomes the daemon.
fn spawn_detached_daemon() -> bool {
    use std::os::unix::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    std::process::Command::new(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .is_ok()
}

/// Forward `args` once the freshly spawned daemon's socket accepts.
/// The child binds early in `start`, but not synchronously, so poll
/// the connect for a few seconds rather than assume readiness.
fn forward_when_ready(path: &PathBuf, args: &[String]) -> bool {
    let payload = args.join("\n");
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if let Ok(mut stream) = UnixStream::connect(path) {
            return stream.write_all(payload.as_bytes()).is_ok();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

pub(super) fn bind_socket() -> Result<UnixListener, String> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener =
        UnixListener::bind(&path).map_err(|e| format!("bind {}: {e}", path.display()))?;
    // The socket drives screen capture; only this user may connect.
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("nonblocking socket: {e}"))?;
    Ok(listener)
}

pub(super) fn accept_args(listener: &UnixListener) -> Vec<String> {
    let mut args = Vec::new();
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // The accepted socket does not inherit O_NONBLOCK, and a
                // client that connects but never writes or closes would
                // stall every later command on a blocking read. Bound
                // the read: 2s of poll, then give up on the peer.
                let mut buf: Vec<u8> = Vec::new();
                {
                    use std::io::Read;
                    use std::os::unix::io::AsRawFd;
                    let fd = stream.as_raw_fd();
                    let mut chunk = [0u8; 4096];
                    let started = std::time::Instant::now();
                    // Read until the peer closes (forward_if_running
                    // drops its stream after the write) or 2s passes
                    // with no data. Payloads are argv lines; 1MiB caps
                    // a hostile flood.
                    loop {
                        let mut pfd = libc::pollfd {
                            fd,
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        if unsafe { libc::poll(&mut pfd, 1, 2000) } <= 0 {
                            break;
                        }
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&chunk[..n]);
                                // 1MiB caps a hostile flood; the 5s
                                // total deadline caps a slow drip that
                                // keeps poll() fed forever.
                                if buf.len() > (1 << 20)
                                    || started.elapsed() > Duration::from_secs(5)
                                {
                                    break;
                                }
                            }
                        }
                    }
                }
                // Decode once: a multi-byte char split across the 4KiB
                // read boundary would corrupt into U+FFFD under
                // per-chunk from_utf8_lossy.
                let text = String::from_utf8_lossy(&buf);
                args.extend(
                    text.lines()
                        .map(|l| l.to_string())
                        .filter(|l| !l.is_empty()),
                );
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    args
}
