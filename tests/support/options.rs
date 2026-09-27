//! Every option `iris --help` lists, classified by the window it opens.
//! `tests/surfaces.rs` fails when `--help` lists an option this table
//! does not, so a case that walks the table covers a new window once it
//! is classified.

use std::path::Path;

/// How an `iris --help` option relates to windows with one subject.
#[derive(Clone, Copy)]
pub enum Kind {
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
pub const OPTIONS: &[(&str, Kind)] = &[
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
    ("--daemon", Kind::Other),
    ("--version", Kind::Other),
    ("--check-update", Kind::Other),
    ("--update", Kind::Other),
    ("--help", Kind::Other),
];

/// The title of the editor on `single.png`.
pub const EDITOR: &str = "single.png - iris";

/// Writes `single.png` into `shots`: the file an `--annotate` of
/// [`open_args`] opens.
pub fn single_png(shots: &Path) -> std::path::PathBuf {
    let shot = shots.join("single.png");
    image::RgbaImage::from_pixel(64, 48, image::Rgba([40, 90, 200, 255]))
        .save(&shot)
        .unwrap();
    shot
}

/// The arguments that open `option`'s window: `--annotate` takes the
/// file `shot`.
pub fn open_args<'a>(option: &'a str, shot: &'a Path) -> Vec<&'a std::ffi::OsStr> {
    let mut args = vec![std::ffi::OsStr::new(option)];
    if option == "--annotate" {
        args.push(shot.as_os_str());
    }
    args
}
