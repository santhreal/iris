use super::*;
use crate::update::tests::{
    million_a, test_key, ASSET, LEGACY, OTHER_FILE, OTHER_KEY, OTHER_VERSION, SIGNED,
};

/// `million_a()` in a file of its own.
fn asset_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(ASSET);
    std::fs::write(&path, million_a()).expect("write");
    (dir, path)
}

fn v999() -> semver::Version {
    semver::Version::new(9, 9, 9)
}

/// The trusted comment is the one the release workflow signs with:
/// `minisign -t "file:<asset> version:<version>"`, prerelease included.
#[test]
fn the_trusted_comment_states_the_asset_and_the_cargo_version() {
    assert_eq!(
        trusted_comment(ASSET, &v999()),
        "file:iris-9.9.9-linux-x86_64.AppImage version:9.9.9"
    );
    assert_eq!(
        trusted_comment(
            "iris-0.2.0-beta.1-linux-x86_64.deb",
            &semver::Version::parse("0.2.0-beta.1").expect("semver")
        ),
        "file:iris-0.2.0-beta.1-linux-x86_64.deb version:0.2.0-beta.1"
    );
}

// WHY: the class closed here is "`verify` accepts a signature that is
// not the release workflow's for this file": each field of a minisign
// signature (key, file hash, trusted comment and its global signature,
// algorithm) is checked, against the file's bytes read whole. Not
// covered: the download around it (update::tests).

#[test]
fn the_release_signature_of_the_file_verifies() {
    let (_dir, path) = asset_file();
    assert_eq!(verify(&path, SIGNED, &test_key(), ASSET, &v999()), Ok(()));
}

#[test]
fn every_other_signature_is_rejected() {
    let (_dir, path) = asset_file();
    let rewritten = OTHER_VERSION.replace("version:9.9.8", "version:9.9.9");
    let truncated = std::fs::read(&path).expect("read");
    let short = tempfile::NamedTempFile::new().expect("tempfile");
    std::fs::write(short.path(), &truncated[..truncated.len() - 1]).expect("write");
    let other_key = PublicKey::decode(include_str!("../fixtures/other.pub")).expect("key");
    let mismatch = format!(
        "update: {ASSET} does not match its signature {ASSET}.minisig: \
         The signature verification failed"
    );
    let cases: [(&str, &Path, &str, PublicKey, String); 8] = [
        (
            "a signature by another key",
            &path,
            OTHER_KEY,
            test_key(),
            format!(
                "update: {ASSET}.minisig is not a release signature: \
                 The signature was created with a different key than the one provided"
            ),
        ),
        (
            "the release signature checked with another key",
            &path,
            SIGNED,
            other_key,
            format!(
                "update: {ASSET}.minisig is not a release signature: \
                 The signature was created with a different key than the one provided"
            ),
        ),
        (
            "a file one byte short",
            short.path(),
            SIGNED,
            test_key(),
            mismatch.clone(),
        ),
        (
            "a comment naming another file",
            &path,
            OTHER_FILE,
            test_key(),
            format!(
                "update: {ASSET}.minisig signs \"file:iris-9.9.9-macos-universal.dmg \
                 version:9.9.9\", not \"file:{ASSET} version:9.9.9\""
            ),
        ),
        (
            "a comment naming another version",
            &path,
            OTHER_VERSION,
            test_key(),
            format!(
                "update: {ASSET}.minisig signs \"file:{ASSET} version:9.9.8\", \
                 not \"file:{ASSET} version:9.9.9\""
            ),
        ),
        (
            "a comment rewritten after signing",
            &path,
            &rewritten,
            test_key(),
            mismatch,
        ),
        (
            "a legacy signature",
            &path,
            LEGACY,
            test_key(),
            format!(
                "update: {ASSET}.minisig is not a release signature: \
                 StreamVerifier only supports non-legacy mode signatures"
            ),
        ),
        (
            "an empty .minisig",
            &path,
            "",
            test_key(),
            format!(
                "update: {ASSET}.minisig is not a minisign signature: \
                 Invalid encoding in minisign data"
            ),
        ),
    ];
    for (case, file, minisig, key, want) in cases {
        assert_eq!(
            verify(file, minisig, &key, ASSET, &v999()),
            Err(want),
            "{case}"
        );
    }
}

#[test]
fn a_missing_file_is_an_error_naming_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(ASSET);
    let err = verify(&path, SIGNED, &test_key(), ASSET, &v999()).expect_err("missing");
    assert!(
        err.starts_with(&format!("update: open {}: ", path.display())),
        "{err}"
    );
}

/// The built-in key is packaging/minisign.pub, key id EC8B4C1097043E88,
/// the key the release workflow signs with. Every installed iris holds
/// it, so a new key here leaves those installs unable to verify any
/// later release. It is not the test key.
#[test]
fn the_built_in_key_is_the_published_release_key() {
    let mut lines = RELEASE_KEY.lines();
    assert_eq!(
        lines.next(),
        Some("untrusted comment: minisign public key EC8B4C1097043E88")
    );
    assert_eq!(
        lines.next(),
        Some("RWSIPgSXEEyL7E0nxtQR83kxxdTJGm0GcQ1/J/EOUgG+cCDn9RJjIYox")
    );
    let key = release_key().expect("the release key decodes");
    let (_dir, path) = asset_file();
    let err = verify(&path, SIGNED, &key, ASSET, &v999()).expect_err("a test signature");
    assert!(
        err.ends_with("different key than the one provided"),
        "{err}"
    );
}
