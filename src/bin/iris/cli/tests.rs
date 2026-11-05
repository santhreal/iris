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
        Takes::Local(_) | Takes::Background | Takes::FromMenu | Takes::Nothing(_) => {}
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
        if let Takes::Background = opt.takes {
            assert!(p.background, "{args:?}");
            assert!(
                p.cmds.is_empty() && p.forward.is_empty() && p.local.is_empty(),
                "{args:?}"
            );
            continue;
        }
        if let Takes::FromMenu = opt.takes {
            assert!(p.from_menu, "{args:?}");
            assert!(p.cmds.is_empty() && p.local.is_empty(), "{args:?}");
            assert_eq!(p.forward, args, "{args:?}");
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

/// `--background` starts iris with no window and runs nothing else: with
/// any other option the line is an error, and main runs none of it.
#[test]
fn the_background_option_takes_no_other_option() {
    assert!(!parse(&argv(&["--capture"])).background);
    let alone = parse(&argv(&["--background"]));
    assert!(
        alone.background && alone.errors.is_empty(),
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
            [&["--background"][..], other].concat(),
            [other, &["--background"]].concat(),
        ] {
            let p = parse(&argv(&line));
            assert_eq!(p.errors, ["--background takes no other option"], "{line:?}");
        }
    }
}

/// WHY: a bare `iris` that found none running started iris and opened
/// nothing, so a first launch of iris.app or of `iris` in a terminal
/// showed no window. A bare line opens the library whether it forwards
/// or starts iris; `--background`, the login item's line, opens
/// nothing; a flagged line runs its own commands and no library.
#[test]
fn a_start_opens_the_library_only_for_a_bare_line() {
    let names = |line: &[&str]| -> Vec<String> {
        parse(&argv(line))
            .startup()
            .iter()
            .map(|c| format!("{c:?}"))
            .collect()
    };
    assert_eq!(names(&[]), ["Library"]);
    assert_eq!(names(&["-psn_0_12345"]), ["Library"]);
    assert!(names(&["--background"]).is_empty());
    assert_eq!(names(&["--capture"]), ["Capture"]);
    assert_eq!(names(&["--settings"]), ["Settings"]);
    assert_eq!(names(&["--library", "--settings"]), ["Library", "Settings"]);
}

/// WHY: an update restarted the daemon as a bare `iris`, and a bare
/// line opens the library, so every update opened the library. Each line
/// that starts iris in the background is parsed here: `BACKGROUND_LINE`,
/// which a client and an update spawn, the Windows installer's `/RUN`
/// start and Run value, and the Linux and macOS login entries, read
/// from the packaging files. None prints or opens a window. Not
/// covered: a spawn that passes arguments of its own instead of
/// `BACKGROUND_LINE`.
#[test]
fn every_background_start_opens_no_window() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |p: &str| std::fs::read_to_string(root.join(p)).expect(p);
    let words = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
    let mut lines = vec![("BACKGROUND_LINE".to_string(), argv(BACKGROUND_LINE))];
    for l in read("packaging/windows/iris.nsi").lines().map(str::trim) {
        let background =
            l.starts_with("Exec '") || l.starts_with("WriteRegStr HKCU \"${RUN_KEY}\" \"iris\"");
        if let Some(cmd) = l.split('\'').nth(1).filter(|_| background) {
            let args = cmd.strip_prefix("\"$INSTDIR\\iris.exe\"").expect(l);
            lines.push((l.to_string(), words(args)));
        }
    }
    assert_eq!(
        lines.len(),
        3,
        "the installer's /RUN start and Run value: {lines:?}"
    );
    let desktop = read("packaging/linux/iris-autostart.desktop");
    let exec = desktop
        .lines()
        .find_map(|l| l.strip_prefix("Exec="))
        .expect("Exec");
    let args = exec.strip_prefix("iris").expect(exec);
    lines.push((exec.to_string(), words(args)));
    let plist = read("packaging/macos/dev.iris.app.plist");
    let array = plist
        .split("<key>ProgramArguments</key>")
        .nth(1)
        .and_then(|rest| rest.split("</array>").next())
        .expect("ProgramArguments");
    let args = array
        .split("<string>")
        .skip(2)
        .map(|s| s.split("</string>").next().unwrap_or_default().to_string())
        .collect();
    lines.push(("ProgramArguments".to_string(), args));
    for (source, args) in lines {
        let p = parse(&args);
        assert!(!p.prints(), "{source}: {args:?} prints");
        assert!(p.startup().is_empty(), "{source}: {args:?} opens a window");
    }
}

/// WHY: the launcher entries ran `iris --home`, a flagged line: with no
/// iris running it started a detached second process to forward the
/// line to, and on Windows it attached a console. A launch is the app
/// itself: each launcher entry, read from the packaging files (the Linux
/// application entry, the Windows Start menu and desktop shortcuts, and
/// the installer's Finish page launch), is a bare line that prints
/// nothing, does not start iris windowless, and opens the library. iris.app
/// passes no arguments. Not covered: the Desktop Actions, which run one
/// command each.
#[test]
fn every_launcher_entry_starts_the_app_itself() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |p: &str| std::fs::read_to_string(root.join(p)).expect(p);
    let words = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
    let desktop = read("packaging/linux/dev.iris.app.desktop");
    let main_group = desktop.split("\n[").next().expect("[Desktop Entry]");
    let exec = main_group
        .lines()
        .find_map(|l| l.strip_prefix("Exec="))
        .expect("Exec");
    let mut lines = vec![(
        exec.to_string(),
        words(exec.strip_prefix("iris").expect(exec)),
    )];
    let nsi = read("packaging/windows/iris.nsi");
    for l in nsi.lines().map(str::trim) {
        if l.starts_with("CreateShortcut ") {
            let params = l.split('"').nth(5).expect(l);
            lines.push((l.to_string(), words(params)));
        }
        if let Some(params) = l.strip_prefix("!define MUI_FINISHPAGE_RUN_PARAMETERS ") {
            lines.push((l.to_string(), words(params.trim_matches('"'))));
        }
    }
    assert_eq!(
        lines.len(),
        3,
        "the application entry and the two shortcuts: {lines:?}"
    );
    for (source, args) in lines {
        let p = parse(&args);
        assert!(!p.prints(), "{source}: {args:?} prints");
        assert!(!p.background, "{source}: {args:?} opens no window");
        let opens: Vec<String> = p.startup().iter().map(|c| format!("{c:?}")).collect();
        assert_eq!(opens, ["Library"], "{source}: {args:?}");
    }
}

/// WHY: the classes closed here are "a launcher's action names a command
/// differently from the tray" and "a launcher's action freezes its menu
/// into the screen it reads". The desktop entry's actions, which a dock
/// or launcher shows in the iris icon's menu, read Capture Region,
/// Record Screen, and Open Library where the tray and the library read
/// New Screenshot, Record Window, and Library, and Record Screen named a
/// desktop recording for a line that picks a window; and New Screenshot
/// froze the dock's menu with its row lit. Each action is read from the
/// packaging file: it runs one command, its name is the tray row's label
/// for that command, so an action with no tray row fails here, and it
/// passes `--from-menu`. Not covered: translated names.
#[cfg(target_os = "linux")]
#[test]
fn every_desktop_action_reads_as_its_tray_row() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/linux/dev.iris.app.desktop");
    let desktop = std::fs::read_to_string(&path).expect("the desktop entry");
    let listed: Vec<&str> = desktop
        .lines()
        .find_map(|l| l.strip_prefix("Actions="))
        .expect("Actions=")
        .split(';')
        .filter(|id| !id.is_empty())
        .collect();
    let groups: Vec<&str> = desktop.split("\n[Desktop Action ").skip(1).collect();
    assert_eq!(
        groups.len(),
        listed.len(),
        "a group per listed action: {listed:?}"
    );
    for group in groups {
        let (id, body) = group.split_once("]\n").expect(group);
        assert!(listed.contains(&id), "{id} is not listed in Actions=");
        let field = |key: &str| {
            body.lines()
                .find_map(|l| l.strip_prefix(key))
                .unwrap_or_else(|| panic!("{id} has no {key}"))
        };
        let exec = field("Exec=");
        let args: Vec<String> = exec
            .strip_prefix("iris")
            .expect(exec)
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let p = parse(&args);
        assert!(
            p.errors.is_empty() && p.local.is_empty() && !p.background,
            "{id}: {exec}"
        );
        assert!(p.from_menu, "{id}: {exec} lacks --from-menu");
        let cmds = p.startup();
        assert_eq!(cmds.len(), 1, "{id}: {exec} runs one command");
        assert_eq!(
            Some(field("Name=")),
            crate::sys::tray::label_of(&cmds[0]),
            "{id}: {exec}"
        );
    }
}

/// A running iris prints nothing: on Windows one attached to the console
/// of the terminal that started it would print there and end when that
/// terminal closes. A bare line, a lone `--background`, and a lone
/// `--from-menu` start iris; every other option alone, an unknown
/// option, and a `--background` line in error print.
#[test]
fn only_a_start_line_prints_nothing() {
    for line in [&[][..], &["-psn_0_1"]] {
        assert!(!parse(&argv(line)).prints(), "{line:?}");
    }
    for opt in OPTS {
        let line = alone(opt);
        let starts = matches!(opt.takes, Takes::Background | Takes::FromMenu);
        assert_eq!(parse(&line).prints(), !starts, "{line:?}");
    }
    for line in [
        &["--bogus"][..],
        &["--background", "--bogus"],
        &["--background", "--capture"],
        &["--background", "-h"],
    ] {
        assert!(parse(&argv(line)).prints(), "{line:?}");
    }
}

/// The daemon runs what the client forwards, so the forwarded argv must
/// parse to the commands the client parsed, in order. `--background` takes
/// no other option and is never forwarded.
#[test]
fn forwarded_argv_parses_to_the_same_commands() {
    let mut args: Vec<String> = OPTS
        .iter()
        .filter(|opt| !matches!(opt.takes, Takes::Background))
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
    assert!(client.from_menu && daemon.from_menu);
}

// WHY: the class closed here is "a line picked from a menu freezes the
// menu into the screen it reads". Every daemon-bound option is parsed
// alone and with `--from-menu`: with it, an option that starts reading
// the screen waits `MENU_FADE` and no other waits; without it none
// waits, so a hotkey bound to `iris --capture` starts at once. An option
// that reads the screen and is missing from `READS` fails here. Not
// covered: how long a given menu's fade runs.
#[test]
fn a_menu_line_waits_for_the_menu_only_before_reading_the_screen() {
    use crate::daemon::{line_wait, MENU_FADE};
    use std::time::Duration;
    const READS: &[&str] = &[
        "--capture",
        "--capture-fullscreen",
        "--capture-window",
        "--record-window",
        "--record-region",
    ];
    for opt in OPTS {
        if !matches!(
            opt.takes,
            Takes::Nothing(_) | Takes::Secs(_) | Takes::File(_)
        ) {
            continue;
        }
        let line = alone(opt);
        let mut menu = line.clone();
        menu.push("--from-menu".into());
        let p = parse(&menu);
        assert!(p.from_menu && p.errors.is_empty(), "{menu:?}");
        let want = if READS.contains(&opt.flag) {
            MENU_FADE
        } else {
            Duration::ZERO
        };
        assert_eq!(line_wait(&p.cmds, p.from_menu), want, "{menu:?}");
        assert_eq!(
            line_wait(&parse(&line).cmds, false),
            Duration::ZERO,
            "{line:?}"
        );
    }
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
