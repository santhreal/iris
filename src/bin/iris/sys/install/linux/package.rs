//! The deb and the rpm: an update installs the release's package over
//! the installed one with the distribution's package manager, as root.
//! A process that is not root runs the package manager through pkexec,
//! which asks the session's polkit agent, or on a terminal pkexec's own
//! prompt, for an administrator's password.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitStatus};

/// Where both packages install iris (packaging/linux).
pub const BINARY: &str = "/usr/bin/iris";

/// dpkg's list of the files the installed iris deb holds.
const DPKG_LIST: &str = "/var/lib/dpkg/info/iris.list";

const PKEXEC: &str = "/usr/bin/pkexec";
const RPM: &str = "/usr/bin/rpm";

/// pkexec's status when the authentication dialog was dismissed.
const DISMISSED: i32 = 126;

/// pkexec's status when the caller is not authorized, no polkit agent
/// could ask for a password, or pkexec failed.
const NOT_AUTHORIZED: i32 = 127;

/// The package an iris binary was installed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Package {
    Deb,
    Rpm,
}

impl Package {
    /// The package that installed `exe`: the deb when dpkg lists it, the
    /// rpm when rpm reports the iris package owns it, None otherwise.
    pub fn of(exe: &Path) -> Option<Self> {
        let dpkg_list = || std::fs::read_to_string(DPKG_LIST).unwrap_or_default();
        Self::detect(exe, dpkg_list, rpm_owns_binary)
    }

    /// `of`, with dpkg's file list of the iris deb and rpm's answer
    /// whether the iris rpm owns the binary read only when needed.
    fn detect(
        exe: &Path,
        dpkg_list: impl FnOnce() -> String,
        rpm_owns: impl FnOnce() -> bool,
    ) -> Option<Self> {
        if exe != Path::new(BINARY) {
            return None;
        }
        if lists_binary(&dpkg_list()) {
            return Some(Self::Deb);
        }
        rpm_owns().then_some(Self::Rpm)
    }

    /// The package manager that installs a package file, and the
    /// package that holds pkexec on the distributions that use it.
    fn tools(self) -> (&'static str, &'static str) {
        match self {
            Self::Deb => ("/usr/bin/apt-get", "pkexec"),
            Self::Rpm => ("/usr/bin/dnf", "polkit"),
        }
    }

    /// The command a person runs to install `file` by hand.
    fn by_hand(self, file: &Path) -> String {
        let tool = match self {
            Self::Deb => "apt",
            Self::Rpm => "dnf",
        };
        format!("sudo {tool} install {}", file.display())
    }

    /// Ok when this process can run the package manager as root: the
    /// package manager is installed, and pkexec is too unless this
    /// process is root. Checked before the download and before the
    /// daemon stops.
    pub fn ready(self) -> Result<(), String> {
        let (manager, polkit) = self.tools();
        if !Path::new(manager).is_file() {
            return Err(format!(
                "update: {manager} is not installed; install the release's package by hand"
            ));
        }
        if !is_root() && !Path::new(PKEXEC).is_file() {
            return Err(format!(
                "update: {PKEXEC} is not installed, which runs {manager} as root; install the \
                 {polkit} package, or install the release's package by hand"
            ));
        }
        Ok(())
    }

    /// Install the package file `file` over the installed package.
    pub fn install(self, file: &Path) -> Result<(), String> {
        let argv = command(self, file, is_root());
        let shown = argv
            .iter()
            .map(|a| a.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let status = Command::new(&argv[0])
            .args(&argv[1..])
            .status()
            .map_err(|e| format!("update: run {shown}: {e}"))?;
        if status.success() {
            return Ok(());
        }
        Err(format!(
            "update: {shown} {}; install it with: {}",
            failure(status, argv[0] == PKEXEC),
            self.by_hand(file)
        ))
    }
}

/// The argv that installs `file` as the `package`'s manager, through
/// pkexec unless `root`.
fn command(package: Package, file: &Path, root: bool) -> Vec<OsString> {
    let (manager, _) = package.tools();
    let mut argv: Vec<OsString> = Vec::with_capacity(5);
    if !root {
        argv.push(PKEXEC.into());
    }
    argv.extend([manager.into(), "install".into(), "-y".into()]);
    argv.push(file.as_os_str().to_owned());
    argv
}

/// What a failed install's `status` means.
fn failure(status: ExitStatus, through_pkexec: bool) -> String {
    let reason = match status.code() {
        Some(DISMISSED) if through_pkexec => "the password dialog was dismissed",
        Some(NOT_AUTHORIZED) if through_pkexec => {
            "no administrator password was given, or no polkit agent runs in this session"
        }
        Some(code) => return format!("exited with status {code}"),
        None => return format!("ended with {status}"),
    };
    format!("was not authorized: {reason}")
}

/// Whether dpkg's file list `list` holds the iris binary.
fn lists_binary(list: &str) -> bool {
    list.lines().any(|line| line == BINARY)
}

/// Whether rpm reports that the iris package owns the iris binary.
fn rpm_owns_binary() -> bool {
    Command::new(RPM)
        .args(["-qf", "--queryformat", "%{NAME}", BINARY])
        .output()
        .is_ok_and(|out| out.status.success() && out.stdout == b"iris")
}

fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

// WHY: the class closed here is "a deb or rpm install cannot update":
// `iris --update` refused a package install and named a package
// manager that has no repository with iris in it. These pin which file
// list names the package, the command each package's update runs as
// root and as another user, and the reason a failed install reports.
// Not covered: the package manager's own run, and pkexec's prompt.
#[cfg(test)]
mod tests;
