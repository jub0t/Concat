// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! A repository's index: every extra it offers, at one version each, with
//! where to fetch it and what its archive must hash to.
//!
//! ```json
//! {
//!   "schema": 1,
//!   "repository": "Concat Extras",
//!   "revision": "2026-11-14.1",
//!   "extras": [
//!     {
//!       "id": "studio.cinematic-transitions", "name": "Cinematic Transitions",
//!       "version": "1.0.0", "summary": "…", "author": "A Studio",
//!       "licence": "CC-BY-4.0", "category": "transitions",
//!       "provides": ["effects"], "provides_ids": ["studio.whip-pan"],
//!       "requires": [], "installed_size": 182000,
//!       "download": { "url": "cinematic-transitions-1.0.0.concatpkg",
//!                     "size": 61000, "sha256": "…" }
//!     }
//!   ]
//! }
//! ```
//!
//! The index is published beside `index.json.sig`: the Ed25519 signature of
//! its exact bytes, in hex, by the key the repository is configured with.
//! [`Index::verify_and_parse`] refuses an index whose signature does not
//! check out before a single field of it is believed - the index is what
//! vouches for every archive, so an unsigned one would let whoever can get
//! between Concat and the repository substitute anything.
//!
//! A download URL is either `https://`, or relative to the index's own
//! address - which is how a repository on a local disk (`file:`) works the
//! same way as one on a server.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::manifest::{self, Category, Kind, Requirement};

/// The index schema this build reads.
pub const SCHEMA: u32 = 1;

/// The largest an index may be.
pub const MAX_INDEX_SIZE: usize = 16 * 1024 * 1024;

/// The whole index.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Index {
    /// [`SCHEMA`].
    pub schema: u32,
    /// The repository's name, as the sheet shows it.
    #[serde(default)]
    pub repository: String,
    /// Changes whenever the index does; shown in the sheet's footer.
    #[serde(default)]
    pub revision: String,
    /// A number the repository only ever raises: an index with a lower one
    /// than an index already seen from the same repository is an old one
    /// replayed - a signature does not expire on its own - and is refused.
    /// 0 for an index that does not take part.
    #[serde(default)]
    pub serial: u64,
    /// When the index stops being believed, in Unix seconds; 0 for never.
    /// Signed with the rest, so an index that is withheld and an old one
    /// served in its place both stop working on a known day.
    #[serde(default)]
    pub expires: u64,
    /// What it offers.
    #[serde(default)]
    pub extras: Vec<Entry>,
}

/// One extra the repository offers.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Entry {
    /// As its manifest says.
    pub id: String,
    /// As its manifest says.
    pub name: String,
    /// As its manifest says.
    pub version: String,
    /// As its manifest says.
    #[serde(default)]
    pub summary: String,
    /// As its manifest says.
    #[serde(default)]
    pub description: String,
    /// As its manifest says.
    pub author: String,
    /// As its manifest says.
    pub licence: String,
    /// As its manifest says.
    #[serde(default)]
    pub homepage: String,
    /// As its manifest says.
    #[serde(default)]
    pub category: Category,
    /// As its manifest says.
    #[serde(default)]
    pub min_app: String,
    /// As its manifest says.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// As its manifest says.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// The kinds its manifest provides.
    #[serde(default)]
    pub provides: Vec<Kind>,
    /// The ids a project can name that it provides: what lets a project
    /// that uses a missing effect say which extra brings it.
    #[serde(default)]
    pub provides_ids: Vec<String>,
    /// As its manifest says.
    #[serde(default)]
    pub requires: Vec<Requirement>,
    /// What it unpacks to.
    #[serde(default)]
    pub installed_size: u64,
    /// Shown on the sheet's Discover page.
    #[serde(default)]
    pub featured: bool,
    /// What changed in this version, for the detail page.
    #[serde(default)]
    pub changelog: String,
    /// The archive.
    pub download: Download,
}

/// Where an archive is and what it must be.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Download {
    /// `https://…`, or relative to the index.
    pub url: String,
    /// The archive's size in bytes.
    pub size: u64,
    /// The archive's SHA-256, lowercase hex.
    pub sha256: String,
}

impl Entry {
    /// The version, parsed. Validation has already made sure it parses.
    pub fn version(&self) -> semver::Version {
        semver::Version::parse(&self.version).unwrap_or_else(|_| semver::Version::new(0, 0, 0))
    }

    /// Whether this build of Concat on this machine can use it; the reason
    /// when not.
    pub fn compatible(&self, app_version: &str, platform: &str) -> Result<(), String> {
        manifest::compatible(&self.min_app, &self.platforms, app_version, platform)
    }

    /// Whether a search finds it: every word of `query`, in any of its
    /// name, summary, author, keywords or id, case folded.
    pub fn matches(&self, query: &str) -> bool {
        let haystack = format!(
            "{} {} {} {} {} {}",
            self.name,
            self.summary,
            self.author,
            self.keywords.join(" "),
            self.id,
            self.category.key()
        )
        .to_lowercase();
        query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }

    fn validate(&self) -> Result<(), Error> {
        manifest::check_id(&self.id)?;
        let id = &self.id;
        let invalid = |message: String| Err(Error::Invalid(format!("the index's {id}: {message}")));
        if semver::Version::parse(&self.version).is_err() {
            return invalid(format!("`{}` is not a version", self.version));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return invalid("its name is empty or too long".to_owned());
        }
        if !crate::is_sha256(&self.download.sha256) {
            return invalid("its download has no SHA-256".to_owned());
        }
        if self.download.size == 0 || self.download.size > manifest::MAX_TOTAL_SIZE {
            return invalid("its download's size is not plausible".to_owned());
        }
        let url = &self.download.url;
        if url.contains("://") {
            if !url.starts_with("https://") {
                return invalid(format!("{url} is not an https:// address"));
            }
        } else {
            manifest::safe_path(url)
                .map_err(|why| Error::Invalid(format!("the index's {id}: its download {why}")))?;
        }
        for requirement in &self.requires {
            manifest::check_id(&requirement.id)?;
            requirement.range()?;
        }
        Ok(())
    }
}

impl Index {
    /// Checks `signature` (hex) over `bytes` with `public_key` (hex), and
    /// only then parses them. An index with no key to check it by is
    /// refused: there is no unsigned repository.
    pub fn verify_and_parse(
        bytes: &[u8],
        signature: &str,
        public_key: &str,
    ) -> Result<Index, Error> {
        Index::verify_and_parse_at(bytes, signature, public_key, unix_now())
    }

    /// [`Index::verify_and_parse`] as of `now`, in Unix seconds: the
    /// signature, then the index, then that it has not expired.
    pub fn verify_and_parse_at(
        bytes: &[u8],
        signature: &str,
        public_key: &str,
        now: u64,
    ) -> Result<Index, Error> {
        verify(bytes, signature, public_key)?;
        let index = Index::parse(bytes)?;
        index.check_current(now)?;
        Ok(index)
    }

    /// Whether the index is still to be believed at `now`.
    pub fn check_current(&self, now: u64) -> Result<(), Error> {
        if self.expires != 0 && now >= self.expires {
            return Err(Error::Integrity(format!(
                "the repository's index expired {} days ago; the repository has to publish a newly signed one",
                (now - self.expires) / 86_400
            )));
        }
        Ok(())
    }

    /// Whether this index may replace one already seen with `serial`: not
    /// an older one served again.
    pub fn check_not_older_than(&self, serial: u64) -> Result<(), Error> {
        if self.serial != 0 && serial != 0 && self.serial < serial {
            return Err(Error::Integrity(format!(
                "the repository served index {} after index {serial} had been seen: an old index served again is refused",
                self.serial
            )));
        }
        Ok(())
    }

    /// Parses and validates an index whose signature has been checked.
    pub fn parse(bytes: &[u8]) -> Result<Index, Error> {
        if bytes.len() > MAX_INDEX_SIZE {
            return Err(Error::Invalid(
                "the repository's index is too large".to_owned(),
            ));
        }
        let index: Index = serde_json::from_slice(bytes).map_err(|error| {
            Error::Invalid(format!("the repository's index does not parse: {error}"))
        })?;
        if index.schema == 0 || index.schema > SCHEMA {
            return Err(Error::Incompatible(format!(
                "the repository's index is schema {}; this Concat reads {SCHEMA} - a newer Concat is needed",
                index.schema
            )));
        }
        let mut ids = HashSet::new();
        for entry in &index.extras {
            entry.validate()?;
            if !ids.insert(entry.id.as_str()) {
                return Err(Error::Invalid(format!(
                    "the repository's index lists {} twice",
                    entry.id
                )));
            }
        }
        Ok(index)
    }

    /// The entry with this id.
    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.extras.iter().find(|entry| entry.id == id)
    }

    /// The entry that provides `provided` - an effect id a project names.
    pub fn provider_of(&self, provided: &str) -> Option<&Entry> {
        self.extras
            .iter()
            .find(|entry| entry.provides_ids.iter().any(|id| id == provided))
    }
}

/// Now, in Unix seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Resolves an index entry's download URL against the index's own.
/// `https://` URLs are taken as they are; a relative one replaces the last
/// part of `index_url`.
pub fn resolve_url(index_url: &str, url: &str) -> String {
    if url.contains("://") {
        return url.to_owned();
    }
    match index_url.rsplit_once('/') {
        Some((base, _)) => format!("{base}/{url}"),
        None => url.to_owned(),
    }
}

/// Checks an Ed25519 signature (hex) over `bytes` against a public key
/// (hex, 32 bytes).
pub fn verify(bytes: &[u8], signature: &str, public_key: &str) -> Result<(), Error> {
    let key = crate::unhex(public_key)
        .filter(|key| key.len() == 32)
        .ok_or_else(|| {
            Error::Integrity(
                "this repository has no valid public key configured, so its index cannot be trusted"
                    .to_owned(),
            )
        })?;
    let signature = crate::unhex(signature)
        .filter(|signature| signature.len() == 64)
        .ok_or_else(|| {
            Error::Integrity("the repository's index signature is not a signature".to_owned())
        })?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
        .verify(bytes, &signature)
        .map_err(|_| {
            Error::Integrity(
                "the repository's index is not signed by the repository's key: it may have been altered on the way"
                    .to_owned(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack;

    pub(crate) fn entry(id: &str, version: &str) -> Entry {
        Entry {
            id: id.to_owned(),
            name: "A Pack".to_owned(),
            version: version.to_owned(),
            summary: "Looks".to_owned(),
            description: String::new(),
            author: "Someone".to_owned(),
            licence: "CC0-1.0".to_owned(),
            homepage: String::new(),
            category: Category::Effects,
            min_app: String::new(),
            platforms: Vec::new(),
            keywords: vec!["film".to_owned()],
            provides: vec![Kind::Effects],
            provides_ids: vec![format!("{id}.look")],
            requires: Vec::new(),
            installed_size: 10,
            featured: false,
            changelog: String::new(),
            download: Download {
                url: format!("{id}.concatpkg"),
                size: 10,
                sha256: crate::manifest::tests::SHA.to_owned(),
            },
        }
    }

    fn index(entries: Vec<Entry>) -> Vec<u8> {
        serde_json::to_vec(&Index {
            schema: SCHEMA,
            repository: "Test".to_owned(),
            revision: "1".to_owned(),
            serial: 0,
            expires: 0,
            extras: entries,
        })
        .unwrap()
    }

    #[test]
    fn a_signed_index_is_believed_and_an_altered_one_is_not() {
        let (secret, public) = pack::keygen().expect("a key");
        let bytes = index(vec![entry("a.one", "1.0.0")]);
        let signature = pack::sign(&secret, &bytes).expect("signs");
        let parsed = Index::verify_and_parse(&bytes, &signature, &public).expect("verifies");
        assert_eq!(parsed.extras.len(), 1);

        let mut altered = bytes.clone();
        let at = altered.iter().position(|&b| b == b'1').unwrap();
        altered[at] = b'2';
        let error = Index::verify_and_parse(&altered, &signature, &public).expect_err("altered");
        assert!(matches!(error, Error::Integrity(_)), "{error}");

        let (_, other) = pack::keygen().unwrap();
        assert!(
            Index::verify_and_parse(&bytes, &signature, &other).is_err(),
            "another key"
        );
        assert!(
            Index::verify_and_parse(&bytes, &signature, "").is_err(),
            "no key"
        );
        assert!(
            Index::verify_and_parse(&bytes, "00", &public).is_err(),
            "no signature"
        );
    }

    /// An index past its expiry is refused even though its signature is
    /// good; one before it, or one that never expires, is believed.
    #[test]
    fn an_expired_index_is_refused() {
        let (secret, public) = pack::keygen().unwrap();
        let day = 86_400;
        let now = 1_800_000_000;
        let signed = |expires: u64| {
            let bytes = pack::write_index_with("T", "1", 7, expires, vec![entry("a.one", "1.0.0")])
                .unwrap();
            let signature = pack::sign(&secret, &bytes).unwrap();
            (bytes, signature)
        };
        let (bytes, signature) = signed(now - 3 * day);
        let error = Index::verify_and_parse_at(&bytes, &signature, &public, now).unwrap_err();
        assert!(matches!(error, Error::Integrity(_)), "{error}");
        assert!(error.to_string().contains("expired 3 days ago"), "{error}");
        let (bytes, signature) = signed(now + day);
        Index::verify_and_parse_at(&bytes, &signature, &public, now).expect("still current");
        let (bytes, signature) = signed(0);
        Index::verify_and_parse_at(&bytes, &signature, &public, now).expect("never expires");
    }

    /// A lower serial than one already seen is an old index replayed.
    #[test]
    fn an_older_serial_is_refused() {
        let parsed = |serial: u64| {
            Index::parse(&pack::write_index_with("T", "1", serial, 0, Vec::new()).unwrap()).unwrap()
        };
        assert!(parsed(5).check_not_older_than(6).is_err(), "replayed");
        parsed(6)
            .check_not_older_than(6)
            .expect("the same index again");
        parsed(7).check_not_older_than(6).expect("a newer one");
        parsed(0)
            .check_not_older_than(6)
            .expect("an index without serials takes no part");
        parsed(5).check_not_older_than(0).expect("nothing seen yet");
    }

    /// Every malformed or misdirected signature is refused before the
    /// index is read: wrong length, not hex, padded, another index's,
    /// a key that is not a key, and an index changed by one byte.
    #[test]
    fn malformed_and_misdirected_signatures_are_refused() {
        let (secret, public) = pack::keygen().unwrap();
        let bytes = index(vec![entry("a.one", "1.0.0")]);
        let good = pack::sign(&secret, &bytes).unwrap();
        let other = pack::sign(&secret, b"another index").unwrap();
        let refused = |signature: &str, key: &str, bytes: &[u8]| {
            assert!(
                matches!(
                    Index::verify_and_parse(bytes, signature, key),
                    Err(Error::Integrity(_))
                ),
                "accepted {signature:?} by {key:?}"
            );
        };
        refused(&good[..126], &public, &bytes);
        refused(&format!("{good}00"), &public, &bytes);
        refused(
            &good.replace(|c: char| c.is_ascii_digit(), "z"),
            &public,
            &bytes,
        );
        refused(&format!("{} {}", &good[..64], &good[64..]), &public, &bytes);
        refused(&other, &public, &bytes);
        refused(&good, &public[..62], &bytes);
        refused(&good, &"0".repeat(64), &bytes);
        refused(&good, "not hex at all", &bytes);
        let mut flipped = bytes.clone();
        flipped[10] ^= 1;
        refused(&good, &public, &flipped);
        // Whitespace around a signature file's contents is not a forgery.
        Index::verify_and_parse(&bytes, &format!("{good}\n"), &public).expect("a trailing newline");
        // Signed, but not an index: refused as invalid, not believed.
        let junk = b"{\"schema\": 1, \"extras\": 7}";
        let signature = pack::sign(&secret, junk).unwrap();
        assert!(matches!(
            Index::verify_and_parse(junk, &signature, &public),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn an_index_is_checked_entry_by_entry() {
        Index::parse(&index(vec![
            entry("a.one", "1.0.0"),
            entry("a.two", "2.0.0"),
        ]))
        .expect("fine");
        let twice = Index::parse(&index(vec![
            entry("a.one", "1.0.0"),
            entry("a.one", "1.1.0"),
        ]));
        assert!(twice.unwrap_err().to_string().contains("twice"));

        let mut plain = entry("a.one", "1.0.0");
        plain.download.url = "http://example.com/a.concatpkg".to_owned();
        assert!(
            Index::parse(&index(vec![plain]))
                .unwrap_err()
                .to_string()
                .contains("https")
        );
        let mut escape = entry("a.one", "1.0.0");
        escape.download.url = "../../secrets".to_owned();
        assert!(Index::parse(&index(vec![escape])).is_err());
        let mut bad = entry("a.one", "one");
        bad.version = "one".to_owned();
        assert!(Index::parse(&index(vec![bad])).is_err());
        let mut unsized_ = entry("a.one", "1.0.0");
        unsized_.download.size = 0;
        assert!(Index::parse(&index(vec![unsized_])).is_err());

        let newer = br#"{"schema": 2, "extras": []}"#;
        assert!(matches!(Index::parse(newer), Err(Error::Incompatible(_))));
        assert!(Index::parse(b"not json").is_err());
    }

    #[test]
    fn urls_resolve_against_the_index() {
        assert_eq!(
            resolve_url("https://example.com/extras/v1/index.json", "pack.concatpkg"),
            "https://example.com/extras/v1/pack.concatpkg"
        );
        assert_eq!(
            resolve_url("file:///D:/repo/index.json", "packs/a.concatpkg"),
            "file:///D:/repo/packs/a.concatpkg"
        );
        assert_eq!(
            resolve_url(
                "https://example.com/index.json",
                "https://cdn.example.com/a.concatpkg"
            ),
            "https://cdn.example.com/a.concatpkg"
        );
    }

    #[test]
    fn search_and_providers() {
        let index = Index::parse(&index(vec![entry("a.one", "1.0.0")])).unwrap();
        let one = index.get("a.one").unwrap();
        assert!(one.matches("FILM pack"));
        assert!(one.matches("someone"));
        assert!(!one.matches("film noise"));
        assert_eq!(
            index.provider_of("a.one.look").map(|e| e.id.as_str()),
            Some("a.one")
        );
        assert!(index.provider_of("nothing").is_none());
        assert!(one.compatible("0.2.11", "windows-x86_64").is_ok());
    }
}
