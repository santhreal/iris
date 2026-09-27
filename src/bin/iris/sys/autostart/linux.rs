//! Linux: XDG autostart (the Desktop Application Autostart
//! Specification). An entry in `$XDG_CONFIG_HOME/autostart` overrides
//! the entry of the same name in each `$XDG_CONFIG_DIRS` autostart
//! directory, and an entry with `Hidden=true` starts nothing.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// The entry's name: the one the deb and the rpm install in
/// /etc/xdg/autostart (packaging/linux/iris-autostart.desktop).
const ENTRY: &str = "iris-autostart.desktop";

/// The entry that overrides a package's entry and starts nothing.
const HIDDEN: &str = "[Desktop Entry]\nType=Application\nName=iris\nHidden=true\n";

/// The directories autostart entries are read from.
struct Places {
    /// `$XDG_CONFIG_HOME/autostart`: this account's.
    user: PathBuf,
    /// The autostart directory of each `$XDG_CONFIG_DIRS` entry.
    system: Vec<PathBuf>,
}

impl Places {
    fn of_this_account() -> Result<Self, String> {
        let base = directories::BaseDirs::new()
            .ok_or_else(|| "start at login: this account has no home directory".to_string())?;
        let dirs = std::env::var_os("XDG_CONFIG_DIRS")
            .filter(|dirs| !dirs.is_empty())
            .unwrap_or_else(|| "/etc/xdg".into());
        Ok(Self {
            user: base.config_dir().join("autostart"),
            system: std::env::split_paths(&dirs)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join("autostart"))
                .collect(),
        })
    }

    /// The first system entry of this name: the one a user entry
    /// overrides.
    fn system_entry(&self) -> Option<PathBuf> {
        self.system
            .iter()
            .map(|dir| dir.join(ENTRY))
            .find(|entry| entry.is_file())
    }
}

/// This iris as an autostart entry starts it.
struct Iris {
    /// The `Exec` value of its entry.
    exec: String,
    /// Whether a package installed it: a package's entry runs `iris`
    /// from PATH, which is this binary. An AppImage is not one.
    packaged: bool,
}

impl Iris {
    fn this() -> Result<Self, String> {
        let (program, packaged) = match std::env::var_os("APPIMAGE") {
            Some(image) => (PathBuf::from(image), false),
            None => (
                std::env::current_exe()
                    .map_err(|e| format!("start at login: find this iris: {e}"))?,
                true,
            ),
        };
        Ok(Self {
            exec: format!("{} --daemon", exec_arg(&program)?),
            packaged,
        })
    }
}

/// Whether an entry starts this iris at login.
pub fn enabled() -> bool {
    match (Places::of_this_account(), Iris::this()) {
        (Ok(places), Ok(iris)) => enabled_in(&places, &iris),
        _ => false,
    }
}

/// Make this iris start at login, or start nothing.
pub fn set(on: bool) -> Result<(), String> {
    set_in(&Places::of_this_account()?, &Iris::this()?, on)
}

fn enabled_in(places: &Places, iris: &Iris) -> bool {
    let user = places.user.join(ENTRY);
    if let Ok(text) = std::fs::read_to_string(&user) {
        let entry = Entry::parse(&text);
        return entry.starts && entry.exec == Some(iris.exec.as_str());
    }
    iris.packaged
        && places
            .system_entry()
            .and_then(|entry| std::fs::read_to_string(entry).ok())
            .is_some_and(|text| Entry::parse(&text).starts)
}

fn set_in(places: &Places, iris: &Iris, on: bool) -> Result<(), String> {
    let user = places.user.join(ENTRY);
    if on {
        return write(&user, &entry(&iris.exec));
    }
    if places.system_entry().is_some() {
        return write(&user, HIDDEN);
    }
    match std::fs::remove_file(&user) {
        Err(e) if e.kind() != ErrorKind::NotFound => {
            Err(format!("start at login: delete {}: {e}", user.display()))
        }
        _ => Ok(()),
    }
}

/// The entry that runs `exec` at login. The delay and the KDE ordering
/// are the package entry's: the tray's host starts with the panel.
fn entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=iris\nComment=Start the iris daemon at login\n\
         Exec={exec}\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n\
         X-GNOME-Autostart-Delay=2\nX-KDE-autostart-after=panel\n"
    )
}

/// Write `text` to `path` through a sibling a desktop does not read
/// (no .desktop suffix) and one rename, so a session that starts
/// meanwhile reads the old entry or the new one.
fn write(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let staged = dir.join(format!(".{ENTRY}.new"));
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&staged, text))
        .and_then(|()| std::fs::rename(&staged, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            format!("start at login: write {}: {e}", path.display())
        })
}

/// What an autostart entry does, from its `[Desktop Entry]` group.
struct Entry<'a> {
    /// Neither `Hidden=true` nor `X-GNOME-Autostart-enabled=false`.
    starts: bool,
    exec: Option<&'a str>,
}

impl<'a> Entry<'a> {
    fn parse(text: &'a str) -> Self {
        let mut entry = Entry {
            starts: true,
            exec: None,
        };
        let mut group = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                group = line == "[Desktop Entry]";
                continue;
            }
            let Some((key, value)) = line.split_once('=').filter(|_| group) else {
                continue;
            };
            match (key.trim_end(), value.trim_start()) {
                ("Hidden", "true") | ("X-GNOME-Autostart-enabled", "false") => entry.starts = false,
                ("Exec", exec) => entry.exec = Some(exec),
                _ => {}
            }
        }
        entry
    }
}

/// `program` as one quoted `Exec` argument (Desktop Entry
/// Specification, "The Exec key"). In a quoted argument `"`, `` ` ``,
/// and `$` take a backslash and a backslash takes another; the string
/// escape rule then doubles every backslash. `%` doubles.
fn exec_arg(program: &Path) -> Result<String, String> {
    let text = program.to_str().ok_or_else(|| {
        format!(
            "start at login: {} is not UTF-8, which a desktop entry holds",
            program.display()
        )
    })?;
    let mut arg = String::with_capacity(text.len() + 2);
    arg.push('"');
    for c in text.chars() {
        match c {
            '"' | '`' | '$' => {
                arg.push_str(r"\\");
                arg.push(c);
            }
            '\\' => arg.push_str(r"\\\\"),
            '%' => arg.push_str("%%"),
            c if c.is_control() => {
                return Err(format!(
                    "start at login: {} holds a control character, which an Exec line cannot",
                    program.display()
                ))
            }
            c => arg.push(c),
        }
    }
    arg.push('"');
    Ok(arg)
}

// WHY: the classes closed here are "Start at login reads on for an
// entry that starts nothing or starts another iris", "off leaves the
// package's /etc/xdg entry running iris", and "a path with a space or
// a shell character starts nothing". Not covered: whether a given
// desktop honors the entry; that is the specification's contract.
#[cfg(test)]
mod tests;
