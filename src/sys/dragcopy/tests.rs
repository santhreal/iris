// WHY: the class closed here is "a file URI names another file": a path
// with a space, a '#', a '%', or UTF-8 must encode to a URI a file
// manager accepts and decode back to the same path, and an escape that
// is not two hex digits must stay as it is instead of decoding to a byte
// or panicking on a UTF-8 boundary. Not covered: non-UTF-8 paths, which
// decode lossily.

use std::path::PathBuf;

use super::{uri_decode_path, uri_encode_path};

#[test]
fn decodes_percent_escapes() {
    assert_eq!(
        uri_decode_path("/home/u/my%20shot%20%231.png"),
        PathBuf::from("/home/u/my shot #1.png")
    );
}

#[test]
fn decodes_utf8_sequences() {
    assert_eq!(
        uri_decode_path("/tmp/%C3%A9cran.png"),
        PathBuf::from("/tmp/écran.png")
    );
}

#[test]
fn leaves_plain_and_malformed_input_alone() {
    for literal in [
        "/a/b.png",
        // A trailing '%', a short escape, and non-hex digits.
        "/a/100%.png",
        "/a/100%",
        "/a/%2",
        "/a/%zz.png",
        // A sign parses as part of a number, never as a hex digit.
        "/a/%+1.png",
        // A '%' before a multibyte character: the escape's second byte
        // falls inside the character.
        "/a/%aé.png",
        "/a/%é.png",
    ] {
        assert_eq!(
            uri_decode_path(literal),
            PathBuf::from(literal),
            "{literal}"
        );
    }
}

#[test]
fn decoding_an_encoded_path_gives_the_path() {
    for path in [
        "/home/u/a b.png",
        "/home/u/#1?x=y&z;.png",
        "/home/u/100% done.png",
        "/home/u/écran 画面.png",
        "/home/u/%41.png",
    ] {
        let uri = uri_encode_path(path);
        assert!(
            uri.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.~/%".contains(&b)),
            "{path} encodes to {uri}"
        );
        assert_eq!(uri_decode_path(&uri), PathBuf::from(path), "{uri}");
    }
}
