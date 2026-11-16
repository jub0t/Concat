// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! What is installed: `installed.json` in the extras folder, and the
//! folders it names.
//!
//! ```text
//! <extras>/
//!     installed.json        this registry
//!     packages/<id>/        an installed extra: its manifest and its files
//!     downloads/            archives on their way in (.part, then whole)
//!     staging/              archives being unpacked and checked
//!     trash/                folders on their way out
//!     cache/                each repository's last good index
//! ```
//!
//! The registry is the one source of truth for "installed": a folder under
//! `packages/` the registry does not name is not an installed extra, and
//! nothing reads it. It is rewritten whole, through a temporary file and a
//! rename, so a crash leaves either the old registry or the new one.
//!
//! Each part of Concat that takes content asks [`Registry::roots`] for the
//! folders of one [`Kind`]; an extra that is switched off contributes none.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::manifest::{Category, Kind, Manifest, Provide, Requirement};
use crate::resolve::Installed;

/// The registry file's name.
pub const FILE: &str = "installed.json";

/// The registry's schema.
pub const SCHEMA: u32 = 1;

/// Where an installed extra came from.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "from", rename_all = "kebab-case")]
pub enum Origin {
    /// A repository's signed index vouched for it.
    Repository {
        /// The repository's name.
        name: String,
    },
    /// A file the person chose. `verified` when its digest is one a
    /// repository's signed index lists; otherwise they were told it was
    /// not and installed it anyway.
    File {
        /// Whether a signed index vouched for it.
        verified: bool,
    },
}

impl Origin {
    /// Whether anything signed vouched for the extra.
    pub fn verified(&self) -> bool {
        match self {
            Origin::Repository { .. } => true,
            Origin::File { verified } => *verified,
        }
    }
}

/// One installed extra.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Record {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Its author.
    #[serde(default)]
    pub author: String,
    /// Where it is shelved.
    #[serde(default)]
    pub category: Category,
    /// When it was installed, in Unix seconds.
    #[serde(default)]
    pub installed_at: u64,
    /// What its files add up to.
    #[serde(default)]
    pub size: u64,
    /// Switched on: its content is offered.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// What it provides, and where.
    #[serde(default)]
    pub provides: Vec<Provide>,
    /// The ids it provides that a project can name: its manifest's, and
    /// every effect id its packages declared at install.
    #[serde(default)]
    pub provides_ids: Vec<String>,
    /// What it needs.
    #[serde(default)]
    pub requires: Vec<Requirement>,
    /// Where it came from.
    pub origin: Origin,
    /// The SHA-256 of the archive it was installed from.
    #[serde(default)]
    pub sha256: String,
}

fn yes() -> bool {
    true
}

impl Record {
    /// A record for a just-unpacked extra.
    pub fn new(
        manifest: &Manifest,
        origin: Origin,
        sha256: String,
        provides_ids: Vec<String>,
    ) -> Record {
        let mut ids = manifest.extra.provides_ids.clone();
        for id in provides_ids {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Record {
            id: manifest.extra.id.clone(),
            name: manifest.extra.name.clone(),
            version: manifest.extra.version.clone(),
            author: manifest.extra.author.clone(),
            category: manifest.extra.category,
            installed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0),
            size: manifest.total_size(),
            enabled: true,
            provides: manifest.provides.clone(),
            provides_ids: ids,
            requires: manifest.requires.clone(),
            origin,
            sha256,
        }
    }

    /// The version, parsed.
    pub fn version(&self) -> semver::Version {
        semver::Version::parse(&self.version).unwrap_or_else(|_| semver::Version::new(0, 0, 0))
    }
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    schema: u32,
    #[serde(default)]
    extras: Vec<Record>,
}

/// The install database, and the folder it describes.
#[derive(Clone, Debug)]
pub struct Registry {
    root: PathBuf,
    records: Vec<Record>,
}

impl Registry {
    /// The registry in `root`, the extras folder. A folder with no
    /// registry is one with nothing installed. A registry that does not
    /// parse is reported and read as empty - nothing is offered, and
    /// nothing is deleted: the folders stay where they are for a reinstall
    /// to find again.
    pub fn open(root: &Path) -> Registry {
        let path = root.join(FILE);
        let records = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                Ok(file) if file.schema <= SCHEMA => file.extras,
                Ok(file) => {
                    log::warn!(
                        "extras: {} is schema {}, newer than this Concat reads; no extras are offered",
                        path.display(),
                        file.schema
                    );
                    Vec::new()
                }
                Err(error) => {
                    log::warn!(
                        "extras: {} does not parse: {error}; no extras are offered",
                        path.display()
                    );
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        // Every id and root becomes a path under the folder: one that would
        // lead out of it - a registry edited by hand, or damaged - is not
        // believed, and nothing is read from or deleted at it.
        let records = records
            .into_iter()
            .filter(|record| {
                let fine = crate::manifest::check_id(&record.id).is_ok()
                    && record
                        .provides
                        .iter()
                        .all(|provide| crate::manifest::safe_path(&provide.root).is_ok());
                if !fine {
                    log::warn!(
                        "extras: {} names an extra `{}` with an id or a folder that is not allowed; it is ignored",
                        path.display(),
                        record.id
                    );
                }
                fine
            })
            .collect();
        Registry {
            root: root.to_path_buf(),
            records,
        }
    }

    /// The extras folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where an installed extra's folder is.
    pub fn package_dir(&self, id: &str) -> PathBuf {
        package_dir(&self.root, id)
    }

    /// Every installed extra, in the order they were installed.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// The installed extra with this id.
    pub fn get(&self, id: &str) -> Option<&Record> {
        self.records.iter().find(|record| record.id == id)
    }

    /// The installed extra that provides `provided`, an id a project names.
    pub fn provider_of(&self, provided: &str) -> Option<&Record> {
        self.records
            .iter()
            .find(|record| record.provides_ids.iter().any(|id| id == provided))
    }

    /// The folders every enabled extra hands to `kind`, in install order,
    /// leaving out any that is not on disk.
    pub fn roots(&self, kind: Kind) -> Vec<PathBuf> {
        self.records
            .iter()
            .filter(|record| record.enabled)
            .flat_map(|record| {
                let dir = self.package_dir(&record.id);
                record
                    .provides
                    .iter()
                    .filter(move |provide| provide.kind == kind)
                    .map(move |provide| crate::archive::join(&dir, &provide.root))
            })
            .filter(|root| root.is_dir())
            .collect()
    }

    /// What a plan needs to know about what is installed.
    pub fn installed(&self) -> BTreeMap<String, Installed> {
        self.records
            .iter()
            .map(|record| {
                (
                    record.id.clone(),
                    Installed {
                        version: record.version(),
                        requires: record.requires.clone(),
                        enabled: record.enabled,
                    },
                )
            })
            .collect()
    }

    /// Records `record`, replacing one with its id.
    pub fn put(&mut self, record: Record) {
        match self.records.iter_mut().find(|held| held.id == record.id) {
            Some(held) => *held = record,
            None => self.records.push(record),
        }
    }

    /// Forgets `id`; whether there was one.
    pub fn forget(&mut self, id: &str) -> bool {
        let before = self.records.len();
        self.records.retain(|record| record.id != id);
        self.records.len() != before
    }

    /// Switches `id` on or off. Not saved until [`Registry::save`].
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), Error> {
        let record = self
            .records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| Error::Invalid(format!("{id} is not installed")))?;
        record.enabled = enabled;
        Ok(())
    }

    /// Writes the registry: a temporary file, then a rename over the old.
    pub fn save(&self) -> Result<(), Error> {
        std::fs::create_dir_all(&self.root).map_err(|error| Error::io(&self.root, error))?;
        let path = self.root.join(FILE);
        let temporary = self.root.join(format!("{FILE}.new"));
        let bytes = serde_json::to_vec_pretty(&File {
            schema: SCHEMA,
            extras: self.records.clone(),
        })
        .expect("a registry always serialises");
        {
            use std::io::Write as _;
            let mut file =
                std::fs::File::create(&temporary).map_err(|error| Error::io(&temporary, error))?;
            file.write_all(&bytes)
                .map_err(|error| Error::io(&temporary, error))?;
            file.sync_all()
                .map_err(|error| Error::io(&temporary, error))?;
        }
        std::fs::rename(&temporary, &path).map_err(|error| Error::io(&path, error))
    }

    /// A number that changes whenever what is offered could have: the
    /// registry file's size and time. Part of the stamp the window's
    /// package watch compares.
    pub fn stamp(root: &Path) -> u64 {
        std::fs::metadata(root.join(FILE))
            .map(|meta| {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |elapsed| elapsed.as_nanos() as u64);
                meta.len().wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ modified
            })
            .unwrap_or(0)
    }
}

/// Where the extra `id` is installed under the extras folder `root`.
pub fn package_dir(root: &Path, id: &str) -> PathBuf {
    root.join("packages").join(id)
}

/// The roots every enabled extra in `root` hands to `kind`: one call for a
/// part of Concat that only wants the folders.
pub fn roots(root: &Path, kind: Kind) -> Vec<PathBuf> {
    Registry::open(root).roots(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, kind: Kind) -> Record {
        let mut manifest = crate::manifest::tests::manifest(id);
        manifest.provides[0].kind = kind;
        Record::new(
            &manifest,
            Origin::Repository {
                name: "test".to_owned(),
            },
            String::new(),
            vec![format!("{id}.x")],
        )
    }

    #[test]
    fn a_registry_round_trips_and_offers_only_enabled_roots() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = Registry::open(temp.path());
        assert!(registry.records().is_empty());
        registry.put(record("a.one", Kind::Effects));
        registry.put(record("a.two", Kind::Fonts));
        registry.save().unwrap();
        for id in ["a.one", "a.two"] {
            std::fs::create_dir_all(registry.package_dir(id).join("effects")).unwrap();
        }
        let registry = Registry::open(temp.path());
        assert_eq!(registry.records().len(), 2);
        assert_eq!(
            registry.roots(Kind::Effects),
            vec![registry.package_dir("a.one").join("effects")]
        );
        assert_eq!(
            registry.roots(Kind::Fonts),
            vec![registry.package_dir("a.two").join("effects")]
        );
        assert_eq!(
            registry.provider_of("a.one.x").map(|r| r.id.as_str()),
            Some("a.one")
        );

        let mut registry = registry;
        registry.set_enabled("a.one", false).unwrap();
        registry.save().unwrap();
        assert!(roots(temp.path(), Kind::Effects).is_empty(), "switched off");
        assert!(!registry.installed()["a.one"].enabled);
        assert!(registry.set_enabled("a.gone", true).is_err());
        assert!(registry.forget("a.two"));
        assert!(!registry.forget("a.two"));
    }

    #[test]
    fn a_root_that_is_not_on_disk_is_not_offered() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = Registry::open(temp.path());
        registry.put(record("a.one", Kind::Effects));
        assert!(registry.roots(Kind::Effects).is_empty());
    }

    #[test]
    fn a_broken_registry_reads_as_empty_and_deletes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join(FILE), b"{ not json").unwrap();
        std::fs::create_dir_all(temp.path().join("packages/a.one")).unwrap();
        assert!(Registry::open(temp.path()).records().is_empty());
        assert!(temp.path().join("packages/a.one").is_dir());
        std::fs::write(temp.path().join(FILE), br#"{"schema": 99, "extras": []}"#).unwrap();
        assert!(Registry::open(temp.path()).records().is_empty());
    }

    /// A record whose id or root would lead out of the folder - written by
    /// hand, or damaged - is ignored, so nothing is read from it, offered
    /// or deleted there; the others stand.
    #[test]
    fn a_record_that_leads_out_of_the_folder_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let mut registry = Registry::open(temp.path());
        registry.put(record("a.one", Kind::Effects));
        let mut escaping_id = record("a.two", Kind::Effects);
        escaping_id.id = "../../outside".to_owned();
        let mut escaping_root = record("a.three", Kind::Effects);
        escaping_root.provides[0].root = "../../../outside".to_owned();
        registry.records.push(escaping_id);
        registry.records.push(escaping_root);
        registry.save().unwrap();
        let read = Registry::open(temp.path());
        let ids: Vec<&str> = read.records().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a.one"]);
    }

    #[test]
    fn the_stamp_moves_when_the_registry_is_written() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(Registry::stamp(temp.path()), 0);
        let mut registry = Registry::open(temp.path());
        registry.put(record("a.one", Kind::Effects));
        registry.save().unwrap();
        let first = Registry::stamp(temp.path());
        assert_ne!(first, 0);
        registry.put(record("a.two", Kind::Effects));
        registry.save().unwrap();
        assert_ne!(Registry::stamp(temp.path()), first);
    }

    #[test]
    fn provided_ids_merge_the_manifests_and_the_packages() {
        let mut manifest = crate::manifest::tests::manifest("a.one");
        manifest.extra.provides_ids = vec!["x".to_owned(), "y".to_owned()];
        let record = Record::new(
            &manifest,
            Origin::File { verified: false },
            String::new(),
            vec!["y".to_owned(), "z".to_owned()],
        );
        assert_eq!(record.provides_ids, ["x", "y", "z"]);
        assert!(!record.origin.verified());
    }
}
