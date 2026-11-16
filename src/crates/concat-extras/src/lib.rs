// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! Extras: optional content a person downloads into Concat after installing
//! it - effect and transition packs, fonts, title presets, stickers, sound
//! packs - and the machinery that decides whether a download is what it
//! claims to be.
//!
//! An extra is one `.concatpkg`: a gzip'd tar whose first member is
//! `extra.toml` ([`manifest`]), which names every other member with its size
//! and SHA-256. A repository publishes an `index.json` listing the extras it
//! offers ([`index`]), signed with Ed25519 over its exact bytes, and each
//! entry carries the SHA-256 of its archive. So a signature vouches for the
//! index, the index vouches for the archive, and the archive's manifest
//! vouches for every file in it.
//!
//! Installing ([`install`]) checks all three, unpacks into a staging folder
//! with every path, type and size checked ([`archive`]), hands the result to
//! the caller's vet (the host makes effect packages load before they are
//! accepted), and only then moves it into place and writes the install
//! database ([`registry`]). Plans ([`resolve`]) put dependencies first and
//! refuse cycles. [`pack`] is the authoring side: a folder into an archive,
//! archives into an index, and the key that signs it.
//!
//! Nothing here touches the network or draws anything: the host fetches,
//! the window shows. And nothing an extra carries is ever run - an extra
//! is data that existing, sandboxed parts of the app read.

pub mod archive;
pub mod index;
pub mod install;
pub mod manifest;
pub mod pack;
pub mod registry;
pub mod resolve;

pub use index::{Entry, Index};
pub use manifest::{Category, Kind, Manifest};
pub use registry::{Record, Registry};

/// The version of Concat this was built as: what `min_app` is compared to.
/// The workspace shares one version, so the crate's is the app's.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Why something about an extra could not be done, in words a person can
/// act on.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// The manifest, the index or an archive is not well formed, or breaks
    /// a rule.
    #[error("{0}")]
    Invalid(String),
    /// A digest or a signature did not match: the bytes are not the ones
    /// that were vouched for.
    #[error("{0}")]
    Integrity(String),
    /// The extra cannot be used by this build of Concat on this machine.
    #[error("{0}")]
    Incompatible(String),
    /// What it needs is not installed, or would be broken by this.
    #[error("{0}")]
    Dependency(String),
    /// The person cancelled.
    #[error("cancelled")]
    Cancelled,
    /// A file could not be read or written.
    #[error("{path}: {message}")]
    Io {
        /// The file or folder.
        path: std::path::PathBuf,
        /// The system's reason.
        message: String,
    },
}

impl Error {
    /// An I/O failure on `path`.
    pub fn io(path: &std::path::Path, error: impl std::fmt::Display) -> Error {
        Error::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    }
}

/// This build's platform as a manifest names it: `<os>-<arch>`, with Rust's
/// own names (`windows-x86_64`, `macos-aarch64`, `linux-x86_64`,
/// `android-aarch64`).
pub fn current_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The bytes a hex string spells, or None when it is not hex of an even
/// length.
pub fn unhex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    // Hex digits only: `from_str_radix` alone would also take a sign, so
    // "+a" would read as "0a" and two spellings would name one key.
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

/// Whether `text` is a SHA-256 as the formats write one: 64 lowercase hex
/// digits.
pub fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A human size: "4.2 MB", "870 KB", "12 bytes". Decimal units, as the
/// model rows write them.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [(&str, f64); 3] = [("GB", 1e9), ("MB", 1e6), ("KB", 1e3)];
    for (unit, scale) in UNITS {
        let value = bytes as f64 / scale;
        if value >= 1.0 {
            return if value >= 100.0 {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            };
        }
    }
    format!("{bytes} bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_refuses_what_is_not_hex() {
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        assert_eq!(unhex("000fff"), Some(vec![0, 15, 255]));
        assert_eq!(unhex("0g"), None);
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("+a"), None, "a sign is not a digit");
        assert_eq!(unhex("-0"), None);
        assert_eq!(unhex(" 0a\n"), Some(vec![10]), "space around is trimmed");
        assert!(is_sha256(&"a".repeat(64)));
        assert!(
            !is_sha256(&"A".repeat(64)),
            "uppercase is not how we write one"
        );
        assert!(!is_sha256("abc"));
    }

    #[test]
    fn sizes_read_as_a_person_would_write_them() {
        assert_eq!(human_size(12), "12 bytes");
        assert_eq!(human_size(870_000), "870 KB");
        assert_eq!(human_size(4_200_000), "4.2 MB");
        assert_eq!(human_size(1_500_000_000), "1.5 GB");
    }

    #[test]
    fn the_platform_names_os_and_arch() {
        let platform = current_platform();
        assert!(platform.starts_with(std::env::consts::OS));
        assert!(platform.ends_with(std::env::consts::ARCH));
    }
}
