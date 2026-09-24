//! WHY: the classes closed here are "a daemon no client can reach",
//! "a client that stalls the daemon or runs part of a command line",
//! "a quit that ends its wait while the daemon still runs", "two
//! daemons", and "a forward that lags the bind". The first: a listener
//! that fails to bind on one platform, as every macOS bind did while
//! the socket's mode was set with an fchmod macOS rejects, or a command
//! line that arrives split or altered. The second: a client that
//! connects and sends nothing, which held the accept thread and every
//! later command line behind it, and a command line cut short at a
//! limit, whose first part ran. The third: `--update` sent `--quit` and
//! replaced the binary 5 s later whether or not the daemon had exited,
//! and a daemon saving a recording runs longer. The fourth: processes
//! that started at once each found no daemon and became one, and on
//! Unix each bind replaced the socket file of the one before, so every
//! daemon ran on. A claim must go to one taker at a time, to one of
//! many that take it at once, and to the next once dropped. The fifth:
//! a client waiting for a starting daemon retried its connect every
//! 50 ms, so a cold `iris --home` opened its window up to 50 ms after
//! the daemon could have. Each test tightens only the limit it reaches,
//! on a name no daemon uses, wherever `cargo test` runs. Not covered:
//! the daemon's own start, which needs a display (tests/instance.rs), a
//! claim that ends with its process, the update's own 60 s wait, and a
//! lag under 10 ms.

use std::prelude::v1::test;

use super::*;
use futures::channel::mpsc::TryRecvError;

/// The next command line the listener delivers within `limit`, if any.
fn within(rx: &mut UnboundedReceiver<Vec<String>>, limit: Duration) -> Option<Vec<String>> {
    let deadline = Instant::now() + limit;
    loop {
        match rx.try_recv() {
            Ok(args) => return Some(args),
            Err(TryRecvError::Closed) => panic!("the accept thread ended"),
            Err(TryRecvError::Empty) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(TryRecvError::Empty) => return None,
        }
    }
}

/// The next command line the listener delivers, within `limit`.
fn next_within(rx: &mut UnboundedReceiver<Vec<String>>, limit: Duration) -> Vec<String> {
    within(rx, limit).unwrap_or_else(|| panic!("no command line within {limit:?}"))
}

/// Every command line the listener delivers until none arrives for
/// `quiet`.
fn drain(rx: &mut UnboundedReceiver<Vec<String>>, quiet: Duration) -> Vec<Vec<String>> {
    std::iter::from_fn(|| within(rx, quiet)).collect()
}

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// Whether the daemon closes `stream` within `limit` without a byte
/// written to it.
fn closed_within(mut stream: LocalSocketStream, limit: Duration) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(stream.read(&mut [0u8; 1]).ok());
    });
    rx.recv_timeout(limit) == Ok(Some(0))
}

#[test]
fn a_listener_receives_each_command_line_whole() {
    let (name, _dir) = imp::scratch_name();
    let mut rx = listen(name.clone(), LIMITS).expect("bind the daemon's socket");
    // Past one 4 KiB read, with a two-byte character across the first
    // read's end when the read fills.
    let long = format!("x{}", "é".repeat(3000));
    let lines: [&[&str]; 3] = [
        &["--annotate", "/a dir/ü ñ 字.png"],
        &["--library"],
        &["--toast", long.as_str()],
    ];
    for args in lines {
        let args = argv(args);
        assert!(send(name.borrow(), &args), "connect to {name:?}");
        assert_eq!(next_within(&mut rx, Duration::from_secs(5)), args);
    }
}

#[test]
fn a_silent_connection_delays_no_other_command() {
    let (name, _dir) = imp::scratch_name();
    let mut rx = listen(name.clone(), LIMITS).unwrap();
    let _silent = imp::connect(name.borrow()).unwrap();
    let args = argv(&["--library"]);
    assert!(send(name.borrow(), &args));
    assert_eq!(next_within(&mut rx, Duration::from_secs(5)), args);
}

#[test]
fn each_reader_frees_its_slot() {
    // Three times as many command lines as readers, one at a time: a
    // slot that stayed taken would close the third connection unread.
    let limits = Limits {
        readers: 2,
        ..LIMITS
    };
    let (name, _dir) = imp::scratch_name();
    let mut rx = listen(name.clone(), limits).unwrap();
    for i in 0..limits.readers * 3 {
        let args = argv(&["--toast", &i.to_string()]);
        assert!(send(name.borrow(), &args));
        assert_eq!(next_within(&mut rx, Duration::from_secs(5)), args);
    }
}

#[test]
fn past_the_reader_limit_a_connection_is_closed_unread_until_a_slot_frees() {
    let limits = Limits {
        readers: 2,
        ..LIMITS
    };
    let (name, _dir) = imp::scratch_name();
    let mut rx = listen(name.clone(), limits).unwrap();
    let mut silent: Vec<_> = (0..limits.readers)
        .map(|_| imp::connect(name.borrow()).unwrap())
        .collect();
    let extra = imp::connect(name.borrow()).unwrap();
    assert!(
        closed_within(extra, Duration::from_secs(5)),
        "a connection past the reader limit stayed open"
    );
    // A client that closes frees its reader's slot. Until the reader
    // sees the close, a new connection is still past the limit.
    drop(silent.pop());
    let args = argv(&["--library"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if send(name.borrow(), &args)
            && within(&mut rx, Duration::from_millis(100)).as_ref() == Some(&args)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no slot freed after a client closed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_command_line_over_the_size_limit_is_dropped_whole() {
    let limits = Limits {
        line: 64 << 10,
        ..LIMITS
    };
    let (name, _dir) = imp::scratch_name();
    let mut rx = listen(name.clone(), limits).unwrap();
    // The daemon may close the connection before the client's write
    // ends, so the send's own result is not the contract.
    send(name.borrow(), &argv(&["--toast", &"x".repeat(limits.line)]));
    let args = argv(&["--library"]);
    assert!(send(name.borrow(), &args));
    assert_eq!(drain(&mut rx, Duration::from_millis(500)), [args]);
}

#[test]
fn a_command_line_that_arrives_late_is_dropped_whole() {
    let limits = Limits {
        drip: Duration::from_millis(300),
        ..LIMITS
    };
    let (name, _dir) = imp::scratch_name();
    let mut rx = listen(name.clone(), limits).unwrap();
    // Its last bytes late, then its close late: either is past the
    // limit, and the part that arrived in time is not a command line.
    let late = limits.drip + Duration::from_millis(700);
    let tails: [(&[u8], &[u8]); 2] = [(&b"--lib"[..], &b"rary"[..]), (&b"--library"[..], &[])];
    for (head, tail) in tails {
        let mut client = imp::connect(name.borrow()).unwrap();
        client.write_all(head).unwrap();
        std::thread::sleep(late);
        let _ = client.write_all(tail);
        drop(client);
        let args = argv(&["--settings"]);
        assert!(send(name.borrow(), &args));
        assert_eq!(drain(&mut rx, Duration::from_millis(500)), [args]);
    }
}

/// A daemon that answers for `saving` after `--quit` arrives, as one
/// saving a recording does, and then exits: its listener closes.
fn saving_daemon(name: Name<'static>, saving: Duration) -> std::thread::JoinHandle<()> {
    let listener = imp::bind(name).unwrap();
    std::thread::spawn(move || {
        let mut quit_at: Option<Instant> = None;
        for mut conn in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = conn.read_to_string(&mut line);
            if line == "--quit" {
                quit_at.get_or_insert_with(Instant::now);
            }
            if quit_at.is_some_and(|at| at.elapsed() >= saving) {
                return;
            }
        }
    })
}

/// `quit(name, within)` on a thread of its own: whether no daemon
/// answers at its end, and how long it took. Fails when the quit has
/// not returned 5 s past `within`.
fn timed_quit(name: &Name<'static>, within: Duration) -> (bool, Duration) {
    let (tx, rx) = std::sync::mpsc::channel();
    let name = name.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        let exited = quit(name.borrow(), within);
        let _ = tx.send((exited, start.elapsed()));
    });
    let limit = within + Duration::from_secs(5);
    rx.recv_timeout(limit)
        .unwrap_or_else(|_| panic!("the quit still waited {limit:?} after --quit"))
}

#[test]
fn a_quit_waits_for_a_daemon_that_saves_before_it_exits() {
    let (name, _dir) = imp::scratch_name();
    let saving = Duration::from_millis(300);
    let daemon = saving_daemon(name.clone(), saving);
    let (exited, waited) = timed_quit(&name, Duration::from_secs(10));
    assert!(exited, "the daemon still answered {waited:?} after --quit");
    assert!(
        waited >= saving,
        "the quit ended {waited:?} after --quit, while the daemon still saved"
    );
    assert!(
        waited < Duration::from_secs(5),
        "the quit ended {waited:?} after --quit, long after the daemon exited"
    );
    daemon.join().unwrap();
}

#[test]
fn a_quit_gives_up_on_a_daemon_that_still_answers() {
    let (name, _dir) = imp::scratch_name();
    let mut rx = listen(name.clone(), LIMITS).unwrap();
    let within = Duration::from_millis(300);
    let (exited, waited) = timed_quit(&name, within);
    assert!(
        !exited,
        "a quit reported a daemon that still answers as exited"
    );
    assert!(
        waited >= within && waited < within + Duration::from_secs(2),
        "the quit gave up {waited:?} after --quit, not {within:?}"
    );
    assert_eq!(
        next_within(&mut rx, Duration::from_secs(5)),
        argv(&["--quit"])
    );
}

#[test]
fn a_held_claim_refuses_every_other_until_it_is_dropped() {
    let (name, _dir) = imp::scratch_claim_name();
    let held = imp::claim(&name)
        .unwrap()
        .expect("a free claim was refused");
    for _ in 0..3 {
        let again = imp::claim(&name).unwrap();
        assert!(
            again.is_none(),
            "a second claim was granted beside a held one"
        );
    }
    drop(held);
    let next = imp::claim(&name).unwrap();
    assert!(next.is_some(), "a dropped claim still refused the next");
}

#[test]
fn of_claims_made_at_once_one_is_granted() {
    const TAKERS: usize = 8;
    for round in 0..20 {
        let (name, _dir) = imp::scratch_claim_name();
        let start = Arc::new(std::sync::Barrier::new(TAKERS));
        let takers: Vec<_> = (0..TAKERS)
            .map(|_| {
                let (name, start) = (name.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    imp::claim(&name).unwrap()
                })
            })
            .collect();
        // Every claim stays held until all are counted.
        let claims: Vec<_> = takers.into_iter().map(|t| t.join().unwrap()).collect();
        let granted = claims.iter().filter(|c| c.is_some()).count();
        assert_eq!(
            granted, 1,
            "round {round}: {granted} of {TAKERS} claims made at once were granted"
        );
    }
}

#[test]
fn a_forward_lands_soon_after_a_starting_daemon_binds() {
    // The bind comes 30 ms after the client starts to wait: a client that
    // retried every 50 ms forwarded 20 ms after it.
    const BIND_AFTER: Duration = Duration::from_millis(30);
    const WITHIN: Duration = Duration::from_millis(10);
    let mut lags: Vec<Duration> = (0..5)
        .map(|_| {
            let (name, _dir) = imp::scratch_name();
            let daemon = {
                let name = name.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(BIND_AFTER);
                    let rx = listen(name, LIMITS).unwrap();
                    (Instant::now(), rx)
                })
            };
            assert!(forward_when_ready(&name, &argv(&["--home"])));
            let forwarded = Instant::now();
            let (bound, mut rx) = daemon.join().unwrap();
            assert_eq!(
                next_within(&mut rx, Duration::from_secs(5)),
                argv(&["--home"])
            );
            forwarded.saturating_duration_since(bound)
        })
        .collect();
    lags.sort();
    let median = lags[lags.len() / 2];
    assert!(
        median <= WITHIN,
        "a forward landed a median {median:?} after the bind: {lags:?}"
    );
}
