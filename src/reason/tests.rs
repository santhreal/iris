//! WHY: the class closed here is "a failure shows a developer string".
//! The editor, the toast, the pin, the library preview, Copy Image, Copy
//! Path, and Copy Text reported a failed open as the file's full path and
//! the read's or the decoder's own error ("read /home/x/broken.png
//! header: failed to fill whole buffer"), which spent a notice's three
//! lines on the directory. These cases build each error the reads return,
//! or open a real file the way those paths do, and assert the exact
//! sentence: the file's name, never its directory, and the cause in
//! words. The editor's PNG header and the copies are covered beside
//! their code (`editor::tests`, `pipeline::tests`). Not covered: a read
//! that fails on a device error, whose sentence keeps the system's text.

use std::io::{Error, ErrorKind};
use std::path::Path;

use super::*;

#[test]
fn a_read_error_maps_its_kind_to_a_sentence() {
    let path = Path::new("/home/someone/Pictures/shot.png");
    for (e, want) in [
        (
            Error::from(ErrorKind::NotFound),
            "shot.png no longer exists.",
        ),
        (
            Error::from(ErrorKind::PermissionDenied),
            "No permission to read shot.png.",
        ),
        (
            Error::other("disk on fire"),
            "shot.png did not read: disk on fire.",
        ),
    ] {
        assert_eq!(unreadable(path, &e), want);
        assert_eq!(not_an_image(path, &image::ImageError::IoError(e)), want);
    }
    assert_eq!(
        not_an_image(
            path,
            &image::ImageError::IoError(ErrorKind::UnexpectedEof.into())
        ),
        "shot.png is not a readable image.",
        "a stream that ends early is a damaged image"
    );
    assert_eq!(not_png(path), "shot.png is not a PNG image.");
    assert_eq!(file_name(Path::new("/")), "/");
}

#[test]
fn a_file_that_does_not_open_is_stated_by_name_and_cause() {
    let dir = tempfile::tempdir().unwrap();
    let at = |name: &str| dir.path().join(name);
    std::fs::write(
        at("text.png"),
        b"not an image, and longer than a PNG header",
    )
    .unwrap();
    std::fs::write(
        at("damaged.png"),
        b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x08\x00\x00\x00\x08garbage",
    )
    .unwrap();
    let decoded = |name: &str| {
        let path = at(name);
        image::open(&path)
            .map(|_| ())
            .map_err(|e| not_an_image(&path, &e))
    };
    let cases = [
        (
            "decode, missing",
            decoded("gone.png"),
            "gone.png no longer exists.",
        ),
        (
            "decode, text",
            decoded("text.png"),
            "text.png is not a readable image.",
        ),
        (
            "decode, a damaged PNG",
            decoded("damaged.png"),
            "damaged.png is not a readable image.",
        ),
        (
            "Copy Text, missing",
            crate::ocr::recognize_text(&at("gone.png")).map(|_| ()),
            "gone.png no longer exists.",
        ),
    ];
    let shown = dir.path().display().to_string();
    for (case, got, want) in cases {
        let got = got.expect_err(case);
        assert_eq!(got, want, "{case}");
        assert!(!got.contains(&shown), "{case}: {got:?} shows the directory");
    }
}
