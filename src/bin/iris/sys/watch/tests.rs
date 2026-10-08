//! WHY: the classes closed here are "a watch that misses a change", "a
//! watch that does not block", and "a timed wait that ends before its
//! limit". The first leaves a capture that another program deleted on
//! the library grid, or a client waiting out its retry interval after
//! the daemon bound: a removal, a move out, a made entry, a change in a
//! directory past the first, a change that lands before the wait
//! starts, and a watched directory removed. The second turns a thread
//! that waits with no limit into a spin: a wait that returns again for
//! a change it already reported, and a waker whose wake ends only one
//! wait. The third ends a quiet wait early wherever the OS wait ends
//! short of its timeout: a Windows wait can end a timer tick short, and
//! a signal ends a Unix wait. Stand-ins for the OS wait cover that
//! class on every OS, along with a change or a wake ending a timed wait
//! at once. Every case asserts its wait ends, and within what bound.
//! Not covered: a change on a network filesystem made by another host,
//! which no local watch reports, and a watch the OS refuses for want of
//! inotify instances or watches.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{wait_out, DirWatch, Entries, Woke};

/// When a helper thread acts after a wait starts.
const ACT_AFTER: Duration = Duration::from_millis(30);

/// The longest a wait may run past the change that ends it.
const PROMPT: Duration = Duration::from_secs(1);

/// A limit no case reaches when its change is reported.
const LONG: Duration = Duration::from_secs(5);

/// A limit for a wait that no change should end.
const QUIET: Duration = Duration::from_millis(200);

fn file(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"x").unwrap();
    path
}

/// `act` on a thread of its own, `ACT_AFTER` from now, then what ended
/// a wait on `watch` with `limit` and how long it ran.
fn wait_while(
    watch: &DirWatch,
    limit: Option<Duration>,
    act: impl FnOnce() + Send + 'static,
) -> (Woke, Duration) {
    let start = Instant::now();
    let actor = std::thread::spawn(move || {
        std::thread::sleep(ACT_AFTER);
        act();
    });
    let woke = watch.wait(limit);
    let took = start.elapsed();
    actor.join().unwrap();
    (woke, took)
}

/// Assert a wait that `act` ends reports a change, after the act and
/// within `PROMPT` of it.
fn ends_on(watch: &DirWatch, act: impl FnOnce() + Send + 'static) {
    let (woke, took) = wait_while(watch, Some(LONG), act);
    assert_eq!(woke, Woke::Changed, "the wait ended after {took:?}");
    assert!(
        (ACT_AFTER..ACT_AFTER + PROMPT).contains(&took),
        "a wait for a change {ACT_AFTER:?} away ended after {took:?}"
    );
}

/// Assert a wait with limit `QUIET` ends at its limit.
fn stays_quiet(watch: &DirWatch) {
    let start = Instant::now();
    let woke = watch.wait(Some(QUIET));
    let took = start.elapsed();
    assert_eq!(woke, Woke::Limit, "a quiet wait ended after {took:?}");
    assert!(
        (QUIET..QUIET + PROMPT).contains(&took),
        "a {QUIET:?} wait ended after {took:?}"
    );
}

/// Assert that at most `most` waits end for changes already made, and
/// that a wait then ends at its limit. A watch reports changes that land
/// while no wait runs in batches: inotify and kqueue queue them for one
/// wait, and Windows signals once per batch the last wait left.
fn settles(watch: &DirWatch, most: usize) {
    let reports = (0..=most)
        .take_while(|_| watch.wait(Some(QUIET)) == Woke::Changed)
        .count();
    assert!(
        reports <= most,
        "more than {most} waits ended for changes made before them"
    );
    stays_quiet(watch);
}

#[test]
fn a_removed_entry_ends_a_wait() {
    let dir = tempfile::tempdir().unwrap();
    let shot = file(dir.path(), "a.png");
    let watch = DirWatch::new(&[dir.path().to_path_buf()], Entries::Removed).unwrap();
    ends_on(&watch, move || std::fs::remove_file(shot).unwrap());
}

#[test]
fn an_entry_moved_out_ends_a_wait() {
    let dir = tempfile::tempdir().unwrap();
    let away = tempfile::tempdir().unwrap();
    let shot = file(dir.path(), "a.png");
    let to = away.path().join("a.png");
    let watch = DirWatch::new(&[dir.path().to_path_buf()], Entries::Removed).unwrap();
    ends_on(&watch, move || std::fs::rename(shot, to).unwrap());
}

#[test]
fn a_made_entry_ends_a_wait_for_made_entries() {
    let dir = tempfile::tempdir().unwrap();
    let at = dir.path().to_path_buf();
    let watch = DirWatch::new(std::slice::from_ref(&at), Entries::Made).unwrap();
    ends_on(&watch, move || {
        file(&at, "iris.sock");
    });
}

/// inotify selects the kinds it reports. A kqueue vnode filter and a
/// change notification report every entry change, which a caller reads
/// as a cue to look again.
#[cfg(target_os = "linux")]
#[test]
fn a_made_entry_does_not_end_a_wait_for_removals() {
    let dir = tempfile::tempdir().unwrap();
    let watch = DirWatch::new(&[dir.path().to_path_buf()], Entries::Removed).unwrap();
    file(dir.path(), "new.png");
    stays_quiet(&watch);
}

#[test]
fn a_change_in_any_watched_directory_ends_a_wait() {
    let dirs = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let paths: Vec<PathBuf> = dirs.iter().map(|d| d.path().to_path_buf()).collect();
    let watch = DirWatch::new(&paths, Entries::Removed).unwrap();
    for dir in &dirs {
        let shot = file(dir.path(), "a.png");
        // The make may report on platforms that report every change.
        let _ = watch.wait(Some(QUIET));
        ends_on(&watch, move || std::fs::remove_file(shot).unwrap());
    }
}

#[test]
fn a_change_made_before_the_wait_ends_it_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let shot = file(dir.path(), "a.png");
    let watch = DirWatch::new(&[dir.path().to_path_buf()], Entries::Removed).unwrap();
    std::fs::remove_file(shot).unwrap();
    let start = Instant::now();
    assert_eq!(watch.wait(Some(LONG)), Woke::Changed);
    assert!(start.elapsed() < PROMPT, "took {:?}", start.elapsed());
}

#[test]
fn a_reported_change_does_not_end_the_next_wait() {
    let dir = tempfile::tempdir().unwrap();
    const SHOTS: usize = 8;
    let shots: Vec<PathBuf> = (0..SHOTS)
        .map(|i| file(dir.path(), &format!("{i}.png")))
        .collect();
    let watch = DirWatch::new(&[dir.path().to_path_buf()], Entries::Removed).unwrap();
    ends_on(&watch, move || {
        for shot in shots {
            std::fs::remove_file(shot).unwrap();
        }
    });
    // Removals that landed after the first report end at most one wait
    // each.
    settles(&watch, SHOTS - 1);
}

#[test]
fn a_missing_directory_is_skipped_and_the_rest_are_watched() {
    let dir = tempfile::tempdir().unwrap();
    let shot = file(dir.path(), "a.png");
    let gone = dir.path().join("gone");
    let watch = DirWatch::new(&[gone, dir.path().to_path_buf()], Entries::Removed).unwrap();
    ends_on(&watch, move || std::fs::remove_file(shot).unwrap());
}

#[test]
fn a_file_in_place_of_a_directory_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let not_dir = file(dir.path(), "not-a-dir");
    let watch = DirWatch::new(&[not_dir], Entries::Removed).unwrap();
    stays_quiet(&watch);
}

#[test]
fn the_watched_directory_removed_ends_a_wait() {
    let parent = tempfile::tempdir().unwrap();
    let dir = parent.path().join("shots");
    std::fs::create_dir(&dir).unwrap();
    let watch = DirWatch::new(std::slice::from_ref(&dir), Entries::Removed).unwrap();
    ends_on(&watch, move || std::fs::remove_dir(dir).unwrap());
    // A directory that is gone reports nothing more.
    settles(&watch, 1);
}

#[test]
fn a_wait_with_no_change_ends_at_its_limit() {
    let dir = tempfile::tempdir().unwrap();
    let watch = DirWatch::new(&[dir.path().to_path_buf()], Entries::Removed).unwrap();
    stays_quiet(&watch);
}

#[test]
fn a_waker_ends_a_wait_with_no_limit_and_every_later_one() {
    let dir = tempfile::tempdir().unwrap();
    let watch = DirWatch::new(&[dir.path().to_path_buf()], Entries::Removed).unwrap();
    let waker = watch.waker();
    // The wait runs on a thread of its own: a wake that never lands
    // fails the case at LONG instead of hanging the test binary.
    let (tx, rx) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let first = watch.wait(None);
        let ended = Instant::now();
        let later = [watch.wait(None), watch.wait(Some(LONG))];
        tx.send((first, ended, later, Instant::now())).unwrap();
    });
    std::thread::sleep(ACT_AFTER);
    // Read before the wake, so a wait that the wake ended ends after it
    // however late the waiter's thread started.
    let woken = Instant::now();
    waker.wake();
    let (first, ended, later, done) = rx
        .recv_timeout(LONG)
        .expect("a woken wait with no limit never ended");
    waiter.join().unwrap();
    assert_eq!(first, Woke::Woken);
    assert!(
        ended >= woken,
        "the wait ended {:?} before the wake",
        woken - ended
    );
    assert_eq!(later, [Woke::Woken; 2], "a wake ends every later wait");
    assert!(
        done - woken < PROMPT,
        "the woken waits took {:?}",
        done - woken
    );
}

#[test]
fn an_empty_watch_waits_for_its_waker() {
    let watch = DirWatch::new(&[], Entries::Removed).unwrap();
    stays_quiet(&watch);
    let waker = watch.waker();
    let (woke, took) = wait_while(&watch, Some(LONG), move || waker.wake());
    assert_eq!(woke, Woke::Woken, "ended after {took:?}");
}

/// How far short of its timeout a Windows wait can end: one timer tick
/// at the default resolution.
const TICK: Duration = Duration::from_millis(16);

/// How long a stand-in for an OS wait blocks, given what is left of its
/// limit.
type Nap = fn(Duration) -> Duration;

#[test]
fn a_timed_wait_lasts_its_limit_when_the_os_wait_ends_short() {
    let short: [(&str, Nap); 3] = [
        ("a tick short", |left| left.saturating_sub(TICK)),
        ("at half its timeout", |left| left / 2),
        ("at once", |_| Duration::ZERO),
    ];
    for (how, nap) in short {
        let mut slept = Duration::ZERO;
        let start = Instant::now();
        let woke = wait_out(Some(QUIET), |left| {
            let left = left.expect("a timed wait passes what is left of its limit");
            assert!(
                left + slept <= QUIET,
                "{how}: {left:?} passed after {slept:?} of waits"
            );
            let at = Instant::now();
            std::thread::sleep(nap(left));
            slept += at.elapsed();
            Woke::Limit
        });
        let took = start.elapsed();
        assert_eq!(woke, Woke::Limit, "{how}");
        assert!(
            (QUIET..QUIET + PROMPT).contains(&took),
            "{how}: a {QUIET:?} wait ended after {took:?}"
        );
    }
}

#[test]
fn a_change_or_a_wake_ends_a_timed_wait_at_once() {
    for end in [Woke::Changed, Woke::Woken] {
        let mut calls = 0;
        let start = Instant::now();
        let woke = wait_out(Some(LONG), |_| {
            calls += 1;
            std::thread::sleep(ACT_AFTER);
            end
        });
        let took = start.elapsed();
        assert_eq!((woke, calls), (end, 1), "after {took:?}");
        assert!(
            took < ACT_AFTER + PROMPT,
            "{end:?} ended a wait after {took:?}"
        );
    }
}

/// A wait with no limit, or with one past what the clock holds, is one
/// OS wait. A zero limit still makes one, which reports changes made
/// before it.
#[test]
fn a_wait_with_no_deadline_or_a_zero_limit_is_one_os_wait() {
    for limit in [None, Some(Duration::MAX), Some(Duration::ZERO)] {
        let mut passed = Vec::new();
        // A second call returns Woken, so a loop ends instead of hanging.
        let woke = wait_out(limit, |left| {
            passed.push(left);
            if passed.len() == 1 {
                Woke::Limit
            } else {
                Woke::Woken
            }
        });
        assert_eq!(
            (woke, passed),
            (Woke::Limit, vec![limit]),
            "limit {limit:?}"
        );
    }
}
