//! The command line. `OPTS` is the single definition of every option:
//! the parser reads it and `--help` prints it. The client parses its
//! own argv strictly and forwards only the options that parsed, file
//! values made absolute. The daemon parses forwarded lines with the
//! same table and logs what it does not recognize, so a newer client
//! never wedges an older daemon.

use std::fmt::Write as _;
use std::path::PathBuf;

use crate::daemon::Command;

/// An option the client process runs itself; it never reaches the
/// daemon. Declaration order is precedence: of several on one command
/// line, only the first declared runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Local {
    Help,
    Version,
    CheckUpdate,
    Update,
}

/// The value an option takes, and what it runs.
#[derive(Clone, Copy)]
enum Takes {
    Local(Local),
    Nothing(fn() -> Command),
    /// Whole seconds.
    Secs(fn(u64) -> Command),
    /// A file, made absolute against the client's working directory:
    /// the daemon runs in another.
    File(fn(PathBuf) -> Command),
}

struct Opt {
    flag: &'static str,
    takes: Takes,
    help: &'static str,
}

/// Accepted for `--help`.
const HELP_SHORT: &str = "-h";

const OPTS: &[Opt] = &[
    Opt {
        flag: "--capture",
        takes: Takes::Nothing(|| Command::Capture),
        help: "capture a region: frozen-frame overlay, then the toast",
    },
    Opt {
        flag: "--capture-fullscreen",
        takes: Takes::Nothing(|| Command::CaptureFullscreen),
        help: "capture every display with no overlay, then the toast",
    },
    Opt {
        flag: "--capture-window",
        takes: Takes::Nothing(|| Command::CaptureWindow),
        help: "capture the focused window, then the toast",
    },
    Opt {
        flag: "--delay",
        takes: Takes::Secs(Command::Delayed),
        help: "capture every display after <secs> seconds",
    },
    Opt {
        flag: "--record-window",
        takes: Takes::Nothing(|| Command::RecordToggle),
        help: "pick a window and record it, or stop the active recording",
    },
    Opt {
        flag: "--record-region",
        takes: Takes::Nothing(|| Command::RecordRegionPick),
        help: "pick a screen region and record it",
    },
    Opt {
        flag: "--record-pause",
        takes: Takes::Nothing(|| Command::RecordPause),
        help: "pause or resume the active recording",
    },
    Opt {
        flag: "--record-mic",
        takes: Takes::Nothing(|| Command::RecordMic),
        help: "turn the microphone on or off in the active recording",
    },
    Opt {
        flag: "--library",
        takes: Takes::Nothing(|| Command::Library),
        help: "open the capture library",
    },
    Opt {
        flag: "--settings",
        takes: Takes::Nothing(|| Command::Settings),
        help: "open the settings window",
    },
    Opt {
        flag: "--home",
        takes: Takes::Nothing(|| Command::Home),
        help: "open the home window",
    },
    Opt {
        flag: "--annotate",
        takes: Takes::File(Command::Annotate),
        help: "open <file> in the editor",
    },
    Opt {
        flag: "--toast",
        takes: Takes::File(Command::Toast),
        help: "show the toast for <file>",
    },
    Opt {
        flag: "--quit",
        takes: Takes::Nothing(|| Command::Quit),
        help: "stop the daemon, saving the active recording first",
    },
    Opt {
        flag: "--version",
        takes: Takes::Local(Local::Version),
        help: "print the version",
    },
    Opt {
        flag: "--check-update",
        takes: Takes::Local(Local::CheckUpdate),
        help: "report whether a newer release exists",
    },
    Opt {
        flag: "--update",
        takes: Takes::Local(Local::Update),
        help: "download and install the latest release",
    },
    Opt {
        flag: "--help",
        takes: Takes::Local(Local::Help),
        help: "print this help",
    },
];

/// One parsed command line.
#[derive(Default)]
pub struct Parsed {
    /// The daemon commands, in argument order.
    pub cmds: Vec<Command>,
    /// The daemon-bound options with their values, file values absolute:
    /// what the client forwards.
    pub forward: Vec<String>,
    /// The client-only options, in argument order.
    pub local: Vec<Local>,
    /// One message per argument that is not an option or lacks a valid
    /// value; the other arguments still parse.
    pub errors: Vec<String>,
}

impl Parsed {
    /// The client-only option that runs, by `Local` precedence.
    pub fn first_local(&self) -> Option<Local> {
        self.local.iter().min().copied()
    }
}

pub fn parse(args: &[String]) -> Parsed {
    let mut out = Parsed::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let flag = arg.as_str();
        let Some(opt) = OPTS.iter().find(|o| {
            o.flag == flag || (flag == HELP_SHORT && matches!(o.takes, Takes::Local(Local::Help)))
        }) else {
            // Finder on older macOS passes the app its process serial
            // number.
            if !arg.starts_with("-psn_") {
                out.errors.push(if arg.starts_with('-') {
                    format!("unknown option '{arg}'")
                } else {
                    format!("unexpected argument '{arg}'")
                });
            }
            continue;
        };
        let value = match opt.takes {
            Takes::Local(local) => {
                out.local.push(local);
                continue;
            }
            Takes::Nothing(cmd) => Ok((cmd(), None)),
            Takes::Secs(cmd) => match rest.next() {
                None => Err(format!("{flag} needs a number of seconds")),
                Some(v) => v
                    .parse()
                    .map(|secs| (cmd(secs), Some(v.clone())))
                    .map_err(|_| format!("{flag} takes whole seconds, not '{v}'")),
            },
            Takes::File(cmd) => match rest.next().filter(|v| !v.is_empty()) {
                None => Err(format!("{flag} needs a file")),
                Some(v) => file(v)
                    .map(|(path, text)| (cmd(path), Some(text)))
                    .map_err(|e| format!("{flag}: {e}")),
            },
        };
        match value {
            Ok((cmd, value)) => {
                out.cmds.push(cmd);
                out.forward.push(opt.flag.to_string());
                out.forward.extend(value);
            }
            Err(e) => out.errors.push(e),
        }
    }
    out
}

/// `v` as an absolute path to an existing file, and as forwardable text.
fn file(v: &str) -> Result<(PathBuf, String), String> {
    let path = std::path::absolute(v).map_err(|e| format!("'{v}': {e}"))?;
    if !path.is_file() {
        return Err(format!("no file at {}", path.display()));
    }
    let text = path
        .to_str()
        .ok_or_else(|| format!("{} is not valid UTF-8", path.display()))?
        .to_string();
    Ok((path, text))
}

/// The `--help` text.
pub fn usage() -> String {
    let mut out = String::from(
        "usage: iris [option]...\n\n\
         With no option, iris starts the daemon, or opens the home window\n\
         of the daemon already running.\n\noptions:\n",
    );
    for opt in OPTS {
        let name = match opt.takes {
            Takes::Secs(_) => format!("{} <secs>", opt.flag),
            Takes::File(_) => format!("{} <file>", opt.flag),
            Takes::Local(Local::Help) => format!("{HELP_SHORT}, {}", opt.flag),
            _ => opt.flag.to_string(),
        };
        let _ = writeln!(out, "  {name:<24}{}", opt.help);
    }
    out
}

#[cfg(test)]
mod tests;
