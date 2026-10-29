//! External programs iris runs, resolved to an absolute path before
//! the spawn. A packaged runtime beside iris, or in /usr/lib/iris or
//! /usr/lib64/iris for a native Linux package, takes precedence. The fallback search covers
//! the process PATH, then locations the process PATH can miss: a macOS
//! app started from Finder or a login item inherits launchd's PATH,
//! which has no Homebrew or MacPorts prefix; a Windows daemon keeps the
//! PATH it started with, so a tool installed while it runs is not on it.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

/// A program iris spawns. The matches below are exhaustive, so a new
/// tool does not compile until it has an install command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Recording (every platform) and the NVENC probe.
    Ffmpeg,
    /// OCR for "Copy Text". macOS and Windows recognize text with the
    /// system's own OCR instead.
    #[cfg(not(any(windows, target_os = "macos")))]
    Tesseract,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Ffmpeg => "ffmpeg",
            #[cfg(not(any(windows, target_os = "macos")))]
            Tool::Tesseract => "tesseract",
        }
    }

    /// The instruction that installs the tool on this platform.
    fn install(self) -> &'static str {
        match self {
            Tool::Ffmpeg => match std::env::consts::OS {
                "windows" => "run `winget install Gyan.FFmpeg`",
                "macos" => "run `brew install ffmpeg`",
                _ => "install the ffmpeg package (ffmpeg-free on Fedora)",
            },
            #[cfg(not(any(windows, target_os = "macos")))]
            Tool::Tesseract => "install the tesseract package (tesseract-ocr on Debian and Ubuntu)",
        }
    }

    /// The tool's absolute path, or None when no searched directory
    /// holds it.
    pub fn find(self) -> Option<PathBuf> {
        find_in(self.name(), &self.search_dirs())
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
        let mut cmd = Command::new(program);
        crate::sys::tools::hide_console(&mut cmd);
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

    /// FFmpeg uses the packaged runtime before PATH. Other tools keep
    /// PATH precedence, then search `extra_dirs`.
    fn search_dirs(self) -> Vec<PathBuf> {
        let mut dirs = match self {
            Tool::Ffmpeg => crate::sys::tools::bundled_dirs(),
            #[cfg(not(any(windows, target_os = "macos")))]
            Tool::Tesseract => Vec::new(),
        };
        dirs.extend(
            std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
        dirs.extend(crate::sys::tools::extra_dirs());
        dirs
    }
}

/// The first `dir/name` (plus the platform's executable suffix) that
/// is a file.
fn find_in(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    dirs.iter().map(|d| d.join(&file)).find(|p| p.is_file())
}

/// The error for a program `what` that ended unsuccessfully with
/// `status`: how it ended, then its trimmed `stderr` when it wrote any.
pub fn failure(what: &str, status: std::process::ExitStatus, stderr: &str) -> String {
    let ended = match status.code() {
        Some(code) => format!("{what} failed with exit code {code}"),
        None => ended_by_signal(what, status),
    };
    match stderr.trim() {
        "" => ended,
        stderr => format!("{ended}: {stderr}"),
    }
}

#[cfg(unix)]
fn ended_by_signal(what: &str, status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match status.signal() {
        Some(sig) => format!("{what} was ended by signal {sig}"),
        None => format!("{what} ended with {status}"),
    }
}

#[cfg(not(unix))]
fn ended_by_signal(what: &str, status: std::process::ExitStatus) -> String {
    format!("{what} ended with {status}")
}

// WHY: the class closed here is "an installed tool is reported
// missing": the macOS app's launchd PATH without Homebrew, a Windows
// daemon PATH older than the install. Every spawn resolves through
// `Tool::find`; these pin its search order and what counts as a hit.
// The second class is "a failed tool reads as a stutter": `ExitStatus`
// printed as "exited exit status: 255: " with an empty stderr. Every
// program failure iris reports goes through `failure`. Not covered:
// which extra directories a platform lists (data, checked on each OS
// by use).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::tools::extra_dirs;
    use std::path::Path;

    /// Every tool this platform spawns.
    #[cfg(not(any(windows, target_os = "macos")))]
    const TOOLS: [Tool; 2] = [Tool::Ffmpeg, Tool::Tesseract];
    #[cfg(any(windows, target_os = "macos"))]
    const TOOLS: [Tool; 1] = [Tool::Ffmpeg];

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
    fn bundled_ffmpeg_is_searched_before_path_and_extra_directories() {
        let bundled = crate::sys::tools::bundled_dirs();
        let path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let dirs = Tool::Ffmpeg.search_dirs();
        assert_eq!(dirs[..bundled.len()], bundled[..]);
        assert_eq!(dirs[bundled.len()..bundled.len() + path.len()], path[..]);
        assert_eq!(dirs[bundled.len() + path.len()..], extra_dirs()[..]);
        let exe_dir = crate::sys::exe::this()
            .unwrap()
            .parent()
            .map(Path::to_path_buf);
        assert!(exe_dir.is_some_and(|dir| bundled.contains(&dir)));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn unbundled_tools_keep_path_precedence() {
        let path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let dirs = Tool::Tesseract.search_dirs();
        assert_eq!(dirs[..path.len()], path[..]);
        assert_eq!(dirs[path.len()..], extra_dirs()[..]);
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn native_linux_and_appimage_prefer_their_own_runtime() {
        let native = crate::sys::tools::bundled_dirs_from(Path::new("/usr/bin/iris"));
        assert_eq!(
            native,
            ["/usr/lib/iris", "/usr/lib64/iris", "/usr/bin"].map(PathBuf::from)
        );
        let appimage = crate::sys::tools::bundled_dirs_from(Path::new(
            "/run/user/1000/.mount_iris/usr/bin/iris",
        ));
        assert_eq!(
            appimage.first().map(PathBuf::as_path),
            Some(Path::new("/run/user/1000/.mount_iris/usr/bin"))
        );
        assert!(appimage.contains(&PathBuf::from("/usr/lib/iris")));
        assert!(appimage.contains(&PathBuf::from("/usr/lib64/iris")));
    }

    #[test]
    fn a_missing_tool_reports_its_install_instruction() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        for tool in TOOLS {
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

    /// An exit status built the way this platform's wait reports it.
    #[cfg(unix)]
    fn exited(code: i32) -> std::process::ExitStatus {
        std::os::unix::process::ExitStatusExt::from_raw(code << 8)
    }
    #[cfg(windows)]
    fn exited(code: i32) -> std::process::ExitStatus {
        std::os::windows::process::ExitStatusExt::from_raw(code as u32)
    }

    #[test]
    fn a_failure_states_the_exit_code_and_stderr_once_each() {
        for (status, stderr, want) in [
            (exited(255), "", "ffmpeg failed with exit code 255"),
            (exited(1), " \n\t", "ffmpeg failed with exit code 1"),
            (
                exited(1),
                "\n[in#0] Error opening input\n",
                "ffmpeg failed with exit code 1: [in#0] Error opening input",
            ),
        ] {
            assert_eq!(failure("ffmpeg", status, stderr), want, "{stderr:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_signal_ending_states_the_signal() {
        let killed = std::os::unix::process::ExitStatusExt::from_raw(15);
        assert_eq!(
            failure("ffmpeg join", killed, ""),
            "ffmpeg join was ended by signal 15"
        );
        assert_eq!(
            failure("ffmpeg join", killed, "Immediate exit requested"),
            "ffmpeg join was ended by signal 15: Immediate exit requested"
        );
    }
}
