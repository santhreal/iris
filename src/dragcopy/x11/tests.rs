// WHY: the class closed here is "a drag or copy starts with a path that
// cannot serve": an empty set or a vanished file must fail before any X11
// work begins, not mid-drag. Not covered: the XDnD wire protocol, which
// the on-rig QA scripts exercise against a live drop target.

use std::path::PathBuf;

use super::*;

#[test]
fn validate_rejects_empty_and_missing() {
    assert!(validate_drag_paths(Vec::new()).is_err());
    assert!(validate_drag_paths(vec![PathBuf::from("/nonexistent-xyz")]).is_err());
}

#[test]
fn uri_list_is_crlf_file_uris() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.png");
    let b = dir.path().join("b.png");
    std::fs::write(&a, b"x").unwrap();
    std::fs::write(&b, b"y").unwrap();
    let out = build_uri_list(&[a.clone(), b.clone()]);
    // RFC 2483: one absolute file URI per line, CRLF terminated.
    let lines: Vec<&str> = out.split("\r\n").collect();
    assert_eq!(
        lines.len(),
        3,
        "two URIs plus trailing empty after last CRLF"
    );
    assert!(lines[0].starts_with("file://"));
    assert!(lines[0].ends_with("a.png"));
    assert!(lines[1].ends_with("b.png"));
    assert_eq!(lines[2], "");
}

#[test]
fn validate_accepts_existing_paths() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.png");
    std::fs::write(&f, b"x").unwrap();
    let out = validate_drag_paths(vec![f.clone()]).unwrap();
    assert_eq!(out, vec![f]);
}

/// End to end on a live X server: `copy_abs_paths` owns CLIPBOARD and
/// answers a `text/uri-list` request with the encoded file URI. Runs
/// only with `IRIS_X11_TEST_DISPLAY` set (a private Xvfb), so it never
/// touches a desktop session's clipboard.
#[test]
fn copy_serves_uri_list_on_x11() {
    let Some(display) = std::env::var_os("IRIS_X11_TEST_DISPLAY") else {
        return;
    };
    std::env::set_var("DISPLAY", &display);
    std::env::remove_var("WAYLAND_DISPLAY");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a b.png");
    std::fs::write(&file, b"x").unwrap();
    let abs = std::fs::canonicalize(&file).unwrap();
    copy_abs_paths(std::slice::from_ref(&abs)).unwrap();

    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, CreateWindowAux, WindowClass};
    use x11rb::protocol::Event;
    let (conn, screen) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[screen].root;
    let win = conn.generate_id().unwrap();
    conn.create_window(
        0,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new(),
    )
    .unwrap();
    let clipboard = atom_cached(&conn, b"CLIPBOARD").unwrap();
    let uri = atom_cached(&conn, b"text/uri-list").unwrap();
    let prop = atom_cached(&conn, b"IRIS_TEST").unwrap();
    // The serve thread acquires the selection asynchronously.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let got = loop {
        conn.convert_selection(win, clipboard, uri, prop, x11rb::CURRENT_TIME)
            .unwrap();
        conn.flush().unwrap();
        let ev = conn.wait_for_event().unwrap();
        if let Event::SelectionNotify(n) = ev {
            if n.property != x11rb::NONE {
                let r = conn
                    .get_property(true, win, prop, AtomEnum::ANY, 0, 4096)
                    .unwrap()
                    .reply()
                    .unwrap();
                break String::from_utf8(r.value).unwrap();
            }
        }
        assert!(std::time::Instant::now() < deadline, "no uri-list served");
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let want = format!("file://{}\r\n", uri_encode_path(&abs.to_string_lossy()));
    assert_eq!(got, want);
}
