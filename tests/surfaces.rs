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
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, InputFocus, MapState};

/// How an `iris --help` option relates to windows with one subject.
#[derive(Clone, Copy)]
enum Kind {
    /// Opens the window of this WM_CLASS; a second open focuses it.
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
    ("--library", Kind::Single("dev.iris.library")),
    ("--settings", Kind::Single("dev.iris.settings")),
    ("--home", Kind::Single("dev.iris.home")),
    ("--annotate", Kind::Single("dev.iris.editor")),
    ("--toast", Kind::Other),
    ("--quit", Kind::Other),
    ("--version", Kind::Other),
    ("--check-update", Kind::Other),
    ("--update", Kind::Other),
    ("--help", Kind::Other),
];

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
        let Kind::Single(class) = kind else {
            continue;
        };
        let mut args = vec![OsStr::new(option)];
        if option == "--annotate" {
            args.push(shot.as_os_str());
        }
        daemon.forward(&args);
        let first = daemon.until(
            &format!("{option} to open a {class} window"),
            || match windows_of(&conn, root, class).as_slice() {
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
            windows_of(&conn, root, class),
            [first],
            "a second {option} opened another {class} window"
        );
    }
    // The editor's subject is its file: another file opens another
    // editor.
    let other = daemon.dir.path().join("shots").join("other.png");
    std::fs::copy(&shot, &other).unwrap();
    let editors = windows_of(&conn, root, "dev.iris.editor");
    daemon.forward(&[OsStr::new("--annotate"), other.as_os_str()]);
    daemon.until("--annotate on another file to open a second editor", || {
        (windows_of(&conn, root, "dev.iris.editor").len() == editors.len() + 1).then_some(())
    });
}

/// The mapped top-level windows whose WM_CLASS includes `class`.
fn windows_of(conn: &impl Connection, root: u32, class: &str) -> Vec<u32> {
    let children = conn.query_tree(root).unwrap().reply().unwrap().children;
    children
        .into_iter()
        .filter(|&window| {
            // A window destroyed since the tree was read answers with an
            // error, and counts as gone.
            let mapped = conn
                .get_window_attributes(window)
                .unwrap()
                .reply()
                .is_ok_and(|a| a.map_state == MapState::VIEWABLE);
            mapped
                && conn
                    .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 64)
                    .unwrap()
                    .reply()
                    .is_ok_and(|p| p.value.split(|&b| b == 0).any(|s| s == class.as_bytes()))
        })
        .collect()
}

fn focus(conn: &impl Connection) -> u32 {
    conn.get_input_focus().unwrap().reply().unwrap().focus
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
