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

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, InputFocus};

// Other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use x11::{focus, windows_of};

/// How an `iris --help` option relates to windows with one subject.
#[derive(Clone, Copy)]
enum Kind {
    /// Opens the iris window of this title; a second open focuses it.
    /// Every iris window shares one WM_CLASS, so the title is what
    /// tells the surfaces apart. The editor's is its file's name, here
    /// `single.png`.
    Single(&'static str),
    /// Opens no window that has one subject: a capture, a recording, a
    /// toast card, or a command the client runs itself.
    Other,
}

/// Every option `iris --help` lists, in its order.
const OPTIONS: &[(&str, Kind)] = &[
    ("--capture", Kind::Other),
    ("--capture-fullscreen", Kind::Other),
    ("--capture-window", Kind::Other),
    ("--delay", Kind::Other),
    ("--record-window", Kind::Other),
    ("--record-region", Kind::Other),
    ("--record-pause", Kind::Other),
    ("--record-mic", Kind::Other),
    ("--library", Kind::Single("Library - iris")),
    ("--settings", Kind::Single("Settings - iris")),
    ("--home", Kind::Single("iris")),
    ("--annotate", Kind::Single(EDITOR)),
    ("--toast", Kind::Other),
    ("--quit", Kind::Other),
    ("--version", Kind::Other),
    ("--check-update", Kind::Other),
    ("--update", Kind::Other),
    ("--help", Kind::Other),
];

/// The title of the editor on the test's `single.png`.
const EDITOR: &str = "single.png - iris";

/// How long a window that should not exist has to map.
const SETTLE: Duration = Duration::from_millis(800);

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

#[test]
fn a_second_open_focuses_the_window_already_open() {
    let Some(display) = std::env::var_os("IRIS_X11_TEST_DISPLAY") else {
        return;
    };
    let daemon = Daemon::start(&display);
    let name = display.to_str().expect("IRIS_X11_TEST_DISPLAY is UTF-8");
    let (conn, screen) = x11rb::connect(Some(name)).unwrap();
    let root = conn.setup().roots[screen].root;
    let shot = daemon.dir.path().join("shots").join("single.png");
    image::RgbaImage::from_pixel(64, 48, image::Rgba([40, 90, 200, 255]))
        .save(&shot)
        .unwrap();
    for &(option, kind) in OPTIONS {
        let Kind::Single(title) = kind else {
            continue;
        };
        let mut args = vec![OsStr::new(option)];
        if option == "--annotate" {
            args.push(shot.as_os_str());
        }
        daemon.forward(&args);
        let first = daemon.until(
            &format!("{option} to open a window titled {title:?}"),
            || match windows_of(&conn, root, title).as_slice() {
                [window] => Some(*window),
                _ => None,
            },
        );
        // Focus leaves the window; the round trip orders the change
        // before the second open.
        conn.set_input_focus(InputFocus::POINTER_ROOT, root, x11rb::CURRENT_TIME)
            .unwrap();
        assert_eq!(focus(&conn), root);
        daemon.forward(&args);
        daemon.until(
            &format!("a second {option} to focus the open window"),
            || (focus(&conn) == first).then_some(()),
        );
        std::thread::sleep(SETTLE);
        assert_eq!(
            windows_of(&conn, root, title),
            [first],
            "a second {option} opened another window titled {title:?}"
        );
    }
    // The editor's subject is its file: another file opens another
    // editor beside the first.
    let other = daemon.dir.path().join("shots").join("other.png");
    std::fs::copy(&shot, &other).unwrap();
    daemon.forward(&[OsStr::new("--annotate"), other.as_os_str()]);
    daemon.until("--annotate on another file to open a second editor", || {
        (windows_of(&conn, root, "other.png - iris").len() == 1
            && windows_of(&conn, root, EDITOR).len() == 1)
            .then_some(())
    });
}

/// A daemon on the test display with every iris location, its socket,
/// and its captures in a fresh directory. Its session bus address leads
/// nowhere, so its tray never registers on a desktop's panel. Killed on
/// drop.
struct Daemon {
    dir: tempfile::TempDir,
    display: OsString,
    child: Child,
}

impl Daemon {
    fn start(display: &OsStr) -> Daemon {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["config", "run", "shots", "vids"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        // A session's runtime directory admits only its user, so the
        // daemon binds `run/iris.sock` in it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let private = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(dir.path().join("run"), private).unwrap();
        }
        let quoted = |p: &Path| toml::Value::from(p.to_str().unwrap()).to_string();
        std::fs::write(
            dir.path().join("config").join("config.toml"),
            format!(
                "screenshots_dir = {}\nrecordings_dir = {}\n",
                quoted(&dir.path().join("shots")),
                quoted(&dir.path().join("vids")),
            ),
        )
        .unwrap();
        let log = std::fs::File::create(dir.path().join("daemon.log")).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_iris"));
        configure(&mut cmd, dir.path(), display);
        let child = cmd
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let daemon = Daemon {
            dir,
            display: display.to_owned(),
            child,
        };
        let socket = daemon.dir.path().join("run").join("iris.sock");
        daemon.until("the daemon to bind its socket", || {
            socket.exists().then_some(())
        });
        daemon
    }

    /// `iris <args>` as a client of this daemon.
    fn forward(&self, args: &[&OsStr]) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_iris"));
        configure(&mut cmd, self.dir.path(), &self.display);
        let out = cmd.args(args).output().unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "iris {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Poll `f` until it returns a value, for at most 10 s.
    fn until<T>(&self, what: &str, mut f: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(value) = f() {
                return value;
            }
            if Instant::now() > deadline {
                let log =
                    std::fs::read_to_string(self.dir.path().join("daemon.log")).unwrap_or_default();
                panic!("timed out waiting for {what}; daemon log:\n{log}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The environment every iris process of one daemon shares.
fn configure(cmd: &mut Command, dir: &Path, display: &OsStr) {
    cmd.env("DISPLAY", display)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("IRIS_SLOWMO")
        .env("IRIS_HOME", dir)
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", dir.join("no-bus").display()),
        );
}
