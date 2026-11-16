// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! Installing an archive, removing an installed extra, and switching one
//! on and off.
//!
//! An install is a sequence in which nothing is marked installed until the
//! whole of it has worked:
//!
//! 1. the archive's size and SHA-256 are checked against what the signed
//!    index vouched for, when one did;
//! 2. its manifest is read and must be the extra and the version expected,
//!    one this Concat can use, never older than what is installed (unless
//!    asked), and needing only extras that are installed and switched on;
//! 3. it is unpacked into `staging/`, every member checked ([`crate::archive`]);
//! 4. the caller's vet looks at what was unpacked - the host has every
//!    effect package load and every font parse - and names what it
//!    provides;
//! 5. a version already installed is moved into `trash/`, the staging
//!    folder renamed into `packages/<id>/`, and the registry written; a
//!    failure at either step puts the old version back where it was;
//! 6. the old version is deleted. A file something still holds open - a
//!    font Windows has mapped - is left in `trash/` for [`sweep`].
//!
//! Step 5, a removal and a switch each run under the folder's lock
//! ([`lock`]), and step 5 checks the dependencies again under it: an
//! install runs on a worker while the person can switch or remove other
//! extras, and two Concats can share the folder, so the registry is only
//! ever read, changed and written by one of them at a time.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::Error;
use crate::archive;
use crate::manifest::Manifest;
use crate::registry::{Origin, Record, Registry, package_dir};
use crate::resolve;

/// What the signed index says the archive is.
#[derive(Clone, Copy, Debug)]
pub struct Expected<'a> {
    /// The extra's id.
    pub id: &'a str,
    /// Its version.
    pub version: &'a str,
    /// The archive's size in bytes.
    pub size: u64,
    /// The archive's SHA-256.
    pub sha256: &'a str,
}

/// One install.
#[derive(Clone, Debug)]
pub struct Request<'a> {
    /// The archive, whole, on disk.
    pub archive: &'a Path,
    /// What a signed index vouched it is; None for a file the person chose
    /// that no index lists.
    pub expected: Option<Expected<'a>>,
    /// Recorded with it.
    pub origin: Origin,
    /// This Concat's version.
    pub app_version: &'a str,
    /// This machine's platform, as [`crate::current_platform`] names it.
    pub platform: &'a str,
    /// Install over a newer version. Never set by a plan.
    pub allow_downgrade: bool,
}

/// Where an install has got to, for a progress line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Hashing the archive.
    Verifying,
    /// Unpacking and hashing every file.
    Unpacking,
    /// The caller's vet.
    Checking,
    /// Moving it into place.
    Finishing,
}

/// The vet: given the manifest and the unpacked folder, the ids of what it
/// provides (effect ids, say), or why it must not be installed.
pub type Vet<'a> = dyn FnMut(&Manifest, &Path) -> Result<Vec<String>, String> + 'a;

fn nonce() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos:x}", std::process::id())
}

/// The folder an install unpacks into before it is checked.
fn staging_dir(root: &Path, id: &str) -> PathBuf {
    root.join("staging").join(format!("{id}-{}", nonce()))
}

/// The folder an installed version is moved to on its way out.
fn trash_dir(root: &Path, id: &str) -> PathBuf {
    root.join("trash").join(format!("{id}-{}", nonce()))
}

/// The file whose lock guards the registry and `packages/`.
pub const LOCK_FILE: &str = "extras.lock";

/// The file a running Concat holds a shared lock on, so a Concat starting
/// beside it knows not to sweep what it is in the middle of.
pub const PRESENCE_FILE: &str = "running.lock";

/// The folder's lock, held: released when dropped.
#[derive(Debug)]
pub struct FolderLock {
    _file: std::fs::File,
}

/// Takes the extras folder's lock, waiting for whoever holds it - another
/// thread or another Concat. It is held only around reading, changing and
/// writing the registry and the renames into and out of `packages/`, which
/// take a moment; never across a download or an unpack.
pub fn lock(root: &Path) -> Result<FolderLock, Error> {
    let file = open_lock_file(&root.join(LOCK_FILE))?;
    file.lock()
        .map_err(|error| Error::io(&root.join(LOCK_FILE), error))?;
    Ok(FolderLock { _file: file })
}

fn open_lock_file(path: &Path) -> Result<std::fs::File, Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| Error::io(parent, error))?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|error| Error::io(path, error))
}

/// A running Concat's claim on the extras folder, held for as long as it
/// runs: shared, so any number of Concats can hold one. See [`sweep`].
#[derive(Debug)]
pub struct Presence {
    _file: std::fs::File,
}

/// Claims the extras folder for this run, beside any other Concat. Waits
/// only while a starting Concat is sweeping it.
pub fn attend(root: &Path) -> Result<Presence, Error> {
    let path = root.join(PRESENCE_FILE);
    let file = open_lock_file(&path)?;
    file.lock_shared()
        .map_err(|error| Error::io(&path, error))?;
    Ok(Presence { _file: file })
}

/// Whether `manifest` can go in beside what `registry` says is installed:
/// not older than the installed version (unless `allow_downgrade`), every
/// extra it needs installed, switched on and in range, and no enabled
/// extra that needs it left out of range by it.
fn check_fits(
    registry: &Registry,
    manifest: &Manifest,
    allow_downgrade: bool,
) -> Result<(), Error> {
    let id = manifest.id();
    if let Some(held) = registry.get(id)
        && held.version() > manifest.version()
        && !allow_downgrade
    {
        return Err(Error::Incompatible(format!(
            "{} {} is installed, which is newer than {}; it was left as it is",
            held.name, held.version, manifest.extra.version
        )));
    }
    // A new version must still be one every enabled extra that needs this
    // one will take: an update is not allowed to break another extra.
    let version = manifest.version();
    for dependent in registry
        .records()
        .iter()
        .filter(|r| r.enabled && r.id != id)
    {
        for requirement in dependent.requires.iter().filter(|r| r.id == id) {
            if !requirement.range()?.matches(&version) {
                return Err(Error::Dependency(format!(
                    "{} needs {id} {}, so {} {} would break it; it was not installed",
                    dependent.name, requirement.version, manifest.extra.name, version
                )));
            }
        }
    }
    for requirement in &manifest.requires {
        let range = requirement.range()?;
        match registry.get(&requirement.id) {
            Some(held) if held.enabled && range.matches(&held.version()) => {}
            Some(held) if !held.enabled => {
                return Err(Error::Dependency(format!(
                    "{} needs {}, which is installed but switched off",
                    manifest.extra.name, held.name
                )));
            }
            Some(held) => {
                return Err(Error::Dependency(format!(
                    "{} needs {} {range}, and {} is installed",
                    manifest.extra.name, requirement.id, held.version
                )));
            }
            None => {
                return Err(Error::Dependency(format!(
                    "{} needs {}, which is not installed",
                    manifest.extra.name, requirement.id
                )));
            }
        }
    }
    Ok(())
}

/// Installs an archive into the extras folder `root`. See the module.
pub fn install(
    root: &Path,
    request: &Request<'_>,
    vet: &mut Vet<'_>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Phase, u64, u64),
) -> Result<Record, Error> {
    // 1. The archive is what the index vouched for.
    let size = std::fs::metadata(request.archive)
        .map_err(|error| Error::io(request.archive, error))?
        .len();
    progress(Phase::Verifying, 0, size);
    let digest = archive::sha256_file(request.archive, cancel)?;
    if let Some(expected) = request.expected {
        if size != expected.size {
            return Err(Error::Integrity(format!(
                "the download of {} is {size} bytes; the repository says it is {}",
                expected.id, expected.size
            )));
        }
        if digest != expected.sha256 {
            return Err(Error::Integrity(format!(
                "the download of {} is not the file the repository vouches for (its SHA-256 is {digest}); it was not installed",
                expected.id
            )));
        }
    }
    progress(Phase::Verifying, size, size);

    // 2. It is the extra expected, and one that can go in.
    let manifest = archive::read_manifest(request.archive)?;
    let id = manifest.id().to_owned();
    if let Some(expected) = request.expected
        && (expected.id != id || expected.version != manifest.extra.version)
    {
        return Err(Error::Integrity(format!(
            "the repository offered {} {}, and the archive is {id} {}",
            expected.id, expected.version, manifest.extra.version
        )));
    }
    manifest.compatible(request.app_version, request.platform)?;
    // Checked now, so nothing is unpacked that cannot go in, and again
    // under the lock as it goes in (step 5): what is installed can change
    // while this one unpacks.
    check_fits(&Registry::open(root), &manifest, request.allow_downgrade)?;

    // 3. Unpacked, every member checked.
    let staging = staging_dir(root, &id);
    let unpacked = archive::extract(request.archive, &staging, cancel, &mut |done, all| {
        progress(Phase::Unpacking, done, all)
    })?;
    if unpacked != manifest {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(Error::Integrity(
            "the archive's manifest changed while it was read".to_owned(),
        ));
    }

    // 4. The caller's vet.
    progress(Phase::Checking, 0, 1);
    let provided = match vet(&manifest, &staging) {
        Ok(provided) => provided,
        Err(why) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(Error::Invalid(format!(
                "{} was not installed: {why}",
                manifest.extra.name
            )));
        }
    };
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(Error::Cancelled);
    }

    // 5. Into place, with the old version kept until the registry says so.
    progress(Phase::Finishing, 0, 1);
    let record = Record::new(&manifest, request.origin.clone(), digest, provided);
    let target = package_dir(root, &id);
    let commit = commit(
        root,
        &staging,
        &target,
        &manifest,
        request.allow_downgrade,
        record.clone(),
    );
    if commit.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    let old = commit?;

    // 6. The old version goes.
    if let Some(old) = old
        && let Err(error) = std::fs::remove_dir_all(&old)
    {
        log::warn!(
            "extras: the previous {id} could not be deleted yet ({error}); it is swept on the next start"
        );
    }
    progress(Phase::Finishing, 1, 1);
    Ok(record)
}

/// Moves `staging` to `target` and records it, under the folder's lock and
/// once `manifest` still fits what is installed, putting back what was
/// there on any failure. The folder the old version went to, if one was
/// there.
fn commit(
    root: &Path,
    staging: &Path,
    target: &Path,
    manifest: &Manifest,
    allow_downgrade: bool,
    record: Record,
) -> Result<Option<PathBuf>, Error> {
    let id = manifest.id();
    let _lock = lock(root)?;
    // The registry is read here, under the lock, not reused from before
    // the unpack: it is the file of record, and whatever was switched,
    // removed or installed meanwhile stays as it was left - and may mean
    // this one no longer fits.
    let mut registry = Registry::open(root);
    check_fits(&registry, manifest, allow_downgrade)?;
    let old = if target.exists() {
        let trash = trash_dir(root, id);
        std::fs::create_dir_all(trash.parent().expect("trash has a parent"))
            .map_err(|error| Error::io(&trash, error))?;
        std::fs::rename(target, &trash).map_err(|error| {
            Error::io(
                target,
                format!(
                    "could not move the installed version aside ({error}); is a file in it open?"
                ),
            )
        })?;
        Some(trash)
    } else {
        None
    };
    let restore = |old: &Option<PathBuf>| {
        if let Some(old) = old {
            let _ = std::fs::remove_dir_all(target);
            let _ = std::fs::rename(old, target);
        }
    };
    if let Some(parent) = target.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        restore(&old);
        return Err(Error::io(parent, error));
    }
    if let Err(error) = std::fs::rename(staging, target) {
        restore(&old);
        return Err(Error::io(target, error));
    }
    registry.put(record);
    if let Err(error) = registry.save() {
        restore(&old);
        if old.is_none() {
            let _ = std::fs::remove_dir_all(target);
        }
        return Err(error);
    }
    Ok(old)
}

/// Removes the installed extra `id`. Refused while an enabled extra needs
/// it, unless `force`. Its folder is moved out, the registry written, and
/// then the folder deleted.
pub fn uninstall(root: &Path, id: &str, force: bool) -> Result<(), Error> {
    let _lock = lock(root)?;
    let mut registry = Registry::open(root);
    let Some(record) = registry.get(id).cloned() else {
        return Err(Error::Invalid(format!("{id} is not installed")));
    };
    let needed_by = resolve::dependents(id, &registry.installed());
    if !needed_by.is_empty() && !force {
        return Err(Error::Dependency(format!(
            "{} is needed by {}; remove or switch those off first",
            record.name,
            names(&registry, &needed_by)
        )));
    }
    let dir = registry.package_dir(id);
    let trash = trash_dir(root, id);
    let moved = if dir.exists() {
        std::fs::create_dir_all(trash.parent().expect("trash has a parent"))
            .map_err(|error| Error::io(&trash, error))?;
        std::fs::rename(&dir, &trash).map_err(|error| {
            Error::io(
                &dir,
                format!("could not remove it ({error}); is a file in it open?"),
            )
        })?;
        true
    } else {
        false
    };
    registry.forget(id);
    if let Err(error) = registry.save() {
        if moved {
            let _ = std::fs::rename(&trash, &dir);
        }
        return Err(error);
    }
    if moved && let Err(error) = std::fs::remove_dir_all(&trash) {
        log::warn!(
            "extras: {id}'s files could not all be deleted yet ({error}); they are swept on the next start"
        );
    }
    Ok(())
}

/// Switches the installed extra `id` on or off. Switching off is refused
/// while an enabled extra needs it, unless `force`; switching on is refused
/// while something it needs is missing or off.
pub fn set_enabled(root: &Path, id: &str, enabled: bool, force: bool) -> Result<(), Error> {
    let _lock = lock(root)?;
    let mut registry = Registry::open(root);
    let Some(record) = registry.get(id).cloned() else {
        return Err(Error::Invalid(format!("{id} is not installed")));
    };
    if enabled {
        for requirement in &record.requires {
            let range = requirement.range()?;
            match registry.get(&requirement.id) {
                Some(held) if held.enabled && range.matches(&held.version()) => {}
                _ => {
                    return Err(Error::Dependency(format!(
                        "{} needs {} {range} installed and switched on",
                        record.name, requirement.id
                    )));
                }
            }
        }
    } else {
        let needed_by = resolve::dependents(id, &registry.installed());
        if !needed_by.is_empty() && !force {
            return Err(Error::Dependency(format!(
                "{} is needed by {}; switch those off first",
                record.name,
                names(&registry, &needed_by)
            )));
        }
    }
    registry.set_enabled(id, enabled)?;
    registry.save()
}

fn names(registry: &Registry, ids: &[String]) -> String {
    ids.iter()
        .map(|id| {
            registry
                .get(id)
                .map_or(id.as_str(), |record| record.name.as_str())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Puts back what an update interrupted part way left out of place, so
/// what is in `packages/` is what the registry says is installed:
///
/// * killed after the installed version was moved into `trash/` and before
///   the new one moved in: `packages/<id>` is missing, and the version in
///   the trash goes back;
/// * killed after the new version moved in and before the registry was
///   written: `packages/<id>` holds a version the registry does not name,
///   and the version it does name, waiting in the trash, goes back.
///
/// The registry is written last on every install, so it is the record of
/// what finished. What was put back, by id.
pub fn recover(root: &Path) -> Vec<String> {
    let registry = Registry::open(root);
    let Ok(entries) = std::fs::read_dir(root.join("trash")) else {
        return Vec::new();
    };
    let mut recovered = Vec::new();
    for entry in entries.flatten() {
        let held = entry.path();
        let Ok(manifest) = std::fs::read_to_string(held.join(crate::manifest::MANIFEST_FILE))
            .map_err(|_| ())
            .and_then(|text| Manifest::parse(&text).map_err(|_| ()))
        else {
            continue;
        };
        let Some(record) = registry.get(manifest.id()) else {
            continue;
        };
        if record.version != manifest.extra.version {
            continue;
        }
        let target = registry.package_dir(&record.id);
        let in_place = std::fs::read_to_string(target.join(crate::manifest::MANIFEST_FILE))
            .ok()
            .and_then(|text| Manifest::parse(&text).ok())
            .map(|manifest| manifest.extra.version);
        if in_place.as_deref() == Some(record.version.as_str()) {
            continue;
        }
        if target.exists() {
            // The unfinished new version steps aside for the recorded one.
            let aside = trash_dir(root, &record.id);
            if std::fs::rename(&target, &aside).is_err() {
                log::warn!(
                    "extras: {} is not as recorded and could not be put back",
                    record.id
                );
                continue;
            }
        }
        match std::fs::rename(&held, &target) {
            Ok(()) => {
                log::warn!(
                    "extras: {} {} was put back after an update that did not finish",
                    record.id,
                    record.version
                );
                recovered.push(record.id.clone());
            }
            Err(error) => log::warn!("extras: could not put back {}: {error}", record.id),
        }
    }
    recovered
}

/// Deletes what an interrupted install or removal left behind: staging
/// folders and the trash, after [`recover`] has taken back from the trash
/// anything the registry still names. Downloads are kept, since a partial
/// one is where the next attempt picks up. Called once at start, before
/// anything reads the folder.
///
/// Only when no other Concat is running on the folder ([`attend`]): what
/// another one has in `staging/` or `trash/` may be an install it is in the
/// middle of, not one that was interrupted. The sweep then waits for a
/// start with the folder to itself.
pub fn sweep(root: &Path) {
    let presence = root.join(PRESENCE_FILE);
    let file = match open_lock_file(&presence) {
        Ok(file) => file,
        Err(error) => {
            log::warn!("extras: could not sweep: {error}");
            return;
        }
    };
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            log::info!(
                "extras: another Concat is using {}; what it has in progress is left alone",
                root.display()
            );
            return;
        }
        Err(std::fs::TryLockError::Error(error)) => {
            log::warn!("extras: could not sweep: {error}");
            return;
        }
    }
    let Ok(_lock) = lock(root) else {
        return;
    };
    recover(root);
    for folder in ["staging", "trash"] {
        let dir = root.join(folder);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Err(error) = std::fs::remove_dir_all(entry.path()) {
                log::warn!(
                    "extras: could not sweep {}: {error}",
                    entry.path().display()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::MANIFEST_FILE;
    use crate::pack;

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// Packs an extra `id` at `version` needing `requires`, into `dir`.
    fn packed(dir: &Path, id: &str, version: &str, requires: &str) -> (PathBuf, pack::Packed) {
        let folder = dir.join(format!("{id}-{version}-src"));
        let look = folder.join("effects").join(format!("{id}.look"));
        std::fs::create_dir_all(&look).unwrap();
        std::fs::write(look.join("effect.toml"), format!("version {version}\n")).unwrap();
        std::fs::write(
            folder.join(MANIFEST_FILE),
            format!(
                "format = 1\n[extra]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\nauthor = \"A\"\nlicence = \"MIT\"\n\n[[provides]]\nkind = \"effects\"\nroot = \"effects\"\n{requires}"
            ),
        )
        .unwrap();
        let out = dir.join(format!("{id}-{version}.concatpkg"));
        let packed = pack::pack(&folder, &out).unwrap();
        (out, packed)
    }

    fn request<'a>(archive: &'a Path, expected: Option<Expected<'a>>) -> Request<'a> {
        Request {
            archive,
            expected,
            origin: Origin::Repository {
                name: "test".to_owned(),
            },
            app_version: crate::APP_VERSION,
            platform: "windows-x86_64",
            allow_downgrade: false,
        }
    }

    fn ok_vet() -> impl FnMut(&Manifest, &Path) -> Result<Vec<String>, String> {
        |manifest: &Manifest, _: &Path| Ok(vec![format!("{}.look", manifest.id())])
    }

    #[test]
    fn an_install_lands_in_place_and_is_recorded_last() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (archive, packed) = packed(temp.path(), "a.one", "1.0.0", "");
        let expected = Expected {
            id: "a.one",
            version: "1.0.0",
            size: packed.size,
            sha256: &packed.sha256,
        };
        let mut phases = Vec::new();
        let record = install(
            &root,
            &request(&archive, Some(expected)),
            &mut ok_vet(),
            &never(),
            &mut |phase, _, _| {
                if phases.last() != Some(&phase) {
                    phases.push(phase);
                }
            },
        )
        .unwrap();
        assert_eq!(
            phases,
            [
                Phase::Verifying,
                Phase::Unpacking,
                Phase::Checking,
                Phase::Finishing
            ]
        );
        assert_eq!(record.provides_ids, ["a.one.look"]);
        assert_eq!(record.sha256, packed.sha256);
        let registry = Registry::open(&root);
        assert_eq!(registry.get("a.one").unwrap().version, "1.0.0");
        let roots = registry.roots(crate::Kind::Effects);
        assert_eq!(roots.len(), 1);
        assert!(roots[0].join("a.one.look/effect.toml").is_file());
        assert!(
            std::fs::read_dir(root.join("staging"))
                .unwrap()
                .next()
                .is_none(),
            "no staging left"
        );
    }

    #[test]
    fn a_wrong_digest_size_or_identity_installs_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (archive, packed) = packed(temp.path(), "a.one", "1.0.0", "");
        let wrong = "0".repeat(64);
        for (expected, needle) in [
            (
                Expected {
                    id: "a.one",
                    version: "1.0.0",
                    size: packed.size,
                    sha256: &wrong,
                },
                "not the file",
            ),
            (
                Expected {
                    id: "a.one",
                    version: "1.0.0",
                    size: packed.size + 1,
                    sha256: &packed.sha256,
                },
                "bytes",
            ),
            (
                Expected {
                    id: "a.two",
                    version: "1.0.0",
                    size: packed.size,
                    sha256: &packed.sha256,
                },
                "the archive is a.one",
            ),
            (
                Expected {
                    id: "a.one",
                    version: "2.0.0",
                    size: packed.size,
                    sha256: &packed.sha256,
                },
                "the archive is a.one",
            ),
        ] {
            let error = install(
                &root,
                &request(&archive, Some(expected)),
                &mut ok_vet(),
                &never(),
                &mut |_, _, _| {},
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains(needle), "{needle}: {error}");
            assert!(Registry::open(&root).records().is_empty());
            assert!(!package_dir(&root, "a.one").exists());
        }
    }

    #[test]
    fn a_refused_vet_installs_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (archive, _) = packed(temp.path(), "a.one", "1.0.0", "");
        let mut vet = |_: &Manifest, _: &Path| {
            Err::<Vec<String>, String>("its shader does not compile".to_owned())
        };
        let error = install(
            &root,
            &request(&archive, None),
            &mut vet,
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("shader"), "{error}");
        assert!(Registry::open(&root).records().is_empty());
        assert!(
            std::fs::read_dir(root.join("staging"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn upgrades_replace_reinstalls_repair_and_downgrades_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (one, _) = packed(temp.path(), "a.one", "1.0.0", "");
        let (two, _) = packed(temp.path(), "a.one", "2.0.0", "");
        install(
            &root,
            &request(&one, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        install(
            &root,
            &request(&two, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        let effect = package_dir(&root, "a.one").join("effects/a.one.look/effect.toml");
        assert_eq!(std::fs::read_to_string(&effect).unwrap(), "version 2.0.0\n");
        assert_eq!(Registry::open(&root).get("a.one").unwrap().version, "2.0.0");

        // Damaged, then repaired by installing the same version again.
        std::fs::write(&effect, "garbage").unwrap();
        install(
            &root,
            &request(&two, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&effect).unwrap(), "version 2.0.0\n");

        let error = install(
            &root,
            &request(&one, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap_err();
        assert!(error.to_string().contains("newer"), "{error}");
        let mut older = request(&one, None);
        older.allow_downgrade = true;
        install(&root, &older, &mut ok_vet(), &never(), &mut |_, _, _| {}).unwrap();
        assert_eq!(Registry::open(&root).get("a.one").unwrap().version, "1.0.0");
        assert!(
            std::fs::read_dir(root.join("trash"))
                .unwrap()
                .next()
                .is_none(),
            "old versions deleted"
        );
    }

    #[test]
    fn dependencies_must_be_installed_and_switched_on() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (base, _) = packed(temp.path(), "a.base", "1.0.0", "");
        let needs = "\n[[requires]]\nid = \"a.base\"\nversion = \"^1\"\n";
        let (user, _) = packed(temp.path(), "a.user", "1.0.0", needs);
        let error = install(
            &root,
            &request(&user, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap_err();
        assert!(error.to_string().contains("not installed"), "{error}");

        install(
            &root,
            &request(&base, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        install(
            &root,
            &request(&user, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();

        // The base is needed: not removed nor switched off unless forced.
        assert!(
            uninstall(&root, "a.base", false)
                .unwrap_err()
                .to_string()
                .contains("needed by a.user")
        );
        assert!(set_enabled(&root, "a.base", false, false).is_err());
        set_enabled(&root, "a.user", false, false).unwrap();
        set_enabled(&root, "a.base", false, false).unwrap();
        // ... and the user cannot be switched back on without it.
        assert!(set_enabled(&root, "a.user", true, false).is_err());
        set_enabled(&root, "a.base", true, false).unwrap();
        set_enabled(&root, "a.user", true, false).unwrap();

        uninstall(&root, "a.user", false).unwrap();
        uninstall(&root, "a.base", false).unwrap();
        assert!(Registry::open(&root).records().is_empty());
        assert!(!package_dir(&root, "a.base").exists());
        assert!(uninstall(&root, "a.base", false).is_err(), "already gone");
    }

    #[test]
    fn an_incompatible_extra_is_refused_before_unpacking() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (archive, _) = packed(temp.path(), "a.one", "1.0.0", "");
        let mut old_app = request(&archive, None);
        let mut manifest = archive::read_manifest(&archive).unwrap();
        manifest.extra.min_app = "99.0.0".to_owned();
        let error = manifest
            .compatible(old_app.app_version, old_app.platform)
            .unwrap_err();
        assert!(error.to_string().contains("99.0.0"));
        old_app.app_version = "0.0.1";
        install(&root, &old_app, &mut ok_vet(), &never(), &mut |_, _, _| {})
            .expect("no min_app set, so any version");
    }

    #[test]
    fn a_cancelled_install_leaves_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (archive, _) = packed(temp.path(), "a.one", "1.0.0", "");
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            install(
                &root,
                &request(&archive, None),
                &mut ok_vet(),
                &cancel,
                &mut |_, _, _| {}
            ),
            Err(Error::Cancelled)
        ));
        assert!(Registry::open(&root).records().is_empty());
    }

    fn effect_text(root: &Path) -> String {
        std::fs::read_to_string(package_dir(root, "a.one").join("effects/a.one.look/effect.toml"))
            .unwrap_or_default()
    }

    /// Killed after the installed version went to the trash and before the
    /// new one moved in: the next start puts the recorded version back.
    #[test]
    fn an_update_killed_with_the_old_version_in_the_trash_is_put_back() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (one, _) = packed(temp.path(), "a.one", "1.0.0", "");
        install(
            &root,
            &request(&one, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        std::fs::create_dir_all(root.join("trash")).unwrap();
        std::fs::rename(package_dir(&root, "a.one"), root.join("trash/a.one-crash")).unwrap();
        assert!(
            Registry::open(&root).roots(crate::Kind::Effects).is_empty(),
            "broken now"
        );
        sweep(&root);
        assert_eq!(effect_text(&root), "version 1.0.0\n");
        assert_eq!(Registry::open(&root).roots(crate::Kind::Effects).len(), 1);
        assert!(
            std::fs::read_dir(root.join("trash"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    /// Killed after the new version moved in and before the registry was
    /// written: the registry still names the old version, which comes back
    /// from the trash, and the unfinished new one is cleared away.
    #[test]
    fn an_update_killed_before_the_registry_was_written_is_put_back() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (one, _) = packed(temp.path(), "a.one", "1.0.0", "");
        let (two, _) = packed(temp.path(), "a.one", "2.0.0", "");
        install(
            &root,
            &request(&one, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        // As commit leaves it at the moment of the crash.
        std::fs::create_dir_all(root.join("trash")).unwrap();
        std::fs::rename(package_dir(&root, "a.one"), root.join("trash/a.one-old")).unwrap();
        crate::archive::extract(&two, &package_dir(&root, "a.one"), &never(), &mut |_, _| {})
            .unwrap();
        assert_eq!(effect_text(&root), "version 2.0.0\n");
        assert_eq!(recover(&root), ["a.one"]);
        sweep(&root);
        assert_eq!(
            effect_text(&root),
            "version 1.0.0\n",
            "as the registry says"
        );
        assert_eq!(Registry::open(&root).get("a.one").unwrap().version, "1.0.0");
        assert!(
            std::fs::read_dir(root.join("trash"))
                .unwrap()
                .next()
                .is_none()
        );
        // A trash folder for an extra the registry no longer names is
        // simply deleted.
        std::fs::create_dir_all(root.join("trash/stale")).unwrap();
        sweep(&root);
        assert!(!root.join("trash/stale").exists());
    }

    /// The registry cannot be written mid-update: the old version is put
    /// back where it was, the registry still names it, and nothing is left
    /// in staging or the trash.
    #[test]
    fn a_failed_registry_write_rolls_the_update_back() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (one, _) = packed(temp.path(), "a.one", "1.0.0", "");
        let (two, _) = packed(temp.path(), "a.one", "2.0.0", "");
        install(
            &root,
            &request(&one, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        // A folder where the registry's temporary file must be written.
        std::fs::create_dir_all(root.join(format!("{}.new", crate::registry::FILE))).unwrap();
        let error = install(
            &root,
            &request(&two, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        );
        assert!(error.is_err());
        assert_eq!(effect_text(&root), "version 1.0.0\n");
        assert_eq!(Registry::open(&root).get("a.one").unwrap().version, "1.0.0");
        for folder in ["staging", "trash"] {
            let left = std::fs::read_dir(root.join(folder)).map(|mut d| d.next().is_some());
            assert!(!left.unwrap_or(false), "{folder} left behind");
        }
    }

    /// Installing a version an enabled extra cannot take is refused at
    /// install, not only by the plan.
    #[test]
    fn an_update_that_breaks_an_enabled_dependent_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (base1, _) = packed(temp.path(), "a.base", "1.0.0", "");
        let (base2, _) = packed(temp.path(), "a.base", "2.0.0", "");
        let needs = "\n[[requires]]\nid = \"a.base\"\nversion = \"^1\"\n";
        let (user, _) = packed(temp.path(), "a.user", "1.0.0", needs);
        for archive in [&base1, &user] {
            install(
                &root,
                &request(archive, None),
                &mut ok_vet(),
                &never(),
                &mut |_, _, _| {},
            )
            .unwrap();
        }
        let error = install(
            &root,
            &request(&base2, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap_err();
        assert!(matches!(error, Error::Dependency(_)), "{error}");
        assert!(error.to_string().contains("would break it"), "{error}");
        assert_eq!(
            Registry::open(&root).get("a.base").unwrap().version,
            "1.0.0"
        );
        // With the dependent switched off, the update goes ahead.
        set_enabled(&root, "a.user", false, false).unwrap();
        install(
            &root,
            &request(&base2, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        assert_eq!(
            Registry::open(&root).get("a.base").unwrap().version,
            "2.0.0"
        );
    }

    #[test]
    fn sweep_clears_staging_and_trash_and_keeps_downloads() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for dir in ["staging/x", "trash/y", "downloads"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("downloads/a.part"), b"half").unwrap();
        sweep(root);
        assert!(!root.join("staging/x").exists());
        assert!(!root.join("trash/y").exists());
        assert!(root.join("downloads/a.part").is_file());
    }

    /// A Concat starting while another runs leaves the other's staging and
    /// trash alone: they may be an install it is in the middle of.
    #[test]
    fn a_sweep_leaves_alone_what_a_running_concat_is_doing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for dir in ["staging/a.one-1-2", "trash/a.two-1-2"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        let running = attend(root).unwrap();
        sweep(root);
        assert!(root.join("staging/a.one-1-2").exists(), "in progress");
        assert!(root.join("trash/a.two-1-2").exists(), "in progress");
        // Two can run at once.
        let another = attend(root).unwrap();
        drop(running);
        sweep(root);
        assert!(root.join("staging/a.one-1-2").exists());
        drop(another);
        // Alone now: what is left was interrupted.
        sweep(root);
        assert!(!root.join("staging/a.one-1-2").exists());
        assert!(!root.join("trash/a.two-1-2").exists());
    }

    /// What an install needs is checked again as it goes in: a dependency
    /// removed while it unpacked stops it, and nothing is left behind.
    #[test]
    fn a_dependency_removed_during_an_install_stops_it() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (base, _) = packed(temp.path(), "a.base", "1.0.0", "");
        let needs = "\n[[requires]]\nid = \"a.base\"\nversion = \"^1\"\n";
        let (user, _) = packed(temp.path(), "a.user", "1.0.0", needs);
        install(
            &root,
            &request(&base, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        // The vet runs after the first check and before the commit: the
        // moment the person removes the base on the window's thread.
        let error = install(
            &root,
            &request(&user, None),
            &mut |_: &Manifest, _: &Path| {
                uninstall(&root, "a.base", false).unwrap();
                Ok::<_, String>(Vec::new())
            },
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap_err();
        assert!(matches!(error, Error::Dependency(_)), "{error}");
        let registry = Registry::open(&root);
        assert!(registry.get("a.user").is_none());
        assert!(!package_dir(&root, "a.user").exists());
        let staged = std::fs::read_dir(root.join("staging")).map(|mut d| d.next().is_some());
        assert!(!staged.unwrap_or(false), "staging left behind");
    }

    /// Installs on one thread while another switches an extra on and off:
    /// every write is made under the folder's lock from what is on disk, so
    /// no record is lost and no switch is undone by an install that read
    /// the registry before it.
    #[test]
    fn installs_and_switches_on_two_threads_lose_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extras");
        let (one, _) = packed(temp.path(), "a.one", "1.0.0", "");
        let ids: Vec<String> = (0..12).map(|n| format!("b.n{n}")).collect();
        let archives: Vec<PathBuf> = ids
            .iter()
            .map(|id| packed(temp.path(), id, "1.0.0", "").0)
            .collect();
        install(
            &root,
            &request(&one, None),
            &mut ok_vet(),
            &never(),
            &mut |_, _, _| {},
        )
        .unwrap();
        std::thread::scope(|scope| {
            let switching = scope.spawn(|| {
                for n in 0..40 {
                    set_enabled(&root, "a.one", n % 2 == 1, false).unwrap();
                }
            });
            for archive in &archives {
                install(
                    &root,
                    &request(archive, None),
                    &mut ok_vet(),
                    &never(),
                    &mut |_, _, _| {},
                )
                .unwrap();
            }
            switching.join().unwrap();
        });
        let registry = Registry::open(&root);
        for id in &ids {
            assert!(registry.get(id).is_some(), "{id} was lost");
        }
        assert!(
            registry.get("a.one").unwrap().enabled,
            "the last switch, on, stands"
        );
    }
}
