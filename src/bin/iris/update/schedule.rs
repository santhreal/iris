//! The daemon's background update check. While `check_for_updates` is
//! on, it checks about a minute after the daemon starts and then once a
//! day; `update.json` (`state`) holds the last check, so a restart
//! within a day does not check again. A change of `update_channel`
//! makes a check due. A failed check is retried hourly, and a run of
//! failures writes one line to iris.log. The update a check finds goes
//! to the daemon as `Command::UpdateOffer`: the tray shows an install
//! item for it, and the first offer of each version shows a notice.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use futures::channel::mpsc::UnboundedSender;
use iris_lib::config::{Config, UpdateChannel};

use super::{state, UpdateInfo};
use crate::daemon::Command;

/// The wait from the daemon's start to its first check.
const START_DELAY: Duration = Duration::from_secs(60);

/// The longest wait between passes: a change to `check_for_updates`
/// or `update_channel` takes effect within it.
pub(super) const POLL: Duration = Duration::from_secs(5 * 60);

/// The wait after a failed check before the next one.
pub(super) const RETRY: Duration = Duration::from_secs(60 * 60);

/// The shortest wait between passes.
pub(super) const MIN_WAIT: Duration = Duration::from_secs(1);

/// An update offer for the daemon: the tray shows an install item for
/// `info`, or none for `None`, and `announce` shows the update notice.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Publish {
    pub info: Option<UpdateInfo>,
    pub announce: bool,
}

/// The background check between passes.
pub(super) struct Scheduler {
    state: PathBuf,
    /// No check runs before this.
    not_before: SystemTime,
    /// After a failed check, the next one runs no earlier than this.
    retry_at: Option<SystemTime>,
    /// Whether the last check failed: a run of failures is logged once.
    failing: bool,
    /// The version of the offer last published; `None` for no offer.
    shown: Option<semver::Version>,
}

impl Scheduler {
    pub fn new(state: PathBuf, not_before: SystemTime) -> Self {
        Scheduler {
            state,
            not_before,
            retry_at: None,
            failing: false,
            shown: None,
        }
    }

    /// One pass at `now` for an iris of version `current`: run `check`
    /// on `channel` when `enabled` and a check is due, then return the
    /// wait before the next pass and the offer to publish when it
    /// differs from the one last published. `log` receives the lines
    /// for iris.log.
    pub fn pass(
        &mut self,
        now: SystemTime,
        current: &semver::Version,
        enabled: bool,
        channel: UpdateChannel,
        check: impl FnOnce(UpdateChannel) -> Result<Option<UpdateInfo>, String>,
        mut log: impl FnMut(String),
    ) -> (Duration, Option<Publish>) {
        if !enabled {
            return (POLL, self.publish(None, None));
        }
        let mut state = state::State::load(&self.state);
        let due_at = [
            Some(now + state.until_due(channel, now)),
            Some(self.not_before),
            self.retry_at,
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(now);
        let mut next = due_at;
        if due_at <= now {
            match check(channel) {
                Ok(found) => {
                    if self.failing {
                        log("iris: update check: GitHub answered again".to_string());
                    }
                    self.failing = false;
                    self.retry_at = None;
                    if let Err(e) = state::record_check(&self.state, channel, found.as_ref(), now) {
                        log(format!("iris: update check: {e}"));
                    }
                    state = state::State::load(&self.state);
                    next = now + state::INTERVAL;
                }
                Err(e) => {
                    if !self.failing {
                        log(format!(
                            "iris: update check failed, retrying every {} min: {e}",
                            RETRY.as_secs() / 60
                        ));
                    }
                    self.failing = true;
                    self.retry_at = Some(now + RETRY);
                    next = now + RETRY;
                }
            }
        }
        let wait = next
            .duration_since(now)
            .unwrap_or(Duration::ZERO)
            .clamp(MIN_WAIT, POLL);
        let offer = state.offer_for(channel, current);
        let announced = state.announced.clone();
        let publish = self.publish(offer, announced.as_deref());
        if let Some(Publish {
            info: Some(info),
            announce: true,
        }) = &publish
        {
            if let Err(e) = state::record_announced(&self.state, &info.version) {
                log(format!("iris: update check: {e}"));
            }
        }
        (wait, publish)
    }

    /// The offer to publish: `offer` when it is not the one last
    /// published, announced unless `announced` is its version.
    fn publish(&mut self, offer: Option<UpdateInfo>, announced: Option<&str>) -> Option<Publish> {
        let version = offer.as_ref().map(|info| info.version.clone());
        if version == self.shown {
            return None;
        }
        self.shown = version;
        let announce = offer
            .as_ref()
            .is_some_and(|info| announced != Some(info.version.to_string().as_str()));
        Some(Publish {
            info: offer,
            announce,
        })
    }
}

/// Start the background check on its own thread. It sends each offer
/// to the daemon on `tx` and ends when the daemon's channel closes.
pub(crate) fn spawn(tx: UnboundedSender<Command>) {
    let spawned = std::thread::Builder::new()
        .name("iris-update-check".into())
        .spawn(move || {
            let current = super::current_version();
            let mut scheduler =
                Scheduler::new(state::path(), SystemTime::now() + START_DELAY);
            loop {
                let cfg = Config::load();
                let (wait, publish) = scheduler.pass(
                    SystemTime::now(),
                    &current,
                    cfg.check_for_updates,
                    cfg.update_channel,
                    super::fetch,
                    |line| iris_lib::ilog!("{line}"),
                );
                if let Some(Publish { info, announce }) = publish {
                    if tx
                        .unbounded_send(Command::UpdateOffer { info, announce })
                        .is_err()
                    {
                        return;
                    }
                }
                std::thread::sleep(wait);
            }
        });
    if let Err(e) = spawned {
        iris_lib::ilog!("iris: update check: start its thread: {e}");
    }
}

#[cfg(test)]
mod tests;
