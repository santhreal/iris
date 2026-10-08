//! The packaging scripts (packaging/lib.sh and the build scripts that
//! source it) produce the same packages on every build host.
//!
//! WHY: each test closes one class of host-dependent package.
//! - Version forms: the deb and rpm Version is the tilde form of the
//!   cargo version, so a prerelease sorts before its release. A `-`
//!   after the prerelease separator, or one in the build metadata,
//!   became a `-` or `~` that a deb reads as a Debian revision and an
//!   rpm Version cannot hold; it now fails the build.
//! - Architecture: the asset names and the package Architecture come
//!   from the ELF or PE header of the executable. An `--arch` that is
//!   neither x86_64 nor aarch64 failed with a mismatch error that named
//!   the executable's architecture instead of the bad value. Every
//!   script that packages an executable is found at run time and must
//!   enforce `--arch`; a new one that does not turns the suite red.
//! - File modes: the deb, the portable zip and iris.app took the modes
//!   of their directories and of every file a script wrote from the
//!   caller's umask. Under 002 the deb shipped group-writable
//!   directories and icons; under 077 or 000 dpkg-deb rejected the
//!   control directory. Each package is built under 077, 000 and 002.
//! - Icons: the hicolor set held nine sizes when the build host had
//!   python3 with Pillow and three otherwise.
//! - AppImage tools: appimagetool and the type2-runtime came from the
//!   moving `continuous` releases unchecked; both are now tagged
//!   releases checked against recorded hashes.
//!
//! Not covered: building the rpm and the AppImage (rpmbuild and the real
//! tool downloads are not on the test runners; the package workflow
//! builds and install-checks both), the DMG (hdiutil or genisoimage), and
//! the NSIS installer (makensis).

#![cfg(target_os = "linux")]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const REPO: &str = env!("CARGO_MANIFEST_DIR");

/// The umasks every package is built under: none of them may change a
/// packaged mode.
const UMASKS: [&str; 3] = ["077", "000", "002"];

/// The hicolor icon sizes every Linux package installs.
const ICON_SIZES: [u32; 9] = [16, 24, 32, 48, 64, 128, 256, 512, 1024];

/// Fails the test unless every tool in `tools` is on PATH.
fn require(tools: &[&str]) {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for tool in tools {
        assert!(
            std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()),
            "tests/packaging.rs needs `{tool}` on PATH"
        );
    }
}

/// Runs `bash -c script` in the repository with `args` as $1.., under
/// `umask`.
fn bash<S: AsRef<OsStr>>(umask: &str, script: &str, args: &[S]) -> Output {
    Command::new("bash")
        .arg("-c")
        .arg(format!("umask {umask} && {script}"))
        .arg("bash")
        .args(args)
        .current_dir(REPO)
        .output()
        .expect("run bash")
}

/// Runs the packaging/lib.sh function `func` with `args`: its stdout on
/// success, its stderr on failure. Output on the other stream fails the
/// test.
fn lib<S: AsRef<OsStr>>(func: &str, args: &[S]) -> Result<String, String> {
    let out = bash("022", &format!(". packaging/lib.sh && {func} \"$@\""), args);
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    let stderr = String::from_utf8(out.stderr).expect("utf-8 stderr");
    if out.status.success() {
        assert!(
            stderr.is_empty(),
            "{func} {:?}: stderr {stderr:?}",
            out.status
        );
        Ok(stdout.trim_end_matches('\n').to_string())
    } else {
        assert!(
            stdout.is_empty(),
            "{func} {:?}: stdout {stdout:?}",
            out.status
        );
        Err(stderr.trim_end_matches('\n').to_string())
    }
}

/// Runs the packaging script `script` (relative to the repository) with
/// `args` under `umask`.
fn script<S: AsRef<OsStr>>(umask: &str, script: &str, args: &[S]) -> Output {
    bash(umask, &format!("exec bash {script} \"$@\""), args)
}

fn assert_success(what: &str, out: &Output) {
    assert!(
        out.status.success(),
        "{what}: {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The first 64 bytes of an ELF64 little-endian executable for
/// `machine` (e_machine).
fn elf(machine: u16) -> Vec<u8> {
    let mut b = vec![0u8; 64];
    b[..4].copy_from_slice(b"\x7fELF");
    b[4] = 2; // ELFCLASS64
    b[5] = 1; // ELFDATA2LSB
    b[6] = 1; // EV_CURRENT
    b[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    b[18..20].copy_from_slice(&machine.to_le_bytes());
    b
}

/// The DOS header and PE signature of a PE executable for `machine`
/// (IMAGE_FILE_HEADER.Machine).
fn pe(machine: u16) -> Vec<u8> {
    let mut b = vec![0u8; 0x60];
    b[..2].copy_from_slice(b"MZ");
    b[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes()); // e_lfanew
    b[0x40..0x44].copy_from_slice(b"PE\0\0");
    b[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
    b
}

const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;
const PE_AMD64: u16 = 0x8664;
const PE_ARM64: u16 = 0xaa64;

/// Writes `bytes` to `dir/name`, mode 0755, and returns its path.
fn fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

#[test]
fn version_forms() {
    require(&["bash", "sed", "dpkg"]);
    assert_eq!(
        lib("iris_cargo_version", &[REPO]).as_deref(),
        Ok(env!("CARGO_PKG_VERSION")),
        "iris_cargo_version reads the version cargo builds"
    );

    let tilde = [
        ("0.1.0", "0.1.0"),
        ("0.2.0-beta.1", "0.2.0~beta.1"),
        ("0.2.0-rc.1+build.5", "0.2.0~rc.1+build.5"),
        ("0.2.0+build.5", "0.2.0+build.5"),
        ("0.1.0.1", "0.1.0.1"),
    ];
    for (version, want) in tilde {
        assert_eq!(
            lib("iris_tilde_version", &[version]).as_deref(),
            Ok(want),
            "{version}"
        );
    }
    for version in [
        "1.0.0-rc-1",
        "1.0.0-rc.1-2",
        "1.0.0+build-5",
        "1.0.0-rc.1+b-5",
    ] {
        assert_eq!(
            lib("iris_tilde_version", &[version]),
            Err(format!(
                "Error: version '{version}' holds a '-' after its prerelease separator; deb and rpm versions cannot hold it"
            )),
            "{version}"
        );
    }

    let core = [
        ("0.1.0", "0.1.0"),
        ("0.2.0-beta.1", "0.2.0"),
        ("0.2.0+build.5", "0.2.0"),
        ("10.20.30-rc.1+b", "10.20.30"),
    ];
    for (version, want) in core {
        assert_eq!(
            lib("iris_core_version", &[version]).as_deref(),
            Ok(want),
            "{version}"
        );
    }
    for version in ["0.2", "v0.2.0", "0.2.0.1", "", "0.2.x", "0.2.0beta"] {
        assert_eq!(
            lib("iris_core_version", &[version]),
            Err(format!(
                "Error: version '{version}' is not major.minor.patch[-prerelease]"
            )),
            "{version:?}"
        );
    }

    // Each pair is in semver order; dpkg must order their tilde forms
    // the same way, so a package upgrade follows the release order.
    let ordered = [
        ("0.1.9", "0.2.0-alpha.1"),
        ("0.2.0-alpha.1", "0.2.0-beta.1"),
        ("0.2.0-beta.1", "0.2.0-beta.2"),
        ("0.2.0-beta.2", "0.2.0-rc.1"),
        ("0.2.0-rc.1", "0.2.0"),
        ("0.2.0", "0.2.1"),
    ];
    for (older, newer) in ordered {
        let a = lib("iris_tilde_version", &[older]).expect("tilde");
        let b = lib("iris_tilde_version", &[newer]).expect("tilde");
        let status = Command::new("dpkg")
            .args(["--compare-versions", &a, "lt", &b])
            .status()
            .expect("run dpkg");
        assert!(status.success(), "dpkg orders {a} after {b}");
    }
}

#[test]
fn binary_arch_reads_elf_and_pe_headers() {
    require(&["bash", "od", "tr"]);
    let dir = tempfile::tempdir().expect("tempdir");
    let with = |mut b: Vec<u8>, at: usize, bytes: &[u8]| {
        b[at..at + bytes.len()].copy_from_slice(bytes);
        b
    };
    let cases: [(&str, Vec<u8>, Option<&str>); 18] = [
        ("elf-x86_64", elf(EM_X86_64), Some("x86_64")),
        ("elf-aarch64", elf(EM_AARCH64), Some("aarch64")),
        ("pe-amd64", pe(PE_AMD64), Some("x86_64")),
        ("pe-arm64", pe(PE_ARM64), Some("aarch64")),
        ("elf-i386", elf(3), None),
        ("elf-arm", elf(40), None),
        ("elf-riscv", elf(243), None),
        // x32: a 32-bit ELF for EM_X86_64.
        ("elf32-x86_64", with(elf(EM_X86_64), 4, &[1]), None),
        // aarch64_be: e_machine in big-endian order.
        (
            "elf-aarch64-be",
            with(with(elf(0), 5, &[2]), 18, &EM_AARCH64.to_be_bytes()),
            None,
        ),
        ("elf-truncated", elf(EM_X86_64)[..10].to_vec(), None),
        ("pe-dos-stub-only", pe(PE_AMD64)[..0x20].to_vec(), None),
        ("pe-i386", pe(0x14c), None),
        ("pe-armnt", pe(0x1c4), None),
        ("pe-no-signature", with(pe(PE_AMD64), 0x40, b"NE\0\0"), None),
        (
            "pe-lfanew-past-end",
            with(pe(PE_AMD64), 0x3c, &0x1000u32.to_le_bytes()),
            None,
        ),
        ("pe-truncated", pe(PE_AMD64)[..0x45].to_vec(), None),
        ("text", b"#!/bin/sh\necho iris\n".to_vec(), None),
        ("empty", Vec::new(), None),
    ];
    for (name, bytes, want) in cases {
        let path = fixture(dir.path(), name, &bytes);
        let got = lib("iris_binary_arch", &[path.as_os_str()]);
        match want {
            Some(arch) => assert_eq!(got.as_deref(), Ok(arch), "{name}"),
            None => {
                let err = got.expect_err(name);
                let prefix = format!(
                    "Error: {} is not an x86_64 or aarch64 ELF or PE executable (magic ",
                    path.display()
                );
                assert!(
                    err.starts_with(&prefix) && err.lines().count() == 1,
                    "{name}: {err}"
                );
            }
        }
    }
}

#[test]
fn resolve_arch_checks_the_requested_arch() {
    require(&["bash", "od", "tr"]);
    let dir = tempfile::tempdir().expect("tempdir");
    let x86_file = fixture(dir.path(), "x86", &elf(EM_X86_64));
    let arm_file = fixture(dir.path(), "arm", &pe(PE_ARM64));
    let (x86, arm) = (x86_file.as_path(), arm_file.as_path());
    let mismatch = |want: &str, file: &Path, have: &str| -> Result<String, String> {
        Err(format!(
            "Error: --arch {want}, but {} is built for {have}",
            file.display()
        ))
    };
    let unknown = |arch: &str| -> Result<String, String> {
        Err(format!("Error: --arch {arch} is not x86_64 or aarch64"))
    };
    let cases: Vec<(&Path, &str, Result<String, String>)> = vec![
        (x86, "", Ok("x86_64".into())),
        (x86, "x86_64", Ok("x86_64".into())),
        (x86, "amd64", Ok("x86_64".into())),
        (x86, "aarch64", mismatch("aarch64", x86, "x86_64")),
        (x86, "arm64", mismatch("aarch64", x86, "x86_64")),
        (arm, "", Ok("aarch64".into())),
        (arm, "aarch64", Ok("aarch64".into())),
        (arm, "arm64", Ok("aarch64".into())),
        (arm, "x86_64", mismatch("x86_64", arm, "aarch64")),
        (arm, "amd64", mismatch("x86_64", arm, "aarch64")),
        (x86, "riscv64", unknown("riscv64")),
        (x86, "X86_64", unknown("X86_64")),
        (x86, "i686", unknown("i686")),
        (arm, "armv7", unknown("armv7")),
        // The value is checked before the executable is read.
        (Path::new("/nonexistent/iris"), "x86", unknown("x86")),
    ];
    for (file, arch, want) in cases {
        let file = file.as_os_str();
        assert_eq!(
            lib("iris_resolve_arch", &[file, OsStr::new(arch)]),
            want,
            "{file:?} --arch {arch:?}"
        );
    }
}

/// Scripts that take `--bin` but no `--arch`, and why.
const NO_ARCH: [&str; 1] = [
    // --bin is the universal x86_64 + arm64 Mach-O lipo writes.
    "packaging/macos/make_app.sh",
];

#[test]
fn every_script_that_packages_an_executable_enforces_arch() {
    require(&["bash", "od", "tr", "awk"]);
    let mut scripts = Vec::new();
    for os in ["linux", "windows", "macos"] {
        for entry in std::fs::read_dir(Path::new(REPO).join("packaging").join(os)).expect("ls") {
            let path = entry.expect("entry").path();
            if path.extension() != Some(OsStr::new("sh")) {
                continue;
            }
            let rel = format!(
                "packaging/{os}/{}",
                path.file_name().unwrap().to_string_lossy()
            );
            let help = lib("iris_help", &[&rel]).expect("iris_help");
            if help.contains("--bin <path>") && !NO_ARCH.contains(&rel.as_str()) {
                assert!(help.contains("--arch"), "{rel} --help lists no --arch");
                scripts.push(rel);
            }
        }
    }
    scripts.sort();
    assert!(
        scripts.len() >= 5,
        "found only {scripts:?} packaging an executable"
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let x86 = fixture(dir.path(), "iris", &elf(EM_X86_64));
    let arm = fixture(dir.path(), "iris.exe", &pe(PE_ARM64));
    let out = dir.path().join("out");
    let cases = [
        (
            &x86,
            "aarch64",
            format!(
                "Error: --arch aarch64, but {} is built for x86_64",
                x86.display()
            ),
        ),
        (
            &arm,
            "amd64",
            format!(
                "Error: --arch x86_64, but {} is built for aarch64",
                arm.display()
            ),
        ),
        (
            &x86,
            "riscv64",
            "Error: --arch riscv64 is not x86_64 or aarch64".to_string(),
        ),
    ];
    for rel in &scripts {
        for (bin, arch, want) in &cases {
            let run = script(
                "022",
                rel,
                &[
                    OsStr::new("--bin"),
                    bin.as_os_str(),
                    OsStr::new("--out"),
                    out.as_os_str(),
                    OsStr::new("--arch"),
                    OsStr::new(arch),
                ],
            );
            let stderr = String::from_utf8_lossy(&run.stderr);
            assert_eq!(run.status.code(), Some(1), "{rel} --arch {arch}: {stderr}");
            assert!(
                stderr.lines().any(|l| l == want.as_str()),
                "{rel} --arch {arch}: stderr {stderr:?}, want {want:?}"
            );
            assert!(!out.exists(), "{rel} --arch {arch} created --out");
        }
    }
}

/// The width and height in the IHDR chunk of the PNG at `path`.
fn png_size(path: &Path) -> (u32, u32) {
    let b = std::fs::read(path).expect("read png");
    assert_eq!(
        &b[..8],
        b"\x89PNG\r\n\x1a\n",
        "{} is not a PNG",
        path.display()
    );
    assert_eq!(&b[12..16], b"IHDR", "{}", path.display());
    let be = |at: usize| u32::from_be_bytes(b[at..at + 4].try_into().unwrap());
    (be(16), be(20))
}

#[test]
fn hicolor_icons_do_not_depend_on_the_build_host() {
    require(&["bash", "install"]);
    let dir = tempfile::tempdir().expect("tempdir");
    // A python3 without Pillow, then the PATH the test runs with.
    let shim = dir.path().join("bin");
    std::fs::create_dir(&shim).expect("mkdir");
    fixture(&shim, "python3", b"#!/bin/sh\nexit 1\n");
    let path = std::env::var("PATH").unwrap_or_default();
    for (host, path) in [
        (
            "python3 without Pillow",
            format!("{}:{path}", shim.display()),
        ),
        ("the test's PATH", path),
    ] {
        let out = dir.path().join(host.replace(' ', "-"));
        let run = Command::new("bash")
            .arg("-c")
            .arg(". packaging/lib.sh && iris_hicolor_icons \"$1\" \"$2\"")
            .arg("bash")
            .arg(REPO)
            .arg(&out)
            .env("PATH", path.as_str())
            .current_dir(REPO)
            .output()
            .expect("run bash");
        assert_success(host, &run);
        let mut sizes: Vec<u32> = std::fs::read_dir(&out)
            .expect("ls")
            .map(|e| {
                let name = e.expect("entry").file_name().into_string().expect("utf-8");
                let (w, h) = name.split_once('x').expect("{size}x{size}");
                assert_eq!(w, h, "{host}: {name}");
                w.parse().expect("size")
            })
            .collect();
        sizes.sort_unstable();
        assert_eq!(sizes, ICON_SIZES, "{host}");
        for s in ICON_SIZES {
            let png = out.join(format!("{s}x{s}/apps/iris.png"));
            assert_eq!(png_size(&png), (s, s), "{host}: {}", png.display());
            let mode = std::fs::metadata(&png).expect("stat").permissions().mode() & 0o7777;
            assert_eq!(mode, 0o644, "{host}: {}", png.display());
        }
    }
}

/// The Debian architecture of the host, which `true` is built for.
fn deb_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => panic!("no iris package for {other}"),
    }
}

#[test]
fn deb_modes_version_and_sidecar() {
    require(&[
        "bash",
        "od",
        "tr",
        "dpkg-deb",
        "readelf",
        "sha256sum",
        "true",
    ]);
    let arch = std::env::consts::ARCH;
    let name = format!("iris-0.2.0-beta.1-linux-{arch}.deb");
    for umask in UMASKS {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = script(
            umask,
            "packaging/linux/build_deb.sh",
            &[
                OsStr::new("--bin"),
                OsStr::new("/usr/bin/true"),
                OsStr::new("--out"),
                dir.path().as_os_str(),
                OsStr::new("--version"),
                OsStr::new("0.2.0-beta.1"),
            ],
        );
        assert_success(&format!("build_deb.sh under umask {umask}"), &run);
        let deb = dir.path().join(&name);

        let fields = Command::new("dpkg-deb")
            .arg("-f")
            .arg(deb.as_os_str())
            .args(["Package", "Version", "Architecture"])
            .output()
            .expect("run dpkg-deb");
        assert_success("dpkg-deb -f", &fields);
        assert_eq!(
            String::from_utf8_lossy(&fields.stdout),
            format!(
                "Package: iris\nVersion: 0.2.0~beta.1\nArchitecture: {}\n",
                deb_arch()
            ),
            "umask {umask}"
        );

        let list = Command::new("dpkg-deb")
            .arg("-c")
            .arg(deb.as_os_str())
            .output()
            .expect("run dpkg-deb");
        assert_success("dpkg-deb -c", &list);
        let entries: BTreeMap<String, String> = String::from_utf8_lossy(&list.stdout)
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                (f[f.len() - 1].to_string(), f[0].to_string())
            })
            .collect();
        for (path, mode) in &entries {
            let want = if path.ends_with('/') {
                "drwxr-xr-x"
            } else if path == "./usr/bin/iris" {
                "-rwxr-xr-x"
            } else {
                "-rw-r--r--"
            };
            assert_eq!(mode, want, "umask {umask}: {path}");
        }
        let mut want = vec![
            "./usr/bin/iris".to_string(),
            "./usr/share/doc/iris/copyright".to_string(),
            "./usr/share/applications/dev.iris.app.desktop".to_string(),
            "./usr/share/metainfo/dev.iris.app.metainfo.xml".to_string(),
            "./etc/xdg/autostart/iris-autostart.desktop".to_string(),
        ];
        want.extend(ICON_SIZES.map(|s| format!("./usr/share/icons/hicolor/{s}x{s}/apps/iris.png")));
        for path in want {
            assert!(
                entries.contains_key(&path),
                "umask {umask}: the deb lacks {path}"
            );
        }

        let sidecar = std::fs::read_to_string(dir.path().join(format!("{name}.sha256")))
            .expect("read sidecar");
        let (hex, file) = sidecar
            .strip_suffix('\n')
            .and_then(|s| s.split_once("  "))
            .expect("sha256sum format");
        assert_eq!(file, name);
        assert!(
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{hex}"
        );
        let check = Command::new("sha256sum")
            .args(["-c", &format!("{name}.sha256")])
            .current_dir(dir.path())
            .output()
            .expect("run sha256sum");
        assert_success("sha256sum -c", &check);
    }
}

/// The entries of the zip at `path`: name to the Unix mode in its
/// external attributes.
fn zip_modes(path: &Path) -> BTreeMap<String, u32> {
    let b = std::fs::read(path).expect("read zip");
    let u16_at = |at: usize| u16::from_le_bytes(b[at..at + 2].try_into().unwrap()) as usize;
    let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let eocd = (0..=b.len() - 22)
        .rev()
        .find(|&at| b[at..at + 4] == *b"PK\x05\x06")
        .expect("end of central directory");
    let mut at = u32_at(eocd + 16) as usize;
    let mut modes = BTreeMap::new();
    for _ in 0..u16_at(eocd + 10) {
        assert_eq!(&b[at..at + 4], b"PK\x01\x02", "central directory entry");
        assert_eq!(b[at + 5], 3, "entry made on Unix");
        let (name_len, extra_len, comment_len) =
            (u16_at(at + 28), u16_at(at + 30), u16_at(at + 32));
        let name = String::from_utf8(b[at + 46..at + 46 + name_len].to_vec()).expect("utf-8");
        modes.insert(name, u32_at(at + 38) >> 16);
        at += 46 + name_len + extra_len + comment_len;
    }
    modes
}

#[test]
fn portable_zip_modes() {
    require(&["bash", "od", "tr", "sha256sum"]);
    let bins = tempfile::tempdir().expect("tempdir");
    let exe = fixture(bins.path(), "iris.exe", &pe(PE_AMD64));
    let want = BTreeMap::from([
        ("iris/".to_string(), 0o40755),
        ("iris/iris.exe".to_string(), 0o100755),
        ("iris/LICENSE-MIT".to_string(), 0o100644),
        ("iris/LICENSE-APACHE".to_string(), 0o100644),
        ("iris/Inter-OFL.txt".to_string(), 0o100644),
    ]);
    for umask in UMASKS {
        let dir = tempfile::tempdir().expect("tempdir");
        let run = script(
            umask,
            "packaging/windows/build_portable.sh",
            &[
                OsStr::new("--bin"),
                exe.as_os_str(),
                OsStr::new("--out"),
                dir.path().as_os_str(),
                OsStr::new("--version"),
                OsStr::new("0.2.0-beta.1"),
            ],
        );
        assert_success(&format!("build_portable.sh under umask {umask}"), &run);
        let zip = dir
            .path()
            .join("iris-0.2.0-beta.1-windows-x86_64-portable.zip");
        assert_eq!(zip_modes(&zip), want, "umask {umask}");
    }
}

#[test]
fn app_bundle_modes_and_versions() {
    require(&["bash", "sed", "install", "grep"]);
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = fixture(dir.path(), "iris", b"universal iris\n");
    let want = BTreeMap::from([
        ("", 0o755),
        ("Contents", 0o755),
        ("Contents/Info.plist", 0o644),
        ("Contents/PkgInfo", 0o644),
        ("Contents/MacOS", 0o755),
        ("Contents/MacOS/iris", 0o755),
        ("Contents/Resources", 0o755),
        ("Contents/Resources/iris.icns", 0o644),
        ("Contents/Resources/LICENSE-MIT", 0o644),
        ("Contents/Resources/LICENSE-APACHE", 0o644),
        ("Contents/Resources/Inter-OFL.txt", 0o644),
    ]);
    for umask in UMASKS {
        let out = dir.path().join(format!("out-{umask}"));
        let run = script(
            umask,
            "packaging/macos/make_app.sh",
            &[
                OsStr::new("--bin"),
                bin.as_os_str(),
                OsStr::new("--out"),
                out.as_os_str(),
                OsStr::new("--version"),
                OsStr::new("0.2.0-beta.1"),
            ],
        );
        assert_success(&format!("make_app.sh under umask {umask}"), &run);
        let app = out.join("iris.app");
        let mut modes = BTreeMap::new();
        let mut stack = vec![app.clone()];
        while let Some(path) = stack.pop() {
            let meta = std::fs::symlink_metadata(&path).expect("stat");
            if meta.is_dir() {
                for entry in std::fs::read_dir(&path).expect("ls") {
                    stack.push(entry.expect("entry").path());
                }
            }
            let rel = path
                .strip_prefix(&app)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            modes.insert(rel, meta.permissions().mode() & 0o7777);
        }
        let want: BTreeMap<String, u32> = want.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        assert_eq!(modes, want, "umask {umask}");

        let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).expect("plist");
        for (key, value) in [
            ("CFBundleShortVersionString", "0.2.0"),
            ("CFBundleVersion", "0.2.0"),
            ("IrisVersion", "0.2.0-beta.1"),
        ] {
            let after = plist
                .split_once(&format!("<key>{key}</key>"))
                .unwrap_or_else(|| panic!("Info.plist has no {key}"))
                .1;
            let got = after
                .split_once("<string>")
                .and_then(|(_, s)| s.split_once("</string>"))
                .expect("<string>")
                .0;
            assert_eq!(got, value, "{key}");
        }
    }
}

/// WHY: the AppImage embeds the type2-runtime and is assembled by
/// appimagetool, both downloaded while it builds. They came from the
/// moving `continuous` releases with no checksum, so a release AppImage
/// held whatever runtime was current on its build day. Every download
/// is now a tagged release checked against a SHA-256 recorded in the
/// script before use: other bytes fail the build and write no AppImage,
/// for the runtime of either arch, for appimagetool, and for a cached
/// appimagetool in .build-staging/. A host with no pinned appimagetool
/// fails before any download.
///
/// Not caught: a wrong hash recorded for the right file (the package
/// workflow's AppImage build fails on it), and an `--appimagetool` the
/// caller names, which is run as given.
#[test]
fn appimage_tool_downloads_are_pinned() {
    require(&["bash", "od", "tr", "sha256sum", "cut"]);
    let dir = tempfile::tempdir().expect("tempdir");
    // A repository holding only what the script reads before it
    // downloads, so an appimagetool cached in this checkout is not used.
    let root = dir.path().join("repo");
    for file in ["packaging/lib.sh", "packaging/linux/build_appimage.sh"] {
        let to = root.join(file);
        std::fs::create_dir_all(to.parent().expect("parent")).expect("mkdir");
        std::fs::copy(Path::new(REPO).join(file), &to).expect("copy");
    }
    let shim = dir.path().join("bin");
    std::fs::create_dir(&shim).expect("mkdir");
    let urls = dir.path().join("urls");
    // `curl ... -o FILE URL`: records URL, writes bytes no pin matches.
    let curl = format!(
        "#!/bin/sh\nwhile [ $# -gt 1 ]; do [ \"$1\" = -o ] && out=$2; shift; done\n\
         echo \"$1\" >> '{}'\necho tampered > \"$out\"\n",
        urls.display()
    );
    fixture(&shim, "curl", curl.as_bytes());
    let riscv = dir.path().join("riscv-bin");
    std::fs::create_dir(&riscv).expect("mkdir");
    fixture(&riscv, "uname", b"#!/bin/sh\necho riscv64\n");
    let x86 = fixture(dir.path(), "iris-x86_64", &elf(EM_X86_64));
    let arm = fixture(dir.path(), "iris-aarch64", &elf(EM_AARCH64));
    let host_path = std::env::var("PATH").unwrap_or_default();
    let dist = dir.path().join("dist");

    let run = |what: &str, bin: &Path, tool: Option<&str>, riscv_host: bool| {
        let _ = std::fs::remove_file(&urls);
        let mut path = format!("{}:{host_path}", shim.display());
        if riscv_host {
            path = format!("{}:{path}", riscv.display());
        }
        let mut cmd = Command::new("bash");
        cmd.arg(root.join("packaging/linux/build_appimage.sh"))
            .arg("--bin")
            .arg(bin)
            .arg("--out")
            .arg(&dist)
            .args(["--version", "0.1.0"]);
        if let Some(tool) = tool {
            cmd.args(["--appimagetool", tool]);
        }
        let out = cmd.env("PATH", path).output().expect("run bash");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            !out.status.success(),
            "{what}: built with tampered tools\n{stderr}"
        );
        let built: Vec<_> = std::fs::read_dir(&dist)
            .map(|d| d.map(|e| e.expect("entry").file_name()).collect())
            .unwrap_or_default();
        assert!(built.is_empty(), "{what}: wrote {built:?}");
        let fetched: Vec<String> = std::fs::read_to_string(&urls)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        (stderr, fetched)
    };
    // URL is a file of a tagged release of AppImage/REPO named NAME;
    // returns the tag.
    let pinned = |what: &str, url: &str, repo: &str, name: &str| -> String {
        let prefix = format!("https://github.com/AppImage/{repo}/releases/download/");
        let (tag, file) = url
            .strip_prefix(&prefix)
            .and_then(|rest| rest.split_once('/'))
            .unwrap_or_else(|| panic!("{what}: {url} is not a {repo} release download"));
        assert_eq!(file, name, "{what}: {url}");
        assert!(
            tag != "continuous" && !tag.is_empty(),
            "{what}: {url} is not a tagged release"
        );
        tag.to_owned()
    };
    let rejected = |what: &str, stderr: &str, url: &str| {
        assert!(
            stderr.contains(&format!("Error: {url} has SHA-256 "))
                && stderr.contains(", not the pinned "),
            "{what}: {stderr}"
        );
    };

    for (arch, bin) in [("x86_64", &x86), ("aarch64", &arm)] {
        let what = format!("{arch} runtime");
        let (stderr, fetched) = run(&what, bin, Some("/bin/true"), false);
        let [url] = fetched.as_slice() else {
            panic!("{what}: fetched {fetched:?}");
        };
        pinned(&what, url, "type2-runtime", &format!("runtime-{arch}"));
        rejected(&what, &stderr, url);
    }

    let host = std::env::consts::ARCH;
    let tool_name = format!("appimagetool-{host}.AppImage");
    let (stderr, fetched) = run("appimagetool", &x86, None, false);
    let [url] = fetched.as_slice() else {
        panic!("appimagetool: fetched {fetched:?}");
    };
    let tag = pinned("appimagetool", url, "appimagetool", &tool_name);
    rejected("appimagetool", &stderr, url);
    let cache = root.join(format!(".build-staging/appimagetool-{tag}-{host}.AppImage"));
    assert!(
        !cache.exists(),
        "a rejected appimagetool was cached at {}",
        cache.display()
    );

    // An executable cached appimagetool with other bytes is fetched again.
    fixture(
        cache.parent().expect("parent"),
        cache.file_name().expect("name").to_str().expect("utf-8"),
        b"#!/bin/sh\nexit 0\n",
    );
    let (stderr, fetched) = run("cached appimagetool", &x86, None, false);
    assert_eq!(
        fetched,
        std::slice::from_ref(url),
        "cached appimagetool: {stderr}"
    );
    rejected("cached appimagetool", &stderr, url);

    let (stderr, fetched) = run("riscv64 host", &x86, None, true);
    assert!(fetched.is_empty(), "riscv64 host: fetched {fetched:?}");
    assert!(
        stderr.contains("Error: no pinned appimagetool-riscv64.AppImage exists"),
        "riscv64 host: {stderr}"
    );
}
