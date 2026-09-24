//! External programs iris runs, resolved to an absolute path before
//! the spawn. The search covers the process PATH, then locations the
//! process PATH can miss: a macOS app started from Finder or a login
//! item inherits launchd's PATH, which has no Homebrew or MacPorts
//! prefix; a Windows daemon keeps the PATH it started with, so a tool
//! installed while it runs is not on it, and the Windows Tesseract
//! installer adds no PATH entry at all.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
mod windows;

/// A program iris spawns. The matches below are exhaustive, so a new
/// tool does not compile until it has an install command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Recording (every platform) and the NVENC probe.
    Ffmpeg,
    /// OCR for "Copy text".
    Tesseract,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Ffmpeg => "ffmpeg",
            Tool::Tesseract => "tesseract",
        }
    }

    /// The instruction that installs the tool on this platform.
    fn install(self) -> &'static str {
        let (linux, macos, windows) = match self {
            Tool::Ffmpeg => (
                "install the ffmpeg package",
                "run `brew install ffmpeg`",
                "run `winget install Gyan.FFmpeg`",
            ),
            Tool::Tesseract => (
                "install the tesseract package (tesseract-ocr on Debian and Ubuntu)",
                "run `brew install tesseract`",
                "run `winget install UB-Mannheim.TesseractOCR`",
            ),
        };
        if cfg!(windows) {
            windows
        } else if cfg!(target_os = "macos") {
            macos
        } else {
            linux
        }
    }

    /// The tool's absolute path, or None when no searched directory
    /// holds it.
    pub fn find(self) -> Option<PathBuf> {
        find_in(self.name(), &search_dirs())
    }

    /// A command for the tool at its `find` path. An unresolved tool
    /// keeps its bare name, so the spawn fails with NotFound and
    /// `spawn_error` reports the install command. On Windows the child
    /// gets no console window: the daemon is a GUI process, and a
    /// console per spawn would flash on screen.
    pub fn command(self) -> Command {
        let program = self
            .find()
            .map_or_else(|| OsString::from(self.name()), PathBuf::into_os_string);
        #[allow(unused_mut)]
        let mut cmd = Command::new(program);
        #[cfg(windows)]
        windows::hide_console(&mut cmd);
        cmd
    }

    /// The message for a tool no searched directory holds: the
    /// instruction that installs it.
    pub fn missing(self) -> String {
        format!("{} is not installed: {}", self.name(), self.install())
    }

    /// The message for a failed spawn: a missing tool gets the
    /// instruction that installs it.
    pub fn spawn_error(self, e: &std::io::Error) -> String {
        if e.kind() == std::io::ErrorKind::NotFound {
            self.missing()
        } else {
            format!("{} failed to start: {e}", self.name())
        }
    }
}

/// Directories beyond PATH, searched after it: Homebrew (Apple silicon,
/// then Intel) and MacPorts.
#[cfg(target_os = "macos")]
fn extra_dirs() -> Vec<PathBuf> {
    ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
        .map(PathBuf::from)
        .into()
}

#[cfg(windows)]
use windows::extra_dirs;

/// A Linux session's PATH already holds the package manager's prefix.
#[cfg(not(any(windows, target_os = "macos")))]
fn extra_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// PATH in order, then `extra_dirs`.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(extra_dirs());
    dirs
}

/// The first `dir/name` (plus the platform's executable suffix) that
/// is a file.
fn find_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    dirs.iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

// WHY: the class closed here is "an installed tool is reported
// missing": the macOS app's launchd PATH without Homebrew, the Windows
// Tesseract installer that adds no PATH entry, a daemon PATH older than
// the install. Every spawn resolves through `Tool::find`; these pin its
// search order and what counts as a hit. Not covered: which extra
// directories a platform lists (data, checked on each OS by use).
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn tool(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&p, b"").unwrap();
        p
    }

    #[test]
    fn the_first_directory_holding_the_tool_wins() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let dirs = [
            a.path().join("absent"),
            a.path().to_path_buf(),
            b.path().to_path_buf(),
        ];
        assert_eq!(find_in("ffmpeg", &dirs), None);
        let in_b = tool(b.path(), "ffmpeg");
        assert_eq!(find_in("ffmpeg", &dirs), Some(in_b));
        let in_a = tool(a.path(), "ffmpeg");
        assert_eq!(find_in("ffmpeg", &dirs), Some(in_a));
    }

    #[test]
    fn a_directory_named_like_the_tool_is_not_a_hit() {
        let a = tempfile::tempdir().unwrap();
        let name = format!("tesseract{}", std::env::consts::EXE_SUFFIX);
        std::fs::create_dir(a.path().join(name)).unwrap();
        assert_eq!(find_in("tesseract", &[a.path().to_path_buf()]), None);
    }

    #[test]
    fn path_is_searched_before_the_extra_directories() {
        let path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let dirs = search_dirs();
        assert_eq!(dirs[..path.len()], path[..]);
        assert_eq!(dirs[path.len()..], extra_dirs()[..]);
    }

    #[test]
    fn a_missing_tool_reports_its_install_instruction() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        for tool in [Tool::Ffmpeg, Tool::Tesseract] {
            assert_eq!(
                tool.spawn_error(&missing),
                format!("{} is not installed: {}", tool.name(), tool.install())
            );
            assert!(
                tool.install().to_lowercase().contains(tool.name()),
                "{tool:?}"
            );
            assert!(tool
                .spawn_error(&denied)
                .starts_with(&format!("{} failed to start: ", tool.name())));
        }
    }
}
