use super::*;
use crate::update::tests::{info, ASSET};

// WHY: the class closed here is "update.json resurrects or loses what
// the last check found": a recorded check reads back whole, an
// announcement keeps the check beside it, a missing, corrupt, partial,
// or other-format file reads as no state (so the next check runs and
// replaces it), a write never leaves half a file or a temporary file,
// and the due time and the offer follow the channel, the clock, and
// the running version. Not covered: a second process writing the file
// at once; the daemon is the one writer.

/// Whole seconds since the epoch, the precision the file keeps.
fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

const T: u64 = 1_800_000_000;

fn offer(version: &str) -> UpdateInfo {
    UpdateInfo {
        version: semver::Version::parse(version).expect(version),
        ..info("https://dl.example", ASSET)
    }
}

fn file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("iris").join("update.json");
    (dir, path)
}

#[test]
fn a_recorded_check_reads_back_whole() {
    let (_dir, path) = file();
    let found = offer("9.9.9");
    record_check(&path, UpdateChannel::Beta, Some(&found), at(T)).expect("record");
    assert_eq!(
        State::load(&path),
        State {
            format: FORMAT,
            checked_at: Some(T),
            channel: Some(UpdateChannel::Beta),
            offer: Some(found.clone()),
            announced: None,
        }
    );

    record_announced(&path, &found.version).expect("announce");
    let state = State::load(&path);
    assert_eq!(state.announced.as_deref(), Some("9.9.9"));
    assert_eq!(state.offer, Some(found));
    assert_eq!(state.checked(), Some(at(T)));

    record_check(&path, UpdateChannel::Stable, None, at(T + 60)).expect("record");
    let state = State::load(&path);
    assert_eq!(
        state.offer, None,
        "a check that found nothing withdraws the offer"
    );
    assert_eq!(state.channel, Some(UpdateChannel::Stable));
    assert_eq!(state.announced.as_deref(), Some("9.9.9"));
}

#[test]
fn an_unreadable_file_reads_as_no_state_and_the_next_check_replaces_it() {
    let found = offer("9.9.9");
    let good = serde_json::to_value(State {
        format: FORMAT,
        checked_at: Some(T),
        channel: Some(UpdateChannel::Stable),
        offer: Some(found.clone()),
        announced: Some("9.9.9".into()),
    })
    .expect("serialize");
    let with = |key: &str, value: serde_json::Value| {
        let mut v = good.clone();
        v[key] = value;
        v.to_string()
    };
    let mut no_signature = good.clone();
    no_signature["offer"]
        .as_object_mut()
        .expect("offer object")
        .remove("signature_url");
    let mut no_format = good.clone();
    no_format.as_object_mut().expect("object").remove("format");
    for (case, text) in [
        ("an empty file", String::new()),
        ("text that is not JSON", "update".to_string()),
        ("half a file", good.to_string()[..40].to_string()),
        ("a JSON array", "[]".to_string()),
        ("a file with no format", no_format.to_string()),
        ("an older format", with("format", 0.into())),
        ("a newer format", with("format", (FORMAT + 1).into())),
        ("an offer with no signature URL", no_signature.to_string()),
        ("an offer with a version that is not semver", {
            let mut v = good.clone();
            v["offer"]["version"] = "9.9".into();
            v.to_string()
        }),
        (
            "a channel this iris does not have",
            with("channel", "nightly".into()),
        ),
    ] {
        let (_dir, path) = file();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, &text).expect("write");
        assert_eq!(State::load(&path), State::default(), "{case}");
        assert_eq!(
            State::load(&path).until_due(UpdateChannel::Stable, at(T)),
            Duration::ZERO,
            "{case}: a check is not due"
        );
        record_check(&path, UpdateChannel::Stable, Some(&found), at(T)).expect(case);
        assert_eq!(State::load(&path).offer.as_ref(), Some(&found), "{case}");
    }

    let (_dir, path) = file();
    assert_eq!(State::load(&path), State::default(), "a missing file");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, good.to_string()).expect("write");
    assert_eq!(
        State::load(&path).offer,
        Some(found),
        "the file the others break"
    );
}

/// Writes from threads of one process replace the file whole and one
/// at a time: no temporary file stays behind, and on Unix a reader on
/// another thread finds the previous or the next state, never part of
/// one. Windows replaces no file another thread has open, so the reader
/// runs on Unix only.
#[test]
fn a_write_replaces_the_file_whole() {
    let (dir, path) = file();
    record_check(&path, UpdateChannel::Stable, None, at(T)).expect("record");
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        let writers: Vec<_> = (0..4u64)
            .map(|n| {
                let path = &path;
                s.spawn(move || {
                    for i in 0..50u64 {
                        let found = offer(&format!("9.{n}.{i}"));
                        record_check(path, UpdateChannel::Beta, Some(&found), at(T + i))
                            .expect("record");
                        record_announced(path, &found.version).expect("announce");
                    }
                })
            })
            .collect();
        let reader = cfg!(unix).then(|| {
            s.spawn(|| {
                while !done.load(Ordering::Acquire) {
                    assert_eq!(State::load(&path).format, FORMAT, "a torn read");
                }
            })
        });
        // The reader stops before a writer's panic is raised: a scope
        // joins every thread, and a reader left running never ends.
        let wrote: Vec<_> = writers.into_iter().map(|w| w.join()).collect();
        done.store(true, Ordering::Release);
        let read = reader.map(|r| r.join());
        for result in wrote {
            result.expect("writer");
        }
        if let Some(result) = read {
            result.expect("reader");
        }
    });
    let state = State::load(&path);
    assert_eq!(state.format, FORMAT);
    assert_eq!(state.channel, Some(UpdateChannel::Beta));
    let names: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
        .expect("read dir")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(names, ["update.json"], "in {}", dir.path().display());
}

#[test]
fn a_check_is_due_a_day_after_the_last_on_the_same_channel() {
    let checked = State {
        checked_at: Some(T),
        channel: Some(UpdateChannel::Stable),
        ..State::default()
    };
    let hour = Duration::from_secs(3600);
    for (case, state, channel, now, want) in [
        (
            "no check yet",
            State::default(),
            UpdateChannel::Stable,
            at(T),
            Duration::ZERO,
        ),
        (
            "at the check",
            checked.clone(),
            UpdateChannel::Stable,
            at(T),
            INTERVAL,
        ),
        (
            "an hour on",
            checked.clone(),
            UpdateChannel::Stable,
            at(T + 3600),
            INTERVAL - hour,
        ),
        (
            "a second before the day",
            checked.clone(),
            UpdateChannel::Stable,
            at(T + INTERVAL.as_secs() - 1),
            Duration::from_secs(1),
        ),
        (
            "a day on",
            checked.clone(),
            UpdateChannel::Stable,
            at(T + INTERVAL.as_secs()),
            Duration::ZERO,
        ),
        (
            "two days on",
            checked.clone(),
            UpdateChannel::Stable,
            at(T + 2 * INTERVAL.as_secs()),
            Duration::ZERO,
        ),
        (
            "on another channel",
            checked.clone(),
            UpdateChannel::Beta,
            at(T + 3600),
            Duration::ZERO,
        ),
        (
            "a clock set back before the check",
            checked.clone(),
            UpdateChannel::Stable,
            at(T - 3600),
            Duration::ZERO,
        ),
    ] {
        assert_eq!(state.until_due(channel, now), want, "{case}");
    }
}

#[test]
fn the_offer_is_newer_than_the_running_iris_and_on_its_channel() {
    let holding = |version: &str| State {
        offer: Some(offer(version)),
        ..State::default()
    };
    let v = |text: &str| semver::Version::parse(text).expect(text);
    for (case, state, channel, running, want) in [
        (
            "a newer release on stable",
            holding("9.9.9"),
            UpdateChannel::Stable,
            v("0.1.0"),
            true,
        ),
        (
            "a newer release on beta",
            holding("9.9.9"),
            UpdateChannel::Beta,
            v("0.1.0"),
            true,
        ),
        (
            "a newer prerelease on beta",
            holding("9.9.9-beta.1"),
            UpdateChannel::Beta,
            v("0.1.0"),
            true,
        ),
        (
            "a newer prerelease on stable",
            holding("9.9.9-beta.1"),
            UpdateChannel::Stable,
            v("0.1.0"),
            false,
        ),
        (
            "the running version",
            holding("9.9.9"),
            UpdateChannel::Beta,
            v("9.9.9"),
            false,
        ),
        (
            "an older version",
            holding("9.9.8"),
            UpdateChannel::Beta,
            v("9.9.9"),
            false,
        ),
        (
            "a stable release above a running beta",
            holding("9.9.9"),
            UpdateChannel::Beta,
            v("9.9.9-rc.1"),
            true,
        ),
        (
            "no offer",
            State::default(),
            UpdateChannel::Beta,
            v("0.1.0"),
            false,
        ),
    ] {
        assert_eq!(
            state.offer_for(channel, &running),
            want.then(|| state.offer.clone().expect("an offer")),
            "{case}"
        );
    }
}
