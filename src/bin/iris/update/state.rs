//! What the update check keeps across daemon restarts, in `update.json`
//! beside iris.log: when the last check got an answer and on which
//! channel, the update it found, and the newest version an update
//! notice announced. A check within a day of the last one on the same
//! channel does not run, and a version is announced once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iris_lib::config::UpdateChannel;
use serde::{Deserialize, Serialize};

use super::UpdateInfo;

/// How long a check's answer holds.
pub(super) const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// The file's format. A file of another format reads as no state, so
/// the next check runs and replaces it.
const FORMAT: u32 = 1;

/// Read-modify-write of the file within this process: the background
/// check and a Settings check run on different threads of the daemon.
static WRITE: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub(super) struct State {
    format: u32,
    /// Seconds since the Unix epoch of the last check that got an
    /// answer.
    pub checked_at: Option<u64>,
    /// The channel that check ran on.
    pub channel: Option<UpdateChannel>,
    /// The update that check found.
    pub offer: Option<UpdateInfo>,
    /// The newest version an update notice announced.
    pub announced: Option<String>,
}

/// `update.json`, beside iris.log.
pub(super) fn path() -> PathBuf {
    iris_lib::dirs::log_file().with_file_name("update.json")
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl State {
    /// The state in the file at `path`. A missing, unreadable, or
    /// malformed file, or one of another format, reads as no state.
    pub fn load(path: &Path) -> State {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<State>(&bytes).ok())
            .filter(|state| state.format == FORMAT)
            .unwrap_or_default()
    }

    /// How long until a check on `channel` is due at `now`: zero when
    /// no check got an answer, the last one ran on another channel, it
    /// is `INTERVAL` old, or it is dated after `now` (a clock set back).
    pub fn until_due(&self, channel: UpdateChannel, now: SystemTime) -> Duration {
        let Some(at) = self.checked_at else {
            return Duration::ZERO;
        };
        if self.channel != Some(channel) {
            return Duration::ZERO;
        }
        let now = secs(now);
        if at > now {
            return Duration::ZERO;
        }
        INTERVAL.saturating_sub(Duration::from_secs(now - at))
    }

    /// The update this state offers on `channel` to an iris of version
    /// `current`: the one the last check found, when it is newer than
    /// `current`, and a stable release unless `channel` is beta.
    pub fn offer_for(
        &self,
        channel: UpdateChannel,
        current: &semver::Version,
    ) -> Option<UpdateInfo> {
        self.offer
            .as_ref()
            .filter(|info| info.version > *current)
            .filter(|info| channel == UpdateChannel::Beta || info.version.pre.is_empty())
            .cloned()
    }
}

/// Apply `change` to the state in the file at `path` and write it back.
fn update(path: &Path, change: impl FnOnce(&mut State)) -> Result<(), String> {
    let _held = WRITE.lock();
    let mut state = State::load(path);
    change(&mut state);
    state.format = FORMAT;
    let text = serde_json::to_vec_pretty(&state)
        .map_err(|e| format!("update: serialize {}: {e}", path.display()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("update: create {}: {e}", dir.display()))?;
    }
    // A rename replaces the file whole: a reader never sees half of it.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, text).map_err(|e| format!("update: write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("update: replace {}: {e}", path.display())
    })
}

/// Record a check on `channel` at `now` that found `found`.
pub(super) fn record_check(
    path: &Path,
    channel: UpdateChannel,
    found: Option<&UpdateInfo>,
    now: SystemTime,
) -> Result<(), String> {
    update(path, |state| {
        state.checked_at = Some(secs(now));
        state.channel = Some(channel);
        state.offer = found.cloned();
    })
}

/// Record that an update notice announced `version`.
pub(super) fn record_announced(path: &Path, version: &semver::Version) -> Result<(), String> {
    update(path, |state| state.announced = Some(version.to_string()))
}

#[cfg(test)]
mod tests;
