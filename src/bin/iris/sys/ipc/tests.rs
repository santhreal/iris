//! WHY: the classes closed here are "a daemon no client can reach" and
//! "a client that stalls the daemon or runs part of a command line".
//! The first: a listener that fails to bind on one platform, as every
//! macOS bind did while the socket's mode was set with an fchmod macOS
//! rejects, or a command line that arrives split or altered. The
//! second: a client that connects and sends nothing, which held the
//! accept thread and every later command line behind it, and a command
//! line cut short at a limit, whose first part ran. Each test tightens
//! only the limit it reaches, on a name no daemon uses, wherever
//! `cargo test` runs. Not covered: the daemon's own start, which needs a
//! display, and a second daemon racing the first for the name.

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
        assert!(Instant::now() < deadline, "no slot freed after a client closed");
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
