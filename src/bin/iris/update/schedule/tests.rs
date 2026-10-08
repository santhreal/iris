use std::time::UNIX_EPOCH;

use super::*;
use crate::update::tests::{info, ASSET};

// WHY: the class closed here is "the background check runs at the
// wrong time, too often, or not at all, or reports a failure more than
// once": the first check waits a minute from the start, a check runs
// once a day, a restart inside the day does not check again, a change
// of channel checks at once, `check_for_updates = false` checks never
// and withdraws the offer, a run of failures retries hourly and writes
// one line, and each version is announced once across restarts. The
// clock is the `now` each pass receives, advanced by the wait the pass
// before returned, as `spawn` sleeps it: no test sleeps. Not covered:
// `spawn`'s thread and real clock.

/// The daemon's start, in whole seconds as update.json keeps them.
fn start() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(60 * 60);
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn running() -> semver::Version {
    semver::Version::new(0, 1, 0)
}

fn offer(version: &str) -> UpdateInfo {
    UpdateInfo {
        version: semver::Version::parse(version).expect(version),
        ..info("https://dl.example", ASSET)
    }
}

/// A scheduler of a daemon started at `at`, with update.json in `dir`.
fn daemon(dir: &tempfile::TempDir, at: SystemTime) -> Scheduler {
    Scheduler::new(dir.path().join("update.json"), at + START_DELAY)
}

/// What a run of passes did.
#[derive(Debug)]
struct Run {
    /// When each check ran, as the time since `start()`.
    checks: Vec<Duration>,
    /// Each offer published, with the time since `start()`.
    published: Vec<(Duration, Publish)>,
    /// Each line for iris.log.
    logged: Vec<String>,
    /// The clock after the last pass.
    end: SystemTime,
}

/// Passes of `scheduler` from `from` until the clock reaches `until`,
/// each at the time the one before said, the way `spawn` runs them.
/// `answer` answers each check by its time since `start()`. Every wait
/// is within `MIN_WAIT..=POLL`, so the run ends.
fn run(
    scheduler: &mut Scheduler,
    from: SystemTime,
    until: SystemTime,
    enabled: bool,
    channel: UpdateChannel,
    answer: impl Fn(Duration) -> Result<Option<UpdateInfo>, String>,
) -> Run {
    let since = |t: SystemTime| t.duration_since(start()).expect("after the start");
    let mut checks = Vec::new();
    let mut logged = Vec::new();
    let mut published = Vec::new();
    let mut now = from;
    let bound = until.duration_since(from).expect("forwards").as_secs() / MIN_WAIT.as_secs() + 1;
    for _ in 0..bound {
        if now >= until {
            break;
        }
        let (wait, publish) = scheduler.pass(
            now,
            &running(),
            enabled,
            channel,
            |on| {
                assert_eq!(on, channel, "the check ran on another channel");
                checks.push(since(now));
                answer(since(now))
            },
            |line| logged.push(line),
        );
        assert!(
            (MIN_WAIT..=POLL).contains(&wait),
            "a wait of {wait:?} at {:?}",
            since(now)
        );
        if let Some(publish) = publish {
            published.push((since(now), publish));
        }
        now += wait;
    }
    assert!(now >= until, "the run did not reach its end");
    Run {
        checks,
        published,
        logged,
        end: now,
    }
}

fn nothing(_: Duration) -> Result<Option<UpdateInfo>, String> {
    Ok(None)
}

#[test]
fn the_first_check_runs_a_minute_after_the_start() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut scheduler = daemon(&dir, start());
    let mut checked = false;
    let (wait, _) = scheduler.pass(
        start(),
        &running(),
        true,
        UpdateChannel::Stable,
        |_| {
            checked = true;
            Ok(None)
        },
        |line| panic!("logged {line}"),
    );
    assert!(!checked, "a check ran at the start");
    assert_eq!(wait, START_DELAY);

    let run = run(
        &mut scheduler,
        start() + wait,
        start() + 2 * MINUTE,
        true,
        UpdateChannel::Stable,
        nothing,
    );
    assert_eq!(run.checks, [MINUTE]);
}

#[test]
fn a_check_runs_once_a_day() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut scheduler = daemon(&dir, start());
    let run = run(
        &mut scheduler,
        start(),
        start() + 3 * DAY + HOUR,
        true,
        UpdateChannel::Stable,
        nothing,
    );
    assert_eq!(
        run.checks,
        [MINUTE, MINUTE + DAY, MINUTE + 2 * DAY, MINUTE + 3 * DAY]
    );
    assert!(run.logged.is_empty(), "{:?}", run.logged);
}

#[test]
fn a_restart_within_a_day_does_not_check_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = run(
        &mut daemon(&dir, start()),
        start(),
        start() + 2 * HOUR,
        true,
        UpdateChannel::Stable,
        nothing,
    );
    assert_eq!(first.checks, [MINUTE]);

    let restart = first.end;
    let second = run(
        &mut daemon(&dir, restart),
        restart,
        start() + DAY + 2 * HOUR,
        true,
        UpdateChannel::Stable,
        nothing,
    );
    assert_eq!(second.checks, [MINUTE + DAY]);

    // A restart after the day checks a minute after it starts.
    let late = start() + 3 * DAY;
    let third = run(
        &mut daemon(&dir, late),
        late,
        late + HOUR,
        true,
        UpdateChannel::Stable,
        nothing,
    );
    assert_eq!(third.checks, [3 * DAY + MINUTE]);
}

#[test]
fn a_change_of_channel_checks_at_the_next_pass() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut scheduler = daemon(&dir, start());
    let stable = run(
        &mut scheduler,
        start(),
        start() + HOUR,
        true,
        UpdateChannel::Stable,
        nothing,
    );
    assert_eq!(stable.checks, [MINUTE]);
    let beta = run(
        &mut scheduler,
        stable.end,
        stable.end + HOUR,
        true,
        UpdateChannel::Beta,
        nothing,
    );
    assert_eq!(
        beta.checks,
        [stable.end.duration_since(start()).expect("after")]
    );
}

#[test]
fn checks_off_never_check_and_withdraw_the_offer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut scheduler = daemon(&dir, start());
    let off = run(
        &mut scheduler,
        start(),
        start() + 2 * DAY,
        false,
        UpdateChannel::Stable,
        |_| panic!("a check ran while checks are off"),
    );
    assert!(off.checks.is_empty());
    assert!(off.published.is_empty(), "{:?}", off.published);
    assert!(
        !dir.path().join("update.json").exists(),
        "update.json was written"
    );

    let on = run(
        &mut scheduler,
        off.end,
        off.end + HOUR,
        true,
        UpdateChannel::Stable,
        |_| Ok(Some(offer("9.9.9"))),
    );
    assert_eq!(on.checks.len(), 1, "{:?}", on.checks);
    assert_eq!(
        on.published
            .iter()
            .map(|(_, p)| p.clone())
            .collect::<Vec<_>>(),
        [Publish {
            info: Some(offer("9.9.9")),
            announce: true,
        }]
    );

    let off_again = run(
        &mut scheduler,
        on.end,
        on.end + DAY,
        false,
        UpdateChannel::Stable,
        |_| panic!("a check ran while checks are off"),
    );
    assert_eq!(
        off_again
            .published
            .iter()
            .map(|(at, p)| (*at, p.clone()))
            .collect::<Vec<_>>(),
        [(
            on.end.duration_since(start()).expect("after"),
            Publish {
                info: None,
                announce: false,
            }
        )]
    );
}

#[test]
fn a_run_of_failures_retries_hourly_and_logs_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut scheduler = daemon(&dir, start());
    let down = |_: Duration| -> Result<Option<UpdateInfo>, String> {
        Err("update: GET https://api.github.com/...: offline".to_string())
    };
    let failing = run(
        &mut scheduler,
        start(),
        start() + 5 * HOUR,
        true,
        UpdateChannel::Stable,
        down,
    );
    assert_eq!(
        failing.checks,
        [
            MINUTE,
            MINUTE + HOUR,
            MINUTE + 2 * HOUR,
            MINUTE + 3 * HOUR,
            MINUTE + 4 * HOUR
        ]
    );
    assert_eq!(
        failing.logged,
        ["iris: update check failed, retrying every 60 min: \
          update: GET https://api.github.com/...: offline"]
    );
    assert!(
        failing.published.is_empty(),
        "a failure published {:?}",
        failing.published
    );
    assert!(
        !dir.path().join("update.json").exists(),
        "a failed check was recorded"
    );

    let back = run(
        &mut scheduler,
        failing.end,
        failing.end + 2 * HOUR,
        true,
        UpdateChannel::Stable,
        nothing,
    );
    assert_eq!(back.checks, [MINUTE + 5 * HOUR]);
    assert_eq!(back.logged, ["iris: update check: GitHub answered again"]);

    let again = run(
        &mut scheduler,
        back.end,
        start() + DAY + 6 * HOUR + 2 * MINUTE,
        true,
        UpdateChannel::Stable,
        down,
    );
    assert_eq!(
        again.checks,
        [MINUTE + 5 * HOUR + DAY, MINUTE + 6 * HOUR + DAY]
    );
    assert_eq!(again.logged.len(), 1, "{:?}", again.logged);
}

#[test]
fn each_version_is_announced_once_across_restarts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let published = |run: &Run| {
        run.published
            .iter()
            .map(|(_, p)| p.clone())
            .collect::<Vec<_>>()
    };
    let found = |version: &'static str| {
        move |_: Duration| -> Result<Option<UpdateInfo>, String> { Ok(Some(offer(version))) }
    };

    let first = run(
        &mut daemon(&dir, start()),
        start(),
        start() + 2 * DAY,
        true,
        UpdateChannel::Stable,
        found("9.9.9"),
    );
    assert_eq!(first.checks.len(), 2, "{:?}", first.checks);
    assert_eq!(
        published(&first),
        [Publish {
            info: Some(offer("9.9.9")),
            announce: true,
        }]
    );

    // The tray of a restarted daemon shows the offer again, and no
    // notice repeats it.
    let restart = first.end;
    let second = run(
        &mut daemon(&dir, restart),
        restart,
        restart + HOUR,
        true,
        UpdateChannel::Stable,
        found("9.9.9"),
    );
    assert_eq!(
        published(&second),
        [Publish {
            info: Some(offer("9.9.9")),
            announce: false,
        }]
    );

    let newer = run(
        &mut daemon(&dir, second.end),
        second.end,
        second.end + 2 * DAY,
        true,
        UpdateChannel::Stable,
        found("9.9.10"),
    );
    assert_eq!(
        published(&newer),
        [
            Publish {
                info: Some(offer("9.9.9")),
                announce: false,
            },
            Publish {
                info: Some(offer("9.9.10")),
                announce: true,
            },
        ]
    );
}

/// An offer that is not newer than the running iris, or a prerelease
/// on stable, is not published.
#[test]
fn an_offer_the_running_iris_does_not_take_is_not_published() {
    for (channel, version) in [
        (UpdateChannel::Stable, "0.1.0"),
        (UpdateChannel::Beta, "0.0.9"),
        (UpdateChannel::Stable, "9.9.9-beta.1"),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = run(
            &mut daemon(&dir, start()),
            start(),
            start() + HOUR,
            true,
            channel,
            |_| Ok(Some(offer(version))),
        );
        assert_eq!(run.checks, [MINUTE], "{channel:?} {version}");
        assert!(
            run.published.is_empty(),
            "{channel:?} {version}: {:?}",
            run.published
        );
    }
}
