//! Minisign signatures of release assets (packaging/CONTRACT.md,
//! "Signatures"). The release workflow signs every asset with the key
//! whose public half is packaging/minisign.pub and writes the signature
//! to `<asset>.minisig`, with the trusted comment
//! `file:<asset> version:<version>`. An update installs an asset only
//! when that signature verifies against the built-in key and its
//! comment names the asset and the version the check selected: a
//! signed asset of another release or another platform is rejected.

use std::io::Read;
use std::path::Path;

use minisign_verify::{PublicKey, Signature};

/// packaging/minisign.pub, the key every release asset is signed with.
const RELEASE_KEY: &str = include_str!("../../../../packaging/minisign.pub");

/// The most of a `.minisig` read: a signature file is four short lines.
pub(super) const SIGNATURE_MAX: u64 = 4096;

/// The release signing key.
pub(super) fn release_key() -> Result<PublicKey, String> {
    PublicKey::decode(RELEASE_KEY)
        .map_err(|e| format!("update: the built-in release key does not decode: {e}"))
}

/// The trusted comment the release workflow signs `asset` of `version`
/// with.
pub(super) fn trusted_comment(asset: &str, version: &semver::Version) -> String {
    format!("file:{asset} version:{version}")
}

/// Ok when `minisig`, the text of `asset`'s `.minisig`, is a signature
/// by `key` of the file at `path` whose trusted comment names `asset`
/// and `version`.
pub(super) fn verify(
    path: &Path,
    minisig: &str,
    key: &PublicKey,
    asset: &str,
    version: &semver::Version,
) -> Result<(), String> {
    let signature = Signature::decode(minisig)
        .map_err(|e| format!("update: {asset}.minisig is not a minisign signature: {e}"))?;
    let want = trusted_comment(asset, version);
    if signature.trusted_comment() != want {
        return Err(format!(
            "update: {asset}.minisig signs \"{}\", not \"{want}\"",
            signature.trusted_comment()
        ));
    }
    let mut verifier = key
        .verify_stream(&signature)
        .map_err(|e| format!("update: {asset}.minisig is not a release signature: {e}"))?;
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("update: open {}: {e}", path.display()))?;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("update: read {}: {e}", path.display())),
        };
        verifier.update(&buf[..n]);
    }
    verifier
        .finalize()
        .map_err(|e| format!("update: {asset} does not match its signature {asset}.minisig: {e}"))
}

#[cfg(test)]
mod tests;
