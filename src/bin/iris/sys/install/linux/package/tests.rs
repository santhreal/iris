use std::ffi::OsString;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;

use super::*;

/// Every package, with the package manager that installs its file. The
/// match has no wildcard: a new package fails to compile here until it
/// has a manager.
fn every_package() -> [(Package, &'static str); 2] {
    let manager = |p: Package| match p {
        Package::Deb => "/usr/bin/apt-get",
        Package::Rpm => "/usr/bin/dnf",
    };
    [Package::Deb, Package::Rpm].map(|p| (p, manager(p)))
}

fn argv(parts: &[&str]) -> Vec<OsString> {
    parts.iter().map(OsString::from).collect()
}

#[test]
fn root_runs_the_package_manager_and_another_user_runs_it_through_pkexec() {
    let file = Path::new("/home/u/.cache/dev.iris.app/update/iris-9.9.9-linux-x86_64.pkg");
    let path = file.to_str().unwrap();
    for (package, manager) in every_package() {
        assert_eq!(
            command(package, file, true),
            argv(&[manager, "install", "-y", path]),
            "{package:?} as root"
        );
        assert_eq!(
            command(package, file, false),
            argv(&["/usr/bin/pkexec", manager, "install", "-y", path]),
            "{package:?} as another user"
        );
    }
}

/// A file name with spaces or a leading dash stays one argument: the
/// update directory is under the user's home, whatever its name.
#[test]
fn the_package_file_is_one_argument_whatever_its_name() {
    let file = Path::new("/home/a user/-y --reinstall/iris.deb");
    let got = command(Package::Deb, file, false);
    assert_eq!(got.last().map(OsString::as_os_str), Some(file.as_os_str()));
    assert_eq!(got.len(), 5);
}

#[test]
fn dpkg_lists_the_binary_only_as_a_whole_line() {
    let cases = [
        (
            "/.\n/usr\n/usr/bin\n/usr/bin/iris\n/usr/share/doc/iris\n",
            true,
        ),
        ("/usr/bin/iris", true),
        ("", false),
        ("/usr/bin/iris2\n", false),
        ("/usr/bin/iris/\n", false),
        (" /usr/bin/iris\n", false),
        ("/usr/local/bin/iris\n", false),
    ];
    for (list, want) in cases {
        assert_eq!(lists_binary(list), want, "{list:?}");
    }
}

const DPKG_LISTS_IRIS: &str = "/.\n/usr\n/usr/bin\n/usr/bin/iris\n";

/// The deb when dpkg lists the binary, else the rpm when rpm owns it,
/// else none; rpm is asked only when dpkg does not list the binary.
#[test]
fn the_package_is_the_one_whose_database_holds_usr_bin_iris() {
    let binary = Path::new("/usr/bin/iris");
    let rpm_unasked = || -> bool { panic!("rpm asked after dpkg listed the binary") };
    assert_eq!(
        Package::detect(binary, || DPKG_LISTS_IRIS.into(), rpm_unasked),
        Some(Package::Deb)
    );
    assert_eq!(
        Package::detect(binary, String::new, || true),
        Some(Package::Rpm)
    );
    assert_eq!(Package::detect(binary, String::new, || false), None);
}

/// Another iris is from no package, whatever the databases hold: the
/// update would install /usr/bin/iris and restart the wrong binary.
#[test]
fn a_binary_outside_usr_bin_is_no_package() {
    for exe in [
        "/usr/local/bin/iris",
        "/home/u/iris",
        "/usr/bin/iris-dev",
        "iris",
    ] {
        let got = Package::detect(Path::new(exe), || DPKG_LISTS_IRIS.into(), || true);
        assert_eq!(got, None, "{exe}");
    }
}

/// The status of a process that exited with `code`.
fn exited(code: i32) -> ExitStatus {
    ExitStatus::from_raw(code << 8)
}

#[test]
fn a_failed_install_reports_what_its_status_means() {
    let dismissed = "was not authorized: the password dialog was dismissed";
    let refused = "was not authorized: no administrator password was given, or no polkit agent \
                   runs in this session";
    let cases = [
        (exited(126), true, dismissed.to_string()),
        (exited(127), true, refused.to_string()),
        (exited(100), true, "exited with status 100".to_string()),
        // Without pkexec, 126 and 127 are the package manager's own.
        (exited(126), false, "exited with status 126".to_string()),
        (exited(127), false, "exited with status 127".to_string()),
        (exited(1), false, "exited with status 1".to_string()),
        (
            ExitStatus::from_raw(libc::SIGKILL),
            true,
            format!("ended with {}", ExitStatus::from_raw(libc::SIGKILL)),
        ),
    ];
    for (status, through_pkexec, want) in cases {
        assert_eq!(
            failure(status, through_pkexec),
            want,
            "{status} {through_pkexec}"
        );
    }
}

#[test]
fn a_failed_install_names_the_command_that_installs_it_by_hand() {
    let file = Path::new("/c/update/iris-9.9.9-linux-x86_64.deb");
    assert_eq!(
        Package::Deb.by_hand(file),
        "sudo apt install /c/update/iris-9.9.9-linux-x86_64.deb"
    );
    assert_eq!(
        Package::Rpm.by_hand(Path::new("/c/iris.rpm")),
        "sudo dnf install /c/iris.rpm"
    );
}
