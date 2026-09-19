//! Logging: every diagnostic line goes to stderr AND a rolling log
//! file under the state dir, so a daemon detached from a terminal
//! still leaves a trail. The file is truncated at 256 KiB on open so
//! a long-lived daemon cannot grow it without bound.

use std::io::Write;
use std::path::PathBuf;
use parking_lot::Mutex;

static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
const MAX_LOG: u64 = 256 * 1024;

/// Path of the log file: `$XDG_STATE_HOME/iris/iris.log`, falling
/// back to the config dir's parent so the file always lands beside
/// the rest of iris's state.
pub fn path() -> PathBuf {
    let base = directories::BaseDirs::new()
        .and_then(|b| b.state_dir().map(|p| p.to_path_buf()))
        .or_else(|| directories::BaseDirs::new().map(|b| b.config_dir().to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("iris").join("iris.log")
}

/// Open (and bound) the log file. Idempotent; called once at daemon
/// start. Failure is silent: stderr still carries every line.
pub fn init() {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Truncate when the existing log is already over the cap.
    if let Ok(meta) = std::fs::metadata(&p) {
        if meta.len() > MAX_LOG {
            let _ = std::fs::remove_file(&p);
        }
    }
    if let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        *LOG_FILE.lock() = Some(f);
    }
}

/// Write one line to stderr and the log file.
pub fn line(msg: &str) {
    eprintln!("{msg}");
    if let Some(f) = LOG_FILE.lock().as_mut() {
        let now = chrono::Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(f, "[{now}] {msg}");
    }
}

/// `ilog!("iris: ...")` — stderr plus the log file.
#[macro_export]
macro_rules! ilog {
    ($($arg:tt)*) => {
        $crate::log::line(&format!($($arg)*))
    };
}
