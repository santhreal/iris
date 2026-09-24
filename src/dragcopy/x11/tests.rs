// WHY: the class closed here is "a drag starts with a path that cannot
// serve": an empty set or a vanished file must fail before any X11 work
// begins, not mid-drag. Not covered: the XDnD wire protocol, which the
// on-rig QA scripts exercise against a live drop target.

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
