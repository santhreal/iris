//! Single-instance surfaces: a second open of the home, library, or
//! settings window, or of the editor on one file, focuses the window
//! already open instead of opening another.
//!
//! WHY: the class closed here is "a surface with one window per subject
//! opens a second window". Two settings windows each save their own
//! config snapshot over the other's edits, and two editors on one file
//! each save their own markup over the other's. Every option in
//! `iris --help` is classified in `OPTIONS`, so an option added without
//! a decision fails `every_option_has_a_window_decision`.
//!
//! The window test runs only with `IRIS_X11_TEST_DISPLAY` set (a private
//! Xvfb): the daemon opens real windows there. Not covered: the stacking
//! order a window manager gives the raised window (the test server has
//! none), an editor closing after Done or Discard, whose file opens a
//! new editor, Wayland, where the compositor's activation policy
//! applies, Windows, and macOS.

use std::process::Command;

// The window case uses the rest of the table's helpers, on Linux only.
#[allow(dead_code)]
#[path = "support/options.rs"]
mod options;
#[cfg(target_os = "linux")]
#[path = "surfaces/window.rs"]
mod window;
// Other test files use the rest of the helpers.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use options::OPTIONS;

#[test]
fn every_option_has_a_window_decision() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_iris"))
        .arg("--help")
        .env("IRIS_HOME", home.path())
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let listed: Vec<&str> = text
        .lines()
        .skip_while(|line| *line != "options:")
        .filter_map(|line| line.split_whitespace().find(|t| t.starts_with("--")))
        .collect();
    let classified: Vec<&str> = OPTIONS.iter().map(|(option, _)| *option).collect();
    assert_eq!(
        listed, classified,
        "classify every `iris --help` option in OPTIONS, in its order"
    );
}
