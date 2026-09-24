// WHY: the class closed here is "a command line does something other
// than what it says, or nothing at all". Before this module an unknown
// option or a missing value was dropped without a word: `iris --captrue`
// started a daemon, `iris --delay soon` did nothing, and `iris --annotate
// shot.png` opened shot.png in the daemon's working directory instead of
// the caller's. Every option is checked from OPTS itself, so a new option
// without an expected command here fails; every error kind and the
// absolute file value are checked; the forwarded argv parses back to the
// same commands, which is what the daemon runs. Not covered: the exit
// status and the printing, which main.rs owns.

use std::path::Path;

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

fn debug(cmds: &[Command]) -> Vec<String> {
    cmds.iter().map(|c| format!("{c:?}")).collect()
}

/// A file that exists wherever the tests run, in the platform's own
/// separators, so making it absolute leaves it unchanged.
fn manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

/// `opt` with a valid value when it takes one.
fn alone(opt: &Opt) -> Vec<String> {
    let mut args = vec![opt.flag.to_string()];
    match opt.takes {
        Takes::Secs(_) => args.push("3".into()),
        Takes::File(_) => args.push(manifest().to_str().unwrap().into()),
        Takes::Local(_) | Takes::Daemon | Takes::Nothing(_) => {}
    }
    args
}

/// The command each daemon-bound option sends. An option missing here
/// fails `every_option_sends_its_command`.
fn expected(flag: &str) -> Option<Command> {
    Some(match flag {
        "--capture" => Command::Capture,
        "--capture-fullscreen" => Command::CaptureFullscreen,
        "--capture-window" => Command::CaptureWindow,
        "--delay" => Command::Delayed(3),
        "--record-window" => Command::RecordToggle,
        "--record-region" => Command::RecordRegionPick,
        "--record-pause" => Command::RecordPause,
        "--record-mic" => Command::RecordMic,
        "--library" => Command::Library,
        "--settings" => Command::Settings,
        "--home" => Command::Home,
        "--annotate" => Command::Annotate(manifest()),
        "--toast" => Command::Toast(manifest()),
        "--quit" => Command::Quit,
        _ => return None,
    })
}

#[test]
fn every_option_sends_its_command() {
    for opt in OPTS {
        let args = alone(opt);
        let p = parse(&args);
        assert!(p.errors.is_empty(), "{args:?}: {:?}", p.errors);
        if let Takes::Local(local) = opt.takes {
            assert_eq!(p.local, [local], "{args:?}");
            assert!(p.cmds.is_empty() && p.forward.is_empty(), "{args:?}");
            continue;
        }
        if let Takes::Daemon = opt.takes {
            assert!(p.daemon, "{args:?}");
            assert!(
                p.cmds.is_empty() && p.forward.is_empty() && p.local.is_empty(),
                "{args:?}"
            );
            continue;
        }
        let want = expected(opt.flag)
            .unwrap_or_else(|| panic!("{} has no expected command in this test", opt.flag));
        assert_eq!(debug(&p.cmds), debug(&[want]), "{args:?}");
        assert_eq!(p.forward, args, "{args:?}");
        assert!(p.local.is_empty(), "{args:?}");
    }
}

#[test]
fn help_lists_every_option_with_its_value() {
    let help = usage();
    for opt in OPTS {
        let shown = alone(opt).len() == 2;
        let line = help
            .lines()
            .find(|l| l.trim_start().split([' ', ',']).any(|w| w == opt.flag))
            .unwrap_or_else(|| panic!("{} is not in --help", opt.flag));
        assert_eq!(line.contains(" <"), shown, "{line}");
        assert!(line.ends_with(opt.help), "{line}");
    }
    assert!(help.contains("-h, --help"), "{help}");
    assert_eq!(parse(&argv(&["-h"])).local, [Local::Help]);
}

/// With several client-only options on one line one runs: `--help`
/// before anything, `--version` before the update options, and the
/// check before the update itself.
#[test]
fn one_client_option_runs_by_precedence() {
    let p = parse(&argv(&["--update", "--check-update", "--version", "-h"]));
    assert_eq!(p.first_local(), Some(Local::Help));
    let p = parse(&argv(&["--update", "--check-update", "--version"]));
    assert_eq!(p.first_local(), Some(Local::Version));
    let p = parse(&argv(&["--update", "--check-update"]));
    assert_eq!(p.first_local(), Some(Local::CheckUpdate));
    let p = parse(&argv(&["--capture", "--update"]));
    assert_eq!(p.first_local(), Some(Local::Update));
    assert_eq!(parse(&argv(&["--capture"])).first_local(), None);
}

/// `--daemon` makes the process the daemon and runs nothing else: with
/// any other option the line is an error, and main runs none of it.
#[test]
fn the_daemon_option_takes_no_other_option() {
    assert!(!parse(&argv(&["--capture"])).daemon);
    let alone = parse(&argv(&["--daemon"]));
    assert!(
        alone.daemon && alone.errors.is_empty(),
        "{:?}",
        alone.errors
    );
    for other in [
        &["--capture"][..],
        &["--delay", "3"],
        &["--version"],
        &["-h"],
    ] {
        for line in [
            [&["--daemon"][..], other].concat(),
            [other, &["--daemon"]].concat(),
        ] {
            let p = parse(&argv(&line));
            assert_eq!(p.errors, ["--daemon takes no other option"], "{line:?}");
        }
    }
}

/// A daemon prints nothing: on Windows one attached to the console of
/// the terminal that started it would print there and end when that
/// terminal closes. A bare line and a lone `--daemon` run a daemon;
/// every option alone, an unknown option, and a `--daemon` line in
/// error print.
#[test]
fn only_a_daemon_line_prints_nothing() {
    for line in [&[][..], &["-psn_0_1"]] {
        assert!(!parse(&argv(line)).prints(), "{line:?}");
    }
    for opt in OPTS {
        let line = alone(opt);
        let daemon = matches!(opt.takes, Takes::Daemon);
        assert_eq!(parse(&line).prints(), !daemon, "{line:?}");
    }
    for line in [
        &["--bogus"][..],
        &["--daemon", "--bogus"],
        &["--daemon", "--capture"],
        &["--daemon", "-h"],
    ] {
        assert!(parse(&argv(line)).prints(), "{line:?}");
    }
}

/// The daemon runs what the client forwards, so the forwarded argv must
/// parse to the commands the client parsed, in order. `--daemon` takes
/// no other option and is never forwarded.
#[test]
fn forwarded_argv_parses_to_the_same_commands() {
    let mut args: Vec<String> = OPTS
        .iter()
        .filter(|opt| !matches!(opt.takes, Takes::Daemon))
        .flat_map(alone)
        .collect();
    args.insert(0, "--version".into());
    let client = parse(&args);
    assert!(client.errors.is_empty(), "{:?}", client.errors);
    let daemon = parse(&client.forward);
    assert!(daemon.errors.is_empty(), "{:?}", daemon.errors);
    assert!(daemon.local.is_empty(), "{:?}", daemon.local);
    assert_eq!(debug(&daemon.cmds), debug(&client.cmds));
    assert_eq!(daemon.forward, client.forward);
}

/// The daemon runs in its own working directory: a relative file must
/// leave the client absolute, resolved against the client's.
#[test]
fn a_relative_file_is_forwarded_absolute() {
    let cwd = std::env::current_dir().unwrap();
    assert!(
        cwd.join("Cargo.toml").is_file(),
        "tests run from the crate root"
    );
    for flag in ["--annotate", "--toast"] {
        let p = parse(&argv(&[flag, "Cargo.toml"]));
        assert!(p.errors.is_empty(), "{flag}: {:?}", p.errors);
        let abs = cwd.join("Cargo.toml");
        assert_eq!(p.forward, [flag.to_string(), abs.to_str().unwrap().into()]);
        let sent = debug(&p.cmds);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains(&format!("{abs:?}")), "{flag}: {sent:?}");
    }
}

#[test]
fn bad_arguments_are_errors_and_are_not_forwarded() {
    let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-file.png");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let cases: [(&[&str], String); 10] = [
        (&["--captrue"], "unknown option '--captrue'".into()),
        (&["-x"], "unknown option '-x'".into()),
        (&["shot.png"], "unexpected argument 'shot.png'".into()),
        (&["--delay"], "--delay needs a number of seconds".into()),
        (
            &["--delay", "soon"],
            "--delay takes whole seconds, not 'soon'".into(),
        ),
        (
            &["--delay", "-1"],
            "--delay takes whole seconds, not '-1'".into(),
        ),
        (&["--annotate"], "--annotate needs a file".into()),
        (&["--toast", ""], "--toast needs a file".into()),
        (
            &["--annotate", missing.to_str().unwrap()],
            format!("--annotate: no file at {}", missing.display()),
        ),
        (
            &["--toast", dir.to_str().unwrap()],
            format!("--toast: no file at {}", dir.display()),
        ),
    ];
    for (args, want) in cases {
        let p = parse(&argv(args));
        assert_eq!(p.errors, [want], "{args:?}");
        assert!(p.cmds.is_empty() && p.forward.is_empty(), "{args:?}");
        assert!(p.local.is_empty(), "{args:?}");
    }
}

/// An error drops only its own argument: the daemon still runs what it
/// understands from a newer client.
#[test]
fn an_error_leaves_the_other_arguments_parsed() {
    let p = parse(&argv(&["--capture", "--from-a-newer-client", "--library"]));
    assert_eq!(p.errors, ["unknown option '--from-a-newer-client'"]);
    assert_eq!(debug(&p.cmds), debug(&[Command::Capture, Command::Library]));
    assert_eq!(p.forward, ["--capture", "--library"]);
}

/// Finder on older macOS appends `-psn_<serial>`; it is neither an
/// error nor a command, so the launch behaves as a bare `iris`.
#[test]
fn a_finder_process_serial_number_is_ignored() {
    let p = parse(&argv(&["-psn_0_1234567"]));
    assert!(p.errors.is_empty() && p.cmds.is_empty() && p.forward.is_empty());
    assert!(p.local.is_empty());
}
