// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where a downloadable model is fetched from, and what proves it arrived
//! whole.
//!
//! Concat carries no weights in its bundle: a cutout network, a Kokoro voice
//! bank and a whisper model together outweigh the editor many times over,
//! and most people never ask for all three. Each is fetched the first time a
//! feature needs one.
//!
//! Every model Concat offers is mirrored onto a release of its own
//! repository, and the mirror is what a download asks for first. Upstream -
//! the third party the mirror was filled from - is the second try and
//! nothing more: a model the editor offers should not stop existing because
//! somebody else moved a path, and the bytes that arrive should be the bytes
//! that were tested.
//!
//! The table of what is mirrored is `models/manifest.toml` at the root of
//! the repository, and `scripts/models.py --check` is what keeps it and the
//! engine's own tables saying the same thing. The tables are here rather
//! than parsed from that file because a model's identity is something the
//! engine should not be able to start without.

use std::path::Path;

/// The repository the mirror lives on.
pub const REPO: &str = "jub0t/Concat";

/// The release every model is mirrored on.
///
/// Bumped only when a model's bytes change - app releases point at it by
/// name, so an unchanged model is never re-uploaded. Must match `release`
/// in `models/manifest.toml`; the check script enforces that.
pub const RELEASE: &str = "models-v1";

/// Where `file` is mirrored.
pub fn mirror(file: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{RELEASE}/{file}")
}

/// Where to try for `file`, in order: our mirror, then the upstream it was
/// filled from.
///
/// Two and not one because a mirror that cannot be reached - a proxy that
/// blocks our host, a release still being published, a region that cannot
/// see it - should cost a retry rather than a feature.
pub fn sources(file: &str, upstream: &str) -> [String; 2] {
    [mirror(file), upstream.to_owned()]
}

/// Checks a finished download against the digest the table carries.
///
/// An empty `expected` passes: a model added to the tables is unverified
/// until the mirror has been filled once and has reported what it holds.
/// Anything else is compared, and a file that does not match is refused -
/// the caller deletes it rather than leaving something that looks installed.
pub fn verify(file: &Path, expected: &str) -> Result<(), String> {
    if expected.is_empty() {
        return Ok(());
    }
    let actual = sha256(file)?;
    if actual == expected {
        return Ok(());
    }
    Err(format!(
        "{} is not the file it should be: expected sha256 {expected}, got {actual}",
        file.display()
    ))
}

/// The SHA-256 of a file, as lowercase hex.
fn sha256(file: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut handle = std::fs::File::open(file)
        .map_err(|error| format!("could not read {}: {error}", file.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = handle
            .read(&mut buffer)
            .map_err(|error| format!("could not read {}: {error}", file.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write;
            let _ = write!(out, "{byte:02x}");
            out
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mirror_comes_before_upstream() {
        let [first, second] = sources("model.onnx", "https://elsewhere.example/model.onnx");
        assert_eq!(
            first,
            format!("https://github.com/{REPO}/releases/download/{RELEASE}/model.onnx")
        );
        assert_eq!(second, "https://elsewhere.example/model.onnx");
    }

    #[test]
    fn a_digest_is_checked_and_an_empty_one_is_not() {
        let dir = std::env::temp_dir().join("concat-models-verify");
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let file = dir.join("bytes");
        std::fs::write(&file, b"concat").expect("a temp file");
        // echo -n concat | sha256sum
        let known = "3f4c1a4b3c9fb1b0e24bd1ec0bd7b16d0db4dbf1fe2f3b5dcdba0bd38b8f8b3a";
        assert!(verify(&file, "").is_ok());
        assert!(verify(&file, known).is_err());
        let actual = sha256(&file).expect("a digest");
        assert_eq!(actual.len(), 64);
        assert!(verify(&file, &actual).is_ok());
        let _ = std::fs::remove_file(&file);
    }
}
