use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use super::checksum::{hex, parse_sidecar, Sha256Writer};
use super::*;

/// SHA-256 of `abc`, the FIPS 180-2 one-block vector.
const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
/// SHA-256 of one million `a` bytes, the FIPS 180-2 long-message vector.
const MILLION_A: &str = "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0";

const ASSET: &str = "iris-9.9.9-linux-x86_64.AppImage";
const SELECTOR: &str = "linux-x86_64.AppImage";

fn info(base: &str, asset_name: &str) -> UpdateInfo {
    UpdateInfo {
        version: semver::Version::new(9, 9, 9),
        asset_url: format!("{base}/asset"),
        asset_name: asset_name.to_string(),
        checksum_url: format!("{base}/asset.sha256"),
    }
}

/// One path an HTTP test server answers.
struct Route {
    path: &'static str,
    body: Vec<u8>,
    /// The `Content-Length` sent. `None` sends none, so the body ends
    /// when the server closes the connection.
    declared: Option<usize>,
}

fn route(path: &'static str, body: impl Into<Vec<u8>>) -> Route {
    let body = body.into();
    let declared = Some(body.len());
    Route {
        path,
        body,
        declared,
    }
}

/// An HTTP/1.1 server on a loopback port answering each route's `GET`
/// and any other path with 404, until the test process exits. Returns
/// its base URL.
fn serve(routes: Vec<Route>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let base = format!("http://{}", listener.local_addr().expect("local addr"));
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut path = None;
            {
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                    if path.is_none() {
                        path = line.split_whitespace().nth(1).map(str::to_owned);
                    }
                    line.clear();
                }
            }
            let found = routes.iter().find(|r| Some(r.path) == path.as_deref());
            let (status, body, declared) = match found {
                Some(r) => ("200 OK", r.body.as_slice(), r.declared),
                None => ("404 Not Found", &[][..], Some(0)),
            };
            let length = declared.map_or(String::new(), |n| format!("Content-Length: {n}\r\n"));
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\n{length}Connection: close\r\n\r\n"
            );
            let _ = stream.write_all(body);
        }
    });
    base
}

fn sidecar(digest: &str, name: &str) -> String {
    format!("{digest}  {name}\n")
}

fn is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).expect("read dir").next().is_none()
}

#[test]
fn a_download_that_matches_its_sidecar_is_kept() {
    let body = vec![b'a'; 1_000_000];
    let base = serve(vec![
        route("/asset", body.clone()),
        route("/asset.sha256", sidecar(MILLION_A, ASSET)),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = download_into(&info(&base, ASSET), dir.path()).expect("verified download");
    assert_eq!(path, dir.path().join(ASSET));
    assert!(std::fs::read(&path).expect("read download") == body);
}

/// A file a failed install left in the update directory is the release
/// asset when its SHA-256 is the one the sidecar lists: the update uses
/// it and fetches only the sidecar. The server has no `/asset`, so a
/// download would fail.
#[test]
fn a_verified_file_already_there_is_not_downloaded_again() {
    let body = vec![b'a'; 1_000_000];
    let base = serve(vec![route("/asset.sha256", sidecar(MILLION_A, ASSET))]);
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(ASSET), &body).expect("write");
    let path = download_into(&info(&base, ASSET), dir.path()).expect("kept file");
    assert_eq!(path, dir.path().join(ASSET));
    assert!(std::fs::read(&path).expect("read") == body);
}

/// A file already there that is not the release asset is replaced by
/// the download, or deleted when the download fails too.
#[test]
fn a_stale_file_already_there_is_replaced_or_deleted() {
    let body = vec![b'a'; 1_000_000];
    let base = serve(vec![
        route("/asset", body.clone()),
        route("/asset.sha256", sidecar(MILLION_A, ASSET)),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(ASSET), b"abc").expect("write");
    let path = download_into(&info(&base, ASSET), dir.path()).expect("download");
    assert!(std::fs::read(&path).expect("read") == body);

    let base = serve(vec![
        route("/asset", "abc"),
        route("/asset.sha256", sidecar(MILLION_A, ASSET)),
    ]);
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(ASSET), &body[1..]).expect("write");
    let err = download_into(&info(&base, ASSET), dir.path()).expect_err("not the release");
    assert!(err.contains(&format!("has SHA-256 {ABC}")), "{err}");
    assert!(is_empty(dir.path()), "the stale file stayed behind");
}

/// Each download fails, and the update directory holds no file of it
/// that a later install could pick up.
#[test]
fn a_download_that_is_not_the_released_file_is_deleted() {
    let million_a = || vec![b'a'; 1_000_000];
    let half = || vec![b'a'; 500_000];
    let cases: [(&str, Vec<Route>, String); 6] = [
        (
            "another file's bytes",
            vec![
                route("/asset", "abc"),
                route("/asset.sha256", sidecar(MILLION_A, ASSET)),
            ],
            format!("has SHA-256 {ABC}, its .sha256 sidecar lists {MILLION_A}"),
        ),
        (
            "a body cut short of its Content-Length",
            vec![
                Route {
                    path: "/asset",
                    body: half(),
                    declared: Some(1_000_000),
                },
                route("/asset.sha256", sidecar(MILLION_A, ASSET)),
            ],
            format!("update: download {ASSET}: response body closed"),
        ),
        (
            "a body with no Content-Length cut short by a close",
            vec![
                Route {
                    path: "/asset",
                    body: half(),
                    declared: None,
                },
                route("/asset.sha256", sidecar(MILLION_A, ASSET)),
            ],
            format!("its .sha256 sidecar lists {MILLION_A}"),
        ),
        (
            "a release asset with no sidecar to fetch",
            vec![route("/asset", million_a())],
            "/asset.sha256: status code 404".to_string(),
        ),
        (
            "a sidecar for another asset",
            vec![
                route("/asset", million_a()),
                route(
                    "/asset.sha256",
                    sidecar(MILLION_A, "iris-9.9.9-macos-universal.dmg"),
                ),
            ],
            format!("update: {ASSET}.sha256 is not one `sha256sum` line for {ASSET}"),
        ),
        (
            "a sidecar with no digest",
            vec![route("/asset", million_a()), route("/asset.sha256", "")],
            format!("update: {ASSET}.sha256 is not one `sha256sum` line for {ASSET}"),
        ),
    ];
    for (case, routes, fragment) in cases {
        let base = serve(routes);
        let dir = tempfile::tempdir().expect("tempdir");
        let err = download_into(&info(&base, ASSET), dir.path()).expect_err(case);
        assert!(err.contains(&fragment), "{case}: {err}");
        assert!(is_empty(dir.path()), "{case}: a file stayed behind");
    }
}

#[test]
fn a_sidecar_lists_one_digest_for_the_asset() {
    for text in [
        sidecar(ABC, ASSET),
        format!("{ABC} *{ASSET}\n"),
        format!("{ABC}  {ASSET}\r\n"),
        format!("{}  {ASSET}", ABC.to_uppercase()),
    ] {
        let digest = parse_sidecar(&text, ASSET).expect(&text);
        assert_eq!(hex(&digest), ABC, "{text:?}");
    }
    for text in [
        String::new(),
        ABC.to_string(),
        sidecar(ABC, "iris-9.9.9-macos-universal.dmg"),
        format!("{ABC}  **{ASSET}"),
        sidecar(&ABC[..63], ASSET),
        sidecar(&format!("{ABC}0"), ASSET),
        // A sign, which u8::from_str_radix accepts before its digits.
        sidecar(&format!("+{}", &ABC[1..]), ASSET),
        sidecar(&format!("{}g", &ABC[..63]), ASSET),
        // 64 bytes, one of them a character of two.
        sidecar(&format!("{}é", &ABC[..62]), ASSET),
        format!("{}{}", sidecar(ABC, ASSET), sidecar(ABC, ASSET)),
    ] {
        assert_eq!(
            parse_sidecar(&text, ASSET),
            Err(format!(
                "update: {ASSET}.sha256 is not one `sha256sum` line for {ASSET}"
            )),
            "{text:?}"
        );
    }
}

/// A writer that takes one byte per call.
struct Trickle;

impl Write for Trickle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len().min(1))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_digest_covers_the_bytes_written_not_the_bytes_offered() {
    let mut out = Sha256Writer::new(Trickle);
    out.write_all(b"abc").expect("write");
    assert_eq!(hex(out.finish().as_ref()), ABC);
}

fn release(tag: &str, assets: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "tag_name": tag, "assets": assets })
}

fn asset(name: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "browser_download_url": format!("https://dl.example/{name}") })
}

#[test]
fn a_newer_release_offers_its_asset_and_the_asset_s_sidecar() {
    let assets = serde_json::json!([
        asset(&format!("{ASSET}.sha256")),
        asset("iris-9.9.9-macos-universal.dmg"),
        asset(ASSET),
    ]);
    let found = select(
        &release("v9.9.9", assets),
        &semver::Version::new(0, 1, 0),
        SELECTOR,
    )
    .expect("select")
    .expect("newer");
    assert_eq!(found.version, semver::Version::new(9, 9, 9));
    assert_eq!(found.asset_name, ASSET);
    assert_eq!(found.asset_url, format!("https://dl.example/{ASSET}"));
    assert_eq!(
        found.checksum_url,
        format!("https://dl.example/{ASSET}.sha256")
    );
}

#[test]
fn a_release_that_is_not_newer_offers_nothing() {
    let current = semver::Version::new(0, 1, 0);
    for tag in ["v0.1.0", "v0.0.9"] {
        let found = select(
            &release(tag, serde_json::json!([asset(ASSET)])),
            &current,
            SELECTOR,
        );
        assert!(matches!(found, Ok(None)), "{tag}: {found:?}");
    }
}

#[test]
fn a_newer_asset_with_no_sidecar_of_its_own_is_an_error() {
    let current = semver::Version::new(0, 1, 0);
    for assets in [
        serde_json::json!([asset(ASSET)]),
        serde_json::json!([asset(ASSET), asset("iris-9.9.9-macos-universal.dmg.sha256")]),
        serde_json::json!([asset(ASSET), { "name": format!("{ASSET}.sha256") }]),
    ] {
        let err = select(&release("v9.9.9", assets.clone()), &current, SELECTOR)
            .expect_err(&assets.to_string());
        assert_eq!(
            err,
            format!("update: release v9.9.9 has no {ASSET}.sha256 to check {ASSET} against")
        );
    }
}

#[test]
fn asset_names_that_leave_the_update_dir_are_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    for name in [
        "../iris.AppImage",
        "a/b.dmg",
        "/abs/setup.exe",
        "..",
        ".",
        "",
    ] {
        let err =
            download_into(&info("https://invalid.example", name), dir.path()).expect_err(name);
        assert!(err.starts_with("update: bad asset name"), "{name:?}: {err}");
    }
}

/// An install that cannot replace itself (a deb or rpm install, a
/// macOS binary outside a bundle) refuses before anything runs: no
/// download, and the daemon keeps running. The asset name here would
/// fail the download with a different error. An install that replaces
/// itself has nothing to refuse.
#[test]
fn a_refused_install_fails_before_the_download() {
    let Err(refusal) = crate::sys::install::ready() else {
        return;
    };
    assert_eq!(
        apply(&info("https://invalid.example", "../escape")).expect_err("refused"),
        refusal
    );
}

/// Whether a swap that failed after `stopped` starts the daemon again.
/// The match has no wildcard: a new `Quit` fails to compile here until
/// it has an answer.
fn restarts(stopped: Quit) -> bool {
    match stopped {
        Quit::NoneRan | Quit::StillRunning => false,
        Quit::Exited => true,
    }
}

/// A failed swap starts the daemon the update stopped, and only that
/// one, then reports whether it runs again beside the swap's error.
#[test]
fn a_failed_swap_restarts_the_daemon_it_stopped() {
    for stopped in [Quit::NoneRan, Quit::Exited, Quit::StillRunning] {
        for started in [true, false] {
            let mut ran = false;
            let got = after_failed_swap("update: replace x: EXDEV".into(), stopped, || {
                ran = true;
                started
            });
            assert_eq!(ran, restarts(stopped), "{stopped:?}");
            let want = match (restarts(stopped), started) {
                (false, _) => "update: replace x: EXDEV".to_string(),
                (true, true) => {
                    "update: replace x: EXDEV; the iris already installed runs again".to_string()
                }
                (true, false) => format!(
                    "update: replace x: EXDEV; the iris already installed did not start again, \
                     see {}",
                    iris_lib::dirs::log_file().display()
                ),
            };
            assert_eq!(got, want, "{stopped:?}, started {started}");
        }
    }
}
