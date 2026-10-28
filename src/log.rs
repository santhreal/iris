//! Logging: every diagnostic line goes to stderr AND a rolling log
//! file under the state dir, so a daemon detached from a terminal
//! still leaves a trail. The file is truncated at 256 KiB on open so
//! a long-lived daemon cannot grow it without bound.
//!
//! The libraries' records (the `log` crate's: GPUI's GPU and windowing
//! failures, wgpu's, zbus's) at warning level and above are written as
//! lines of their own, prefixed with the record's target. A panic on any
//! thread is written as one line with its thread, location, and message
//! before the default hook prints it to stderr.

use parking_lot::Mutex;
use std::io::{Seek, Write};
use std::path::PathBuf;

static LOG_FILE: Mutex<Option<(std::fs::File, u64)>> = Mutex::new(None);
const MAX_LOG: u64 = 256 * 1024;

/// Path of the log file; see [`crate::dirs::log_file`].
pub fn path() -> PathBuf {
    crate::dirs::log_file()
}

/// Open (and bound) the log file, route the libraries' warnings and
/// errors to it, and journal panics to it. Idempotent; called once at
/// start. Failure is silent: stderr still carries every line.
pub fn init() {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Truncate when the existing log is already over the cap.
    let mut len = 0;
    if let Ok(meta) = std::fs::metadata(&p) {
        if meta.len() > MAX_LOG {
            let _ = std::fs::remove_file(&p);
        } else {
            len = meta.len();
        }
    }
    if let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        *LOG_FILE.lock() = Some((f, len));
    }
    if log::set_logger(&RECORDS).is_ok() {
        log::set_max_level(RECORDS_LEVEL);
    }
    install_panic_hook();
}

/// Chain [`journal_panic`] in front of the panic hook in place, once per
/// process.
fn install_panic_hook() {
    static PANIC_HOOK: std::sync::Once = std::sync::Once::new();
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            journal_panic(info);
            previous(info);
        }));
    });
}

/// Write one line to stderr and the log file. Past the cap the file
/// restarts from empty: a daemon that runs for weeks cannot grow the
/// log without bound.
pub fn line(msg: &str) {
    eprintln!("{msg}");
    file_line(msg);
}

/// Write one line to the log file only: a degraded path the command
/// still completes on, which a command line's output does not report.
pub fn file_line(msg: &str) {
    write_line(&mut LOG_FILE.lock(), msg);
}

fn write_line(guard: &mut Option<(std::fs::File, u64)>, msg: &str) {
    if let Some((f, len)) = guard.as_mut() {
        let now = crate::time::log_stamp();
        let line = format!("[{now}] {msg}\n");
        if *len + line.len() as u64 > MAX_LOG {
            // Rotate in place: truncate and restart rather than unlink,
            // so an open reader keeps its fd.
            let _ = f.set_len(0);
            let _ = f.seek(std::io::SeekFrom::Start(0));
            *len = 0;
        }
        if f.write_all(line.as_bytes()).is_ok() {
            *len += line.len() as u64;
        }
    }
}

/// Write a panic to the log file as one line. A panic raised while its
/// own thread holds the file's lock, or while another thread does not
/// release it, is not written: the wait is bounded so the hook cannot
/// deadlock the panicking thread, and stderr still receives it.
fn journal_panic(info: &std::panic::PanicHookInfo) {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("unnamed");
    let at = info
        .location()
        .map_or_else(|| "an unknown location".to_string(), |l| l.to_string());
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a non-text payload");
    let msg = format!(
        "iris: panic on thread '{name}' at {at}: {}",
        message.replace('\n', " ")
    );
    if let Some(mut guard) = LOG_FILE.try_lock_for(std::time::Duration::from_millis(200)) {
        write_line(&mut guard, &msg);
    }
}

/// The least severe library record that is written.
const RECORDS_LEVEL: log::LevelFilter = log::LevelFilter::Warn;

static RECORDS: Records = Records;

/// The `log` crate's logger: a library record at [`RECORDS_LEVEL`] or
/// above becomes one [`line`].
struct Records;

impl log::Log for Records {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= RECORDS_LEVEL
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            line(&format!(
                "{}: {}: {}",
                record.target(),
                record.level(),
                record.args()
            ));
        }
    }

    fn flush(&self) {}
}

/// `ilog!("iris: ...")` — stderr plus the log file.
#[macro_export]
macro_rules! ilog {
    ($($arg:tt)*) => {
        $crate::log::line(&format!($($arg)*))
    };
}

#[cfg(test)]
mod tests;
