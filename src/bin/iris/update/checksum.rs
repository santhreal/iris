//! The SHA-256 check of an update download against the `.sha256`
//! sidecar its release publishes beside it (`packaging/CONTRACT.md`).

use std::io::{self, Write};

/// A SHA-256 digest.
pub(super) type Digest = [u8; 32];

/// The most bytes read of a sidecar. Its one `sha256sum` line is 64 hex
/// digits, a separator, and the asset name.
pub(super) const SIDECAR_MAX: u64 = 1024;

/// The digest a `sha256sum`-format sidecar lists for `asset`: 64 hex
/// digits, whitespace, and the file name, which `sha256sum` in binary
/// mode prefixes with `*`. A sidecar that lists anything else, or names
/// another file, is an error.
pub(super) fn parse_sidecar(text: &str, asset: &str) -> Result<Digest, String> {
    let malformed = || format!("update: {asset}.sha256 is not one `sha256sum` line for {asset}");
    let mut words = text.split_whitespace();
    let (Some(hex), Some(file), None) = (words.next(), words.next(), words.next()) else {
        return Err(malformed());
    };
    if file.strip_prefix('*').unwrap_or(file) != asset || hex.len() != 64 {
        return Err(malformed());
    }
    let mut digest = [0; 32];
    for (byte, &[hi, lo]) in digest.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0) {
        let (Some(hi), Some(lo)) = (nibble(hi), nibble(lo)) else {
            return Err(malformed());
        };
        *byte = hi << 4 | lo;
    }
    Ok(digest)
}

/// The value of one hex digit, either case.
fn nibble(digit: u8) -> Option<u8> {
    char::from(digit).to_digit(16).map(|d| d as u8)
}

/// Ok when `got`, the SHA-256 of the downloaded `asset`, is the `want`
/// its sidecar lists.
pub(super) fn verify(asset: &str, got: &ring::digest::Digest, want: &Digest) -> Result<(), String> {
    if got.as_ref() == want {
        return Ok(());
    }
    Err(format!(
        "update: the downloaded {asset} has SHA-256 {}, its .sha256 sidecar lists {}; \
         nothing was installed",
        hex(got.as_ref()),
        hex(want)
    ))
}

/// `bytes` as lowercase hex, the form `sha256sum` prints.
pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(2 * bytes.len()), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// A writer that hashes each byte it writes to `inner`.
pub(super) struct Sha256Writer<W> {
    inner: W,
    sha: ring::digest::Context,
}

impl<W> Sha256Writer<W> {
    pub(super) fn new(inner: W) -> Self {
        Self {
            inner,
            sha: ring::digest::Context::new(&ring::digest::SHA256),
        }
    }

    /// The SHA-256 of every byte written. Drops `inner`, which closes a
    /// file.
    pub(super) fn finish(self) -> ring::digest::Digest {
        self.sha.finish()
    }
}

impl<W: Write> Write for Sha256Writer<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.sha.update(&buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
