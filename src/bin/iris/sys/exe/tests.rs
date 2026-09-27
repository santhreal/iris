use super::*;

#[test]
fn a_replaced_binary_resolves_to_the_file_installed_in_its_place() {
    let dir = tempfile::tempdir().expect("tempdir");
    let d = dir.path();
    std::fs::write(d.join("iris"), b"new").expect("write");
    std::fs::create_dir(d.join("folder")).expect("mkdir");
    let at = |name: &str| d.join(name);
    for (case, exe, want) in [
        ("replaced", at("iris (deleted)"), at("iris")),
        ("removed", at("gone (deleted)"), at("gone (deleted)")),
        ("running as installed", at("iris"), at("iris")),
        (
            "a directory in its place",
            at("folder (deleted)"),
            at("folder (deleted)"),
        ),
        (
            "the suffix inside the path",
            at("iris (deleted)/x"),
            at("iris (deleted)/x"),
        ),
        (
            "the suffix without its space",
            at("iris(deleted)"),
            at("iris(deleted)"),
        ),
    ] {
        assert_eq!(installed(exe), want, "{case}");
    }
}

/// The kernel's own path for a binary replaced as dpkg replaces one:
/// the new file renamed over the running one.
#[cfg(target_os = "linux")]
#[test]
fn the_kernel_s_path_for_a_replaced_binary_resolves_to_the_new_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The kernel reports the path with every link resolved.
    let bin = dir
        .path()
        .canonicalize()
        .expect("canonicalize")
        .join("sleep");
    std::fs::copy("/bin/sleep", &bin).expect("copy sleep");
    // A process another test forks meanwhile can hold the copy open for
    // writing until it execs, and exec fails with ETXTBSY until then.
    let spawn = || std::process::Command::new(&bin).arg("30").spawn();
    let mut child = spawn();
    for _ in 0..50 {
        match &child {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
                child = spawn();
            }
            _ => break,
        }
    }
    let mut child = child.expect("run the copy");
    // The spawn returns once the child releases this process's memory
    // in execve, and Linux does that before the child's exe link names
    // the new binary: until then the link names this test binary.
    let link = format!("/proc/{}/exe", child.id());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut execed = false;
    while !execed && std::time::Instant::now() < deadline {
        execed = std::fs::read_link(&link).is_ok_and(|exe| exe == bin);
        if !execed {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    let exe = if execed {
        let staged = dir.path().join("sleep.new");
        std::fs::copy("/bin/sleep", &staged).expect("copy sleep");
        std::fs::rename(&staged, &bin).expect("rename over the running copy");
        std::fs::read_link(&link)
    } else {
        Err(std::io::Error::other(
            "the child's exe link never named the copy within 5 s",
        ))
    };
    let _ = child.kill();
    let _ = child.wait();
    let exe = exe.expect("read the child's exe link");

    let mut replaced = bin.clone().into_os_string();
    replaced.push(DELETED);
    assert_eq!(exe, PathBuf::from(replaced));
    assert_eq!(installed(exe), bin);
}
