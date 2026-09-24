//! The `iris` binary as a command line: the options the client runs in
//! its own process print to standard output and exit with the status
//! docs/cli.md states.
//!
//! A stdout whose reader is gone ends the command with status 1 and no
//! panic message. PowerShell closes the pipe of a GUI-subsystem program
//! it does not wait for, and a Unix `head` closes its end once it has
//! read enough. Not covered: `--check-update` and `--update`, which
//! print only after a network round trip.

use std::process::{Command, Output, Stdio};

/// `iris <arg>` with every iris location in a fresh directory, stdout
/// sent to `stdout`, stderr captured.
fn run(arg: &str, stdout: Stdio) -> Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_iris"))
        .arg(arg)
        .env("IRIS_HOME", home.path())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

#[test]
fn version_prints_the_package_version() {
    let out = run("--version", Stdio::piped());
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("iris {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
}

#[test]
fn help_prints_every_option_and_exits_0() {
    let out = run("--help", Stdio::piped());
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    for option in ["--capture", "--library", "--version", "--help"] {
        assert!(text.contains(option), "--help lacks {option}:\n{text}");
    }
}

#[test]
fn a_closed_stdout_ends_the_command_with_status_1_and_no_panic() {
    for arg in ["--version", "--help"] {
        let (reader, writer) = std::io::pipe().unwrap();
        // Every write to the pipe fails: its only reader is gone.
        drop(reader);
        let out = run(arg, writer.into());
        assert_eq!(out.status.code(), Some(1), "iris {arg}");
        assert_eq!(String::from_utf8_lossy(&out.stderr), "", "iris {arg}");
    }
}
