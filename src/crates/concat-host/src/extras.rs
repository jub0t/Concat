// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! Extras on this machine: where they live, which repositories offer them,
//! fetching and caching a repository's signed index, downloading an
//! archive, and the checks an archive's contents must pass before
//! `concat-extras` will move them into place.
//!
//! Everything about whether bytes are trusted is `concat-extras`'s; this
//! module only moves them. The index comes over [`Trust::Shipped`], as the
//! update and sound manifests do - it is what vouches for every archive -
//! and is refused unless the repository's key signed it. An archive comes
//! over [`Trust::System`] through the same resumable downloader models use,
//! since the signed index already says what it must hash to.
//!
//! Extras live in `<data>/extras` - the machine's own folder, beside the
//! models, since packs of fonts and sounds have no business roaming with a
//! profile. Which repositories to ask is the official one, when this build
//! carries its key, plus any listed in `<config>/extras/repositories.toml`:
//!
//! ```toml
//! [[repository]]
//! name = "Our studio"
//! index = "https://extras.example.com/v1/index.json"   # or file:///D:/repo/index.json
//! key = "…64 hex digits, the repository's Ed25519 public key…"
//! ```
//!
//! [`Trust::Shipped`]: crate::net::Trust::Shipped
//! [`Trust::System`]: crate::net::Trust::System

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use concat_extras::index::{self, Entry, Index};
use concat_extras::install::{self, Expected, Phase, Request};
use concat_extras::registry::{Origin, Record, Registry};
use concat_extras::{Kind, Manifest};
use serde::Deserialize;

use crate::AppDirs;

/// The official repository's name.
pub const OFFICIAL_NAME: &str = "Concat Extras";

/// Where the official repository's index is published, beside the sound
/// list in the assets repository.
pub const OFFICIAL_INDEX: &str =
    "https://raw.githubusercontent.com/concatapp/assets/main/extras/v1/index.json";

/// The official repository's Ed25519 public key, in hex. Empty until the
/// maintainers make one (`concat-cli extras keygen`) and publish a signed
/// index: until then the official repository is reported as not set up in
/// this build, and only repositories the person configured are asked.
pub const OFFICIAL_KEY: &str = "";

/// How long a fetched index is used before it is asked for again, unless
/// the person asks: long enough that opening the sheet offline is quiet.
pub const INDEX_MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);

/// Above this a download is confirmed first, with its size.
pub const CONFIRM_ABOVE: u64 = 100 * 1000 * 1000;

/// The extras folder.
pub fn dir(dirs: &AppDirs) -> PathBuf {
    dirs.data.join("extras")
}

/// The folders enabled extras hand to `kind`.
pub fn roots(dirs: &AppDirs, kind: Kind) -> Vec<PathBuf> {
    concat_extras::registry::roots(&dir(dirs), kind)
}

/// The user's own effect packages: `<config>/effects`, one folder each.
pub fn user_effects_dir(dirs: &AppDirs) -> PathBuf {
    dirs.config.join("effects")
}

/// Every folder effect packages load from, in the catalogue's order: the
/// user's own first, then each enabled extra's. The window and the
/// headless tools read the same list, so a package one of them offers is
/// one the other does.
pub fn effect_roots(dirs: &AppDirs) -> Vec<PathBuf> {
    let mut found = vec![user_effects_dir(dirs)];
    found.extend(roots(dirs, Kind::Effects));
    found
}

/// Makes the user's own packages and every enabled extra's part of the
/// app's effects catalogue, for a process with no window: the API on
/// standard input or a socket, the MCP server, `concat-cli`. Each package
/// is parsed and its shader compiled exactly as the window does; what the
/// window adds, a trial run of each shader on its GPU, is the window's,
/// since a headless run may have no GPU to try one on. The packages that
/// would not load, each with why.
pub fn load_effects(dirs: &AppDirs) -> Vec<String> {
    concat_effects::Catalogue::install_from(&effect_roots(dirs), &mut |_| Ok(()))
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// One file an enabled extra brings to a kind: a sticker, a sound.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtraFile {
    /// Stable while the extra is installed: `<extra id>/<path in its root>`.
    /// What a card carries and a project never stores - a placed file is
    /// copied into the project first.
    pub id: String,
    /// What a card calls it: the file's name without its extension.
    pub name: String,
    /// The extra's name, for the card's second line.
    pub extra: String,
    /// The file.
    pub path: PathBuf,
}

/// Every file of one of `extensions` (lowercase) that the enabled extras
/// hand to `kind`, extra by extra in install order, each in path order.
pub fn extra_files(dirs: &AppDirs, kind: Kind, extensions: &[&str]) -> Vec<ExtraFile> {
    let registry = registry(dirs);
    let mut found = Vec::new();
    for record in registry.records().iter().filter(|record| record.enabled) {
        let package = registry.package_dir(&record.id);
        for provide in record
            .provides
            .iter()
            .filter(|provide| provide.kind == kind)
        {
            let root = concat_extras::archive::join(&package, &provide.root);
            for path in files_under(&root) {
                let text = path.to_string_lossy();
                if !extensions.contains(&concat_extras::manifest::extension(&text).as_str()) {
                    continue;
                }
                let Ok(relative) = path.strip_prefix(&root) else {
                    continue;
                };
                let relative: Vec<String> = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect();
                found.push(ExtraFile {
                    id: format!("{}/{}", record.id, relative.join("/")),
                    name: path
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    extra: record.name.clone(),
                    path,
                });
            }
        }
    }
    found
}

/// The install database.
pub fn registry(dirs: &AppDirs) -> Registry {
    Registry::open(&dir(dirs))
}

/// One repository to ask.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Repository {
    /// As the sheet names it.
    pub name: String,
    /// Its `index.json`: https://, or file: for a folder on this machine.
    pub index: String,
    /// Its Ed25519 public key, hex.
    pub key: String,
    /// Concat's own.
    #[serde(skip)]
    pub official: bool,
}

#[derive(Deserialize, Default)]
struct RepositoriesFile {
    #[serde(default, rename = "repository")]
    repositories: Vec<Repository>,
}

/// Where the person's own repositories are listed.
pub fn repositories_file(dirs: &AppDirs) -> PathBuf {
    dirs.config.join("extras").join("repositories.toml")
}

/// Every repository to ask, the official one first, and a sentence for
/// each one that cannot be asked: the official one without a key in this
/// build, a listed one with no key or a plain http:// address.
pub fn repositories(dirs: &AppDirs) -> (Vec<Repository>, Vec<String>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    if OFFICIAL_KEY.is_empty() {
        problems.push(format!(
            "{OFFICIAL_NAME} is not set up in this build of Concat yet, so it offers nothing"
        ));
    } else {
        found.push(Repository {
            name: OFFICIAL_NAME.to_owned(),
            index: OFFICIAL_INDEX.to_owned(),
            key: OFFICIAL_KEY.to_owned(),
            official: true,
        });
    }
    let path = repositories_file(dirs);
    if let Ok(text) = std::fs::read_to_string(&path) {
        match toml::from_str::<RepositoriesFile>(&text) {
            Ok(file) => {
                for repository in file.repositories {
                    if concat_extras::unhex(&repository.key).is_none_or(|key| key.len() != 32) {
                        problems.push(format!(
                            "{} has no valid key in {}, so it is not asked",
                            repository.name,
                            path.display()
                        ));
                    } else if !(repository.index.starts_with("https://")
                        || repository.index.starts_with("file:"))
                    {
                        problems.push(format!(
                            "{}'s index is not an https:// or file: address, so it is not asked",
                            repository.name
                        ));
                    } else if repository.name.trim().eq_ignore_ascii_case(OFFICIAL_NAME) {
                        // Only this build names the official repository, by
                        // the key compiled into it.
                        problems.push(format!(
                            "{} in {} uses the official repository's name, which only Concat itself may; it is not asked",
                            repository.name,
                            path.display()
                        ));
                    } else if found
                        .iter()
                        .any(|held: &Repository| held.name == repository.name)
                    {
                        problems.push(format!("{} is listed twice", repository.name));
                    } else {
                        found.push(repository);
                    }
                }
            }
            Err(error) => problems.push(format!("{} does not parse: {error}", path.display())),
        }
    }
    (found, problems)
}

/// A repository's index, as last fetched.
#[derive(Clone, Debug)]
pub struct Fetched {
    /// Which repository.
    pub repository: Repository,
    /// What it offers. Its signature has been checked.
    pub index: Index,
    /// The index is the cached copy because the repository could not be
    /// reached; the sentence says why.
    pub stale: Option<String>,
}

impl Fetched {
    /// Where an entry's archive is downloaded from.
    pub fn url_of(&self, entry: &Entry) -> String {
        index::resolve_url(&self.repository.index, &entry.download.url)
    }
}

fn slug(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "repository".to_owned()
    } else {
        slug.to_owned()
    }
}

/// The file a `file:` URL names: `file:///D:/repo/index.json` and
/// `file:///home/me/repo/index.json`, with `%20` and its kind decoded.
pub fn file_path(url: &str) -> Option<PathBuf> {
    let rest = url
        .strip_prefix("file://")
        .or_else(|| url.strip_prefix("file:"))?;
    let decoded = percent_decode(rest)?;
    // "/D:/x" is a drive path on Windows; leave "/home/x" as it is.
    let bytes = decoded.as_bytes();
    let path = if bytes.len() > 2
        && bytes[0] == b'/'
        && bytes[2] == b':'
        && bytes[1].is_ascii_alphabetic()
    {
        &decoded[1..]
    } else {
        decoded.as_str()
    };
    Some(PathBuf::from(path))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let pair = text.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The bytes at `url`, refused past `max`: over the network with the
/// shipped roots alone, or from disk for `file:`.
fn fetch_bytes(url: &str, max: usize) -> Result<Vec<u8>, String> {
    if url.starts_with("file:") {
        let path = file_path(url).ok_or_else(|| format!("{url} is not a file address"))?;
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        if bytes.len() > max {
            return Err(format!("{} is larger than an index may be", path.display()));
        }
        return Ok(bytes);
    }
    if !url.starts_with("https://") {
        return Err(format!("{url} is not an https:// address"));
    }
    let response = crate::net::get(
        url,
        Duration::from_secs(15),
        Duration::from_secs(30),
        crate::net::Trust::Shipped,
        &|request| request,
    )
    .map_err(|failure| format!("{url} did not answer: {}", failure.cause.describe()))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{url} stopped partway: {error}"))?;
    if bytes.len() > max {
        return Err(format!("{url} is larger than an index may be"));
    }
    Ok(bytes)
}

/// The cached index and signature of a repository.
///
/// Named for the repository and keyed by its address and its key together,
/// so a repository whose key is changed (by the person, or in a new build)
/// never reads an index the old key signed, and two repositories whose
/// names read alike never share one.
fn cache_dir(dirs: &AppDirs, repository: &Repository) -> PathBuf {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(repository.index.as_bytes());
    hasher.update(b"\n");
    hasher.update(repository.key.trim().to_ascii_lowercase().as_bytes());
    let digest = concat_extras::hex(&hasher.finalize());
    dir(dirs)
        .join("cache")
        .join(format!("{}-{}", slug(&repository.name), &digest[..12]))
}

/// The highest serial seen from a repository: what a later index may not
/// go below. Written only after an index has been verified.
fn seen_serial(cache: &Path) -> u64 {
    std::fs::read_to_string(cache.join("serial"))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

fn read_cache(dirs: &AppDirs, repository: &Repository) -> Option<(Index, Duration)> {
    let cache = cache_dir(dirs, repository);
    let bytes = std::fs::read(cache.join("index.json")).ok()?;
    let signature = std::fs::read_to_string(cache.join("index.json.sig")).ok()?;
    // Checked again on every read: the cache is a file anyone on the
    // machine could have edited.
    let index = Index::verify_and_parse(&bytes, &signature, &repository.key).ok()?;
    let age = std::fs::metadata(cache.join("index.json"))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .unwrap_or(Duration::MAX);
    Some((index, age))
}

/// The repository's index: the cached copy while it is younger than
/// [`INDEX_MAX_AGE`] (unless `force`), otherwise fetched, checked and
/// cached - and the cached copy, marked stale, when the repository cannot
/// be reached.
pub fn load_index(dirs: &AppDirs, repository: &Repository, force: bool) -> Result<Fetched, String> {
    let cached = read_cache(dirs, repository);
    if !force
        && let Some((index, age)) = &cached
        && *age < INDEX_MAX_AGE
    {
        return Ok(Fetched {
            repository: repository.clone(),
            index: index.clone(),
            stale: None,
        });
    }
    let fetched = (|| {
        let bytes = fetch_bytes(&repository.index, index::MAX_INDEX_SIZE)?;
        let signature = fetch_bytes(&format!("{}.sig", repository.index), 1024)?;
        let signature = String::from_utf8_lossy(&signature).trim().to_owned();
        let parsed = Index::verify_and_parse(&bytes, &signature, &repository.key)
            .map_err(|error| error.to_string())?;
        let cache = cache_dir(dirs, repository);
        // Not older than anything this repository has served before: a
        // signature does not expire, so an old index served again would
        // otherwise be believed - and would hide what it fixed.
        let seen = seen_serial(&cache).max(cached.as_ref().map_or(0, |(held, _)| held.serial));
        parsed
            .check_not_older_than(seen)
            .map_err(|error| error.to_string())?;
        if std::fs::create_dir_all(&cache).is_ok() {
            let _ = std::fs::write(cache.join("index.json"), &bytes);
            let _ = std::fs::write(cache.join("index.json.sig"), &signature);
            if parsed.serial > seen {
                let _ = std::fs::write(cache.join("serial"), parsed.serial.to_string());
            }
        }
        Ok::<Index, String>(parsed)
    })();
    match (fetched, cached) {
        (Ok(index), _) => Ok(Fetched {
            repository: repository.clone(),
            index,
            stale: None,
        }),
        (Err(why), Some((index, _))) => {
            log::warn!(
                "extras: {}: {why}; showing the copy fetched before",
                repository.name
            );
            Ok(Fetched {
                repository: repository.clone(),
                index,
                stale: Some(why),
            })
        }
        (Err(why), None) => Err(why),
    }
}

/// Every repository's last good index from the cache alone, checked again,
/// with no network: what a project that opens asks, on the window's
/// thread, to say which extra brings an effect it is missing.
pub fn cached_indexes(dirs: &AppDirs) -> Vec<Fetched> {
    repositories(dirs)
        .0
        .into_iter()
        .filter_map(|repository| {
            read_cache(dirs, &repository).map(|(index, _)| Fetched {
                repository,
                index,
                stale: None,
            })
        })
        .collect()
}

/// Every repository's index, with a sentence for each one that could not
/// be had at all.
pub fn load_all(dirs: &AppDirs, force: bool) -> (Vec<Fetched>, Vec<String>) {
    let (repositories, mut problems) = repositories(dirs);
    let mut fetched = Vec::new();
    for repository in &repositories {
        match load_index(dirs, repository, force) {
            Ok(one) => fetched.push(one),
            Err(why) => problems.push(format!("{}: {why}", repository.name)),
        }
    }
    (fetched, problems)
}

/// Every offered entry, one per id - the first repository to offer an id
/// is the one it comes from - with the repository it came from.
pub fn offered(fetched: &[Fetched]) -> Vec<(&Fetched, &Entry)> {
    let mut out: Vec<(&Fetched, &Entry)> = Vec::new();
    for one in fetched {
        for entry in &one.index.extras {
            if !out.iter().any(|(_, held)| held.id == entry.id) {
                out.push((one, entry));
            }
        }
    }
    out
}

/// What an install is doing, for the sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// Bytes received of bytes in all.
    Downloading(u64, u64),
    /// Checking the archive's digest.
    Verifying,
    /// Bytes unpacked of bytes in all.
    Unpacking(u64, u64),
    /// Checking what was unpacked.
    Checking,
    /// Moving it into place.
    Finishing,
}

/// Free space on the volume holding `path`, when the system says.
pub fn free_space(path: &Path) -> Option<u64> {
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        probe = probe.parent()?.to_path_buf();
    }
    free_space_at(&probe)
}

#[cfg(windows)]
fn free_space_at(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt as _;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    // SAFETY: a NUL-terminated wide path and a valid out pointer; the
    // other two outputs are optional and passed as null.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

#[cfg(unix)]
fn free_space_at(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt as _;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: a NUL-terminated path and a valid out pointer.
    let ok = unsafe { libc::statvfs(c.as_ptr(), &mut stat) } == 0;
    #[allow(clippy::unnecessary_cast)]
    ok.then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
}

#[cfg(not(any(unix, windows)))]
fn free_space_at(_path: &Path) -> Option<u64> {
    None
}

/// Whether there is room for an archive of `download` bytes that unpacks
/// to `installed`: both side by side, with a margin. A sentence when not.
pub fn check_room(dirs: &AppDirs, download: u64, installed: u64) -> Result<(), String> {
    let needed = download
        .saturating_add(installed)
        .saturating_add(installed / 10)
        .saturating_add(16 * 1024 * 1024);
    match free_space(&dir(dirs)) {
        Some(free) if free < needed => Err(format!(
            "there is not enough free space: it needs {}, and {} is free",
            concat_extras::human_size(needed),
            concat_extras::human_size(free)
        )),
        _ => Ok(()),
    }
}

/// Downloads `entry`'s archive into the extras folder's `downloads/`: the
/// copy already there when it is whole, otherwise resumed from its `.part`.
/// Never more than the index says it is - a server that keeps sending is
/// cut off. The archive's path.
pub fn download(
    dirs: &AppDirs,
    fetched: &Fetched,
    entry: &Entry,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<PathBuf, String> {
    let folder = dir(dirs).join("downloads");
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("could not make {}: {error}", folder.display()))?;
    let name = format!(
        "{}-{}.{}",
        entry.id,
        entry.version,
        concat_extras::archive::EXTENSION
    );
    let whole = folder.join(&name);
    if std::fs::metadata(&whole).is_ok_and(|meta| meta.len() == entry.download.size)
        && concat_extras::archive::sha256_file(&whole, cancel)
            .is_ok_and(|digest| digest == entry.download.sha256)
    {
        progress(Progress::Downloading(
            entry.download.size,
            entry.download.size,
        ));
        return Ok(whole);
    }
    let partial = folder.join(format!("{name}.part"));
    let url = fetched.url_of(entry);
    let size = entry.download.size;
    if url.starts_with("file:") {
        let source = file_path(&url).ok_or_else(|| format!("{url} is not a file address"))?;
        copy_with_progress(&source, &partial, size, cancel, progress)?;
    } else {
        // The downloader's cancel is ours: set by the person's cancel or
        // by too many bytes, both seen in the progress callback.
        let stop = AtomicBool::new(false);
        let mut overran = false;
        let result = crate::models::download(
            &url,
            &partial,
            size,
            &stop,
            "cancelled",
            &mut |received, total| {
                if cancel.load(Ordering::Relaxed) {
                    stop.store(true, Ordering::Relaxed);
                }
                if received > size {
                    overran = true;
                    stop.store(true, Ordering::Relaxed);
                }
                progress(Progress::Downloading(
                    received.min(size),
                    total.min(size).max(1),
                ));
            },
        );
        if overran {
            let _ = std::fs::remove_file(&partial);
            return Err(format!(
                "{url} sent more than the repository says the extra is; it was stopped"
            ));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_owned());
        }
        result?;
    }
    let got = std::fs::metadata(&partial)
        .map(|meta| meta.len())
        .unwrap_or(0);
    if got != size {
        let _ = std::fs::remove_file(&partial);
        return Err(format!(
            "the download of {} is {got} bytes; the repository says {size}",
            entry.name
        ));
    }
    std::fs::rename(&partial, &whole)
        .map_err(|error| format!("could not keep the download: {error}"))?;
    Ok(whole)
}

fn copy_with_progress(
    source: &Path,
    dest: &Path,
    size: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), String> {
    use std::io::Write as _;
    let mut input = std::fs::File::open(source)
        .map_err(|error| format!("could not read {}: {error}", source.display()))?;
    let mut output = std::fs::File::create(dest)
        .map_err(|error| format!("could not write {}: {error}", dest.display()))?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut copied = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(dest);
            return Err("cancelled".to_owned());
        }
        let n = input
            .read(&mut buffer)
            .map_err(|error| format!("could not read {}: {error}", source.display()))?;
        if n == 0 {
            break;
        }
        copied += n as u64;
        if copied > size {
            let _ = std::fs::remove_file(dest);
            return Err(format!(
                "{} is larger than the repository says",
                source.display()
            ));
        }
        output
            .write_all(&buffer[..n])
            .map_err(|error| format!("could not write {}: {error}", dest.display()))?;
        progress(Progress::Downloading(copied, size.max(1)));
    }
    output
        .flush()
        .map_err(|error| format!("could not write {}: {error}", dest.display()))
}

/// The checks an unpacked extra must pass before it is moved into place,
/// kind by kind, and the ids it provides:
///
/// * effects: every package folder loads exactly as the catalogue will
///   load it (manifest, fixtures, shader compiled), is not under Concat's
///   own author, and has an id no other installed package answers to;
/// * fonts: every font file is one the text engine reads a family from;
/// * stickers: every image's first bytes are the format its name says.
///
/// Title presets and sounds are read leniently where they are used - a
/// broken one is skipped there - so they are only held to the manifest's
/// file types.
pub fn vet(dirs: &AppDirs, manifest: &Manifest, staged: &Path) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let own_dir = concat_extras::registry::package_dir(&dir(dirs), manifest.id());
    for provide in &manifest.provides {
        let root = concat_extras::archive::join(staged, &provide.root);
        match provide.kind {
            Kind::Effects => {
                let folders =
                    concat_effects::package_folders(&root).map_err(|error| error.to_string())?;
                if folders.is_empty() {
                    return Err(format!(
                        "its `{}` folder holds no effect package",
                        provide.root
                    ));
                }
                for folder in folders {
                    let package = concat_effects::Package::from_folder(&folder)
                        .map_err(|error| error.to_string())?;
                    let id = package.id().to_owned();
                    let author = id.split('.').next().unwrap_or_default();
                    if author == concat_effects::catalogue::BUILTIN_AUTHOR {
                        return Err(format!(
                            "its package {id} is under `{author}.`, which only Concat's own effects may use"
                        ));
                    }
                    let names = std::iter::once(id.clone())
                        .chain(package.manifest.effect.aliases.iter().cloned());
                    for name in names {
                        if let Some(held) = concat_effects::Catalogue::builtin().get(&name) {
                            let ours = held
                                .folder
                                .as_deref()
                                .is_some_and(|folder| folder.starts_with(&own_dir));
                            if !ours {
                                return Err(format!(
                                    "its package {id} answers to `{name}`, which {} already does",
                                    held.manifest.effect.name
                                ));
                            }
                        }
                    }
                    ids.push(id);
                }
            }
            Kind::Fonts => {
                for path in files_under(&root) {
                    let extension = concat_extras::manifest::extension(&path.to_string_lossy());
                    if matches!(extension.as_str(), "ttf" | "otf" | "ttc")
                        && concat_text::families_in(&path).is_empty()
                    {
                        return Err(format!("{} is not a font Concat can read", path.display()));
                    }
                }
            }
            Kind::Stickers => {
                for path in files_under(&root) {
                    sniff_image(&path)?;
                }
            }
            Kind::TitlePresets | Kind::Sounds => {}
        }
    }
    Ok(ids)
}

/// Every file under `root`, depth first.
pub fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(kind) if kind.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

fn sniff_image(path: &Path) -> Result<(), String> {
    let extension = concat_extras::manifest::extension(&path.to_string_lossy());
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path)
        .and_then(|mut file| file.read(&mut head))
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let head = &head[..n];
    let fine = match extension.as_str() {
        "png" => head.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" | "jpeg" => head.starts_with(&[0xff, 0xd8, 0xff]),
        "webp" => head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP",
        _ => true,
    };
    if fine {
        Ok(())
    } else {
        Err(format!("{} is not the image its name says", path.display()))
    }
}

/// Downloads and installs `entry` from the repository it came from.
pub fn install_entry(
    dirs: &AppDirs,
    fetched: &Fetched,
    entry: &Entry,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Record, String> {
    entry.compatible(
        concat_extras::APP_VERSION,
        &concat_extras::current_platform(),
    )?;
    check_room(dirs, entry.download.size, entry.installed_size)?;
    let archive = download(dirs, fetched, entry, cancel, progress)?;
    let request = Request {
        archive: &archive,
        expected: Some(Expected {
            id: &entry.id,
            version: &entry.version,
            size: entry.download.size,
            sha256: &entry.download.sha256,
        }),
        origin: Origin::Repository {
            name: fetched.repository.name.clone(),
        },
        app_version: concat_extras::APP_VERSION,
        platform: &concat_extras::current_platform(),
        allow_downgrade: false,
    };
    let result = run_install(dirs, &request, cancel, progress);
    // A digest that did not match is a download to be had again, not one
    // to resume from.
    if let Err(why) = &result
        && why.contains("SHA-256")
    {
        let _ = std::fs::remove_file(&archive);
    }
    if result.is_ok() {
        let _ = std::fs::remove_file(&archive);
    }
    result
}

fn run_install(
    dirs: &AppDirs,
    request: &Request<'_>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Record, String> {
    let root = dir(dirs);
    install::install(
        &root,
        request,
        &mut |manifest, staged| vet(dirs, manifest, staged),
        cancel,
        &mut |phase, done, all| {
            progress(match phase {
                Phase::Verifying => Progress::Verifying,
                Phase::Unpacking => Progress::Unpacking(done, all.max(1)),
                Phase::Checking => Progress::Checking,
                Phase::Finishing => Progress::Finishing,
            })
        },
    )
    .map_err(|error| error.to_string())
}

/// What a file the person chose claims to be, and whether a signed index
/// vouches for it.
#[derive(Clone, Debug)]
pub struct Inspected {
    /// Its manifest, validated - but not believed until installed.
    pub manifest: Manifest,
    /// Its SHA-256.
    pub sha256: String,
    /// The repository whose signed index lists exactly this file.
    pub vouched_by: Option<String>,
}

/// Reads what an archive the person chose claims to be, and looks for its
/// digest in every index fetched.
pub fn inspect_file(path: &Path, fetched: &[Fetched]) -> Result<Inspected, String> {
    let manifest =
        concat_extras::archive::read_manifest(path).map_err(|error| error.to_string())?;
    manifest
        .compatible(
            concat_extras::APP_VERSION,
            &concat_extras::current_platform(),
        )
        .map_err(|error| error.to_string())?;
    let sha256 = concat_extras::archive::sha256_file(path, &AtomicBool::new(false))
        .map_err(|error| error.to_string())?;
    let vouched_by = fetched
        .iter()
        .find(|one| {
            one.index
                .get(manifest.id())
                .is_some_and(|entry| entry.download.sha256 == sha256)
        })
        .map(|one| one.repository.name.clone());
    Ok(Inspected {
        manifest,
        sha256,
        vouched_by,
    })
}

/// Installs an archive the person chose. One a signed index vouches for is
/// checked against that entry; one none does installs only with
/// `accept_unverified` - the sheet asks first - and is recorded as such.
pub fn install_file(
    dirs: &AppDirs,
    path: &Path,
    fetched: &[Fetched],
    accept_unverified: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Record, String> {
    let inspected = inspect_file(path, fetched)?;
    let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    let entry = fetched.iter().find_map(|one| {
        one.index
            .get(inspected.manifest.id())
            .filter(|entry| entry.download.sha256 == inspected.sha256)
    });
    if entry.is_none() && !accept_unverified {
        return Err(format!(
            "{} is not vouched for by any repository; it was not installed",
            inspected.manifest.extra.name
        ));
    }
    check_room(dirs, 0, inspected.manifest.total_size())?;
    let request = Request {
        archive: path,
        expected: entry.map(|entry| Expected {
            id: &entry.id,
            version: &entry.version,
            size,
            sha256: &entry.download.sha256,
        }),
        origin: Origin::File {
            verified: entry.is_some(),
        },
        app_version: concat_extras::APP_VERSION,
        platform: &concat_extras::current_platform(),
        allow_downgrade: false,
    };
    run_install(dirs, &request, cancel, progress)
}

/// This run's claim on the extras folder; see [`sweep`].
static PRESENCE: std::sync::OnceLock<install::Presence> = std::sync::OnceLock::new();

/// Clears what an interrupted install left - unless another Concat is
/// running on the same folder - and then claims the folder for this run,
/// so a Concat started after this one leaves this one's installs alone.
/// Called once at start, by the window: the one part of Concat that
/// installs.
pub fn sweep(dirs: &AppDirs) {
    let root = dir(dirs);
    install::sweep(&root);
    if PRESENCE.get().is_none() {
        match install::attend(&root) {
            Ok(presence) => {
                let _ = PRESENCE.set(presence);
            }
            Err(error) => log::warn!("extras: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_extras::pack;

    const SHADER: &str = "fn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }";

    /// A source folder for an effects extra `id` with one package
    /// `package`, whose shader is `shader`.
    fn effects_source(at: &Path, id: &str, package: &str, shader: &str) -> PathBuf {
        let folder = at.join(id);
        let look = folder.join("effects").join(package);
        std::fs::create_dir_all(&look).unwrap();
        std::fs::write(
            look.join("effect.toml"),
            format!("format = 2\n[effect]\nid = \"{package}\"\nname = \"Look\"\nkind = \"effect\"\n[wgsl]\nentry = \"effect.wgsl\"\n"),
        )
        .unwrap();
        std::fs::write(look.join("effect.wgsl"), shader).unwrap();
        std::fs::write(
            folder.join("extra.toml"),
            format!("format = 1\n[extra]\nid = \"{id}\"\nname = \"Looks\"\nversion = \"1.0.0\"\nauthor = \"Me\"\nlicence = \"MIT\"\n[[provides]]\nkind = \"effects\"\nroot = \"effects\"\n"),
        )
        .unwrap();
        folder
    }

    /// A repository on disk with these source folders, listed in the
    /// person's repositories file, and the dirs that point at it.
    fn local_repository(temp: &Path, sources: &[PathBuf]) -> (AppDirs, Repository) {
        let (secret, public) = pack::keygen().unwrap();
        let repo = temp.join("repo dir");
        pack::build_repository(sources, &repo, "Local", "1", &secret).unwrap();
        let dirs = AppDirs::under(&temp.join("app"));
        let url = format!(
            "file:///{}",
            repo.join("index.json")
                .to_string_lossy()
                .replace('\\', "/")
                .trim_start_matches('/')
                .replace(' ', "%20")
        );
        std::fs::create_dir_all(repositories_file(&dirs).parent().unwrap()).unwrap();
        std::fs::write(
            repositories_file(&dirs),
            format!("[[repository]]\nname = \"Local\"\nindex = \"{url}\"\nkey = \"{public}\"\n"),
        )
        .unwrap();
        let repository = Repository {
            name: "Local".to_owned(),
            index: url,
            key: public,
            official: false,
        };
        (dirs, repository)
    }

    /// The example in docs/development/extras-example packs, signs and
    /// installs as a person's would, and both its shaders load: the
    /// example the package guide points at is never a broken one.
    #[test]
    fn the_documented_example_installs_and_loads() {
        let example = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/development/extras-example/example.starter-pack");
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("example.starter-pack");
        copy_tree(&example, &source);
        let (dirs, _) = local_repository(temp.path(), &[source]);
        let (fetched, _) = load_all(&dirs, false);
        let entry = fetched[0]
            .index
            .get("example.starter-pack")
            .unwrap()
            .clone();
        assert_eq!(
            entry.provides_ids,
            ["example.diagonal-wipe", "example.warm-glow"]
        );
        let record = install_entry(
            &dirs,
            &fetched[0],
            &entry,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            record.provides_ids,
            ["example.diagonal-wipe", "example.warm-glow"]
        );
        let (catalogue, errors) =
            concat_effects::Catalogue::assemble(&roots(&dirs, Kind::Effects), &mut |_| Ok(()));
        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            catalogue
                .get("example.warm-glow")
                .and_then(|p| p.shader())
                .is_some()
        );
        assert!(
            catalogue
                .get("example.diagonal-wipe")
                .and_then(|p| p.transition())
                .is_some()
        );
    }

    /// An effects extra a project uses, updated while the project is open:
    /// the same id now answers with the new shader (and so a new pipeline,
    /// which is keyed by the shader's source), only the new version is on
    /// disk, and a removal takes the id out of what loads - the project's
    /// link to it is the project's, and nothing here touches it.
    #[test]
    fn an_updated_effect_answers_with_its_new_shader_and_a_removed_one_is_gone() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(&temp.path().join("app"));
        let newer = "fn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv) * 0.371; }";
        let one = effects_source(&temp.path().join("one"), "me.looks", "me.soft", SHADER);
        let two = effects_source(&temp.path().join("two"), "me.looks", "me.soft", newer);
        let manifest = two.join("extra.toml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(&manifest, text.replace("\"1.0.0\"", "\"1.1.0\"")).unwrap();
        let shader = |dirs: &AppDirs| {
            let (catalogue, errors) =
                concat_effects::Catalogue::assemble(&effect_roots(dirs), &mut |_| Ok(()));
            assert!(errors.is_empty(), "{errors:?}");
            catalogue
                .get("me.soft")
                .and_then(|package| package.shader())
                .map(|shader| shader.source().to_owned())
        };
        for (source, version) in [(&one, "1.0.0"), (&two, "1.1.0")] {
            let archive = temp.path().join(format!("looks-{version}.concatpkg"));
            pack::pack(source, &archive).unwrap();
            let record = install_file(
                &dirs,
                &archive,
                &[],
                true,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap();
            assert_eq!(record.version, version);
            assert_eq!(record.provides_ids, ["me.soft"]);
            let loaded = shader(&dirs).expect("the package loads");
            assert_eq!(
                loaded.contains("0.371"),
                version == "1.1.0",
                "{version}'s shader: {loaded}"
            );
        }
        assert_eq!(
            files_under(&dir(&dirs).join("packages")).len(),
            files_under(&two).len(),
            "only the new version is installed"
        );
        install::uninstall(&dir(&dirs), "me.looks", false).unwrap();
        assert_eq!(shader(&dirs), None, "removed: the id no longer loads");
        assert!(!dir(&dirs).join("packages").join("me.looks").exists());
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[test]
    fn file_urls_name_paths_on_every_platform() {
        assert_eq!(
            file_path("file:///D:/my%20repo/index.json"),
            Some(PathBuf::from("D:/my repo/index.json"))
        );
        assert_eq!(
            file_path("file:///home/me/index.json"),
            Some(PathBuf::from("/home/me/index.json"))
        );
        assert_eq!(file_path("https://x"), None);
        assert_eq!(file_path("file:///bad%zz"), None);
    }

    #[test]
    fn a_local_repository_installs_end_to_end_and_lands_in_the_effects_roots() {
        let temp = tempfile::tempdir().unwrap();
        let source = effects_source(temp.path(), "me.looks", "me.soft", SHADER);
        let (dirs, repository) = local_repository(temp.path(), &[source]);

        let (repositories, problems) = repositories(&dirs);
        assert_eq!(repositories, vec![repository.clone()]);
        assert!(
            problems.iter().any(|p| p.contains("not set up")),
            "{problems:?}"
        );

        let (fetched, problems) = load_all(&dirs, false);
        assert!(
            problems.iter().all(|p| p.contains("not set up")),
            "{problems:?}"
        );
        assert_eq!(fetched.len(), 1);
        let entry = fetched[0].index.get("me.looks").unwrap().clone();
        assert_eq!(entry.provides_ids, ["me.soft"]);

        let mut saw_download = false;
        let record = install_entry(
            &dirs,
            &fetched[0],
            &entry,
            &AtomicBool::new(false),
            &mut |p| {
                saw_download |= matches!(p, Progress::Downloading(..));
            },
        )
        .unwrap();
        assert!(saw_download);
        assert_eq!(record.provides_ids, ["me.soft"]);
        assert!(record.origin.verified());
        let effect_roots = roots(&dirs, Kind::Effects);
        assert_eq!(effect_roots.len(), 1);
        assert!(
            effect_roots[0]
                .join("me.soft")
                .join("effect.toml")
                .is_file()
        );
        // The catalogue loads it from there, as the window will.
        let mut catalogue = concat_effects::Catalogue::new();
        assert!(catalogue.load_dir(&effect_roots[0]).is_empty());
        assert!(catalogue.get("me.soft").is_some());
        // The download is not kept once installed.
        assert!(files_under(&dir(&dirs).join("downloads")).is_empty());
        // The index came from the cache the second time: still verified.
        let again = load_index(&dirs, &repository, false).unwrap();
        assert!(again.stale.is_none());
    }

    #[test]
    fn an_effect_that_does_not_compile_is_refused_at_install() {
        let temp = tempfile::tempdir().unwrap();
        let source = effects_source(temp.path(), "me.broken", "me.bad", "fn effect( {");
        let (dirs, _) = local_repository(temp.path(), &[source]);
        let (fetched, _) = load_all(&dirs, false);
        let entry = fetched[0].index.get("me.broken").unwrap().clone();
        let error = install_entry(
            &dirs,
            &fetched[0],
            &entry,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.contains("shader"), "{error}");
        assert!(registry(&dirs).records().is_empty());
    }

    #[test]
    fn an_extra_may_not_take_concats_own_effect_ids() {
        let temp = tempfile::tempdir().unwrap();
        let source = effects_source(temp.path(), "me.sneaky", "concat.glow", SHADER);
        let (dirs, _) = local_repository(temp.path(), &[source]);
        let (fetched, _) = load_all(&dirs, false);
        let entry = fetched[0].index.get("me.sneaky").unwrap().clone();
        let error = install_entry(
            &dirs,
            &fetched[0],
            &entry,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        // The effects catalogue refuses the author itself, in its words.
        assert!(error.contains("under `concat.`"), "{error}");
        assert!(registry(&dirs).records().is_empty());
    }

    #[test]
    fn a_tampered_index_or_archive_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let source = effects_source(temp.path(), "me.looks", "me.soft", SHADER);
        let (dirs, repository) = local_repository(temp.path(), &[source]);
        let index_path = file_path(&repository.index).unwrap();

        // The archive swapped for another: its digest no longer matches.
        let (fetched, _) = load_all(&dirs, false);
        let entry = fetched[0].index.get("me.looks").unwrap().clone();
        let archive = index_path.parent().unwrap().join(&entry.download.url);
        let mut bytes = std::fs::read(&archive).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&archive, &bytes).unwrap();
        let error = install_entry(
            &dirs,
            &fetched[0],
            &entry,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(
            error.contains("not the file the repository vouches for"),
            "{error}"
        );
        assert!(registry(&dirs).records().is_empty());

        // The index edited: its signature no longer checks out, and with
        // no cache there is nothing to fall back on.
        let _ = std::fs::remove_dir_all(dir(&dirs).join("cache"));
        let text = std::fs::read_to_string(&index_path)
            .unwrap()
            .replace("Looks", "Lookz");
        std::fs::write(&index_path, text).unwrap();
        let error = load_index(&dirs, &repository, true).unwrap_err();
        assert!(
            error.contains("not signed by the repository's key"),
            "{error}"
        );
    }

    /// A source that sends more than the signed index says is cut off
    /// while it is read, not trusted until the digest check: an endless
    /// download cannot fill the disk.
    #[test]
    fn a_download_longer_than_the_index_says_is_stopped() {
        let temp = tempfile::tempdir().unwrap();
        let source = effects_source(temp.path(), "me.looks", "me.soft", SHADER);
        let (dirs, repository) = local_repository(temp.path(), &[source]);
        let (fetched, _) = load_all(&dirs, false);
        let entry = fetched[0].index.get("me.looks").unwrap().clone();
        let archive = file_path(&repository.index)
            .unwrap()
            .parent()
            .unwrap()
            .join(&entry.download.url);
        let mut bytes = std::fs::read(&archive).unwrap();
        bytes.extend(std::iter::repeat_n(0u8, 4 * 1024 * 1024));
        std::fs::write(&archive, bytes).unwrap();
        let error = install_entry(
            &dirs,
            &fetched[0],
            &entry,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.contains("larger than the repository says"), "{error}");
        assert!(
            files_under(&dir(&dirs).join("downloads")).is_empty(),
            "nothing kept"
        );
        assert!(registry(&dirs).records().is_empty());
    }

    /// A repository signed with `secret` at `serial` and `expires`, in
    /// `temp/<folder>`, and a `Repository` naming it with `public`.
    fn signed_repository(
        temp: &Path,
        folder: &str,
        sources: &[PathBuf],
        (secret, public): (&str, &str),
        serial: u64,
        expires: u64,
    ) -> Repository {
        let out = temp.join(folder);
        pack::build_repository_with(sources, &out, "Signed", "r", secret, serial, expires).unwrap();
        Repository {
            name: "Signed".to_owned(),
            index: format!(
                "file:///{}",
                out.join("index.json")
                    .to_string_lossy()
                    .replace('\\', "/")
                    .trim_start_matches('/')
                    .replace(' ', "%20")
            ),
            key: public.to_owned(),
            official: false,
        }
    }

    /// An index signed with the right key but older than one already seen
    /// is an old one served again: refused, and the verified newer copy
    /// stays in use, marked stale with the reason.
    #[test]
    fn a_replayed_older_index_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(&temp.path().join("app"));
        let (secret, public) = pack::keygen().unwrap();
        let source = effects_source(temp.path(), "me.looks", "me.soft", SHADER);
        let newer = signed_repository(
            temp.path(),
            "repo",
            std::slice::from_ref(&source),
            (&secret, &public),
            20,
            0,
        );
        assert_eq!(load_index(&dirs, &newer, true).unwrap().index.serial, 20);
        // The same address now serves an older index, properly signed.
        let older = signed_repository(temp.path(), "repo", &[source], (&secret, &public), 10, 0);
        let fetched = load_index(&dirs, &older, true).unwrap();
        assert_eq!(fetched.index.serial, 20, "the newer copy is kept");
        let why = fetched.stale.expect("marked stale");
        assert!(why.contains("old index served again"), "{why}");
    }

    /// An expired index is refused, from the repository and from the cache.
    #[test]
    fn an_expired_index_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(&temp.path().join("app"));
        let (secret, public) = pack::keygen().unwrap();
        let source = effects_source(temp.path(), "me.looks", "me.soft", SHADER);
        let yesterday = concat_extras::index::unix_now() - 86_400;
        let expired = signed_repository(
            temp.path(),
            "repo",
            &[source],
            (&secret, &public),
            1,
            yesterday,
        );
        let error = load_index(&dirs, &expired, true).unwrap_err();
        assert!(error.contains("expired"), "{error}");
        assert!(cached_indexes(&dirs).is_empty());
    }

    /// The cache is keyed by the repository's key: with a new key, the
    /// index the old key signed is never read, even with the repository
    /// unreachable.
    #[test]
    fn a_changed_key_never_reads_the_old_keys_cache() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(&temp.path().join("app"));
        let (secret, public) = pack::keygen().unwrap();
        let source = effects_source(temp.path(), "me.looks", "me.soft", SHADER);
        let repository =
            signed_repository(temp.path(), "repo", &[source], (&secret, &public), 1, 0);
        load_index(&dirs, &repository, true).unwrap();
        std::fs::remove_file(file_path(&repository.index).unwrap()).unwrap();
        let (_, other) = pack::keygen().unwrap();
        let rekeyed = Repository {
            key: other,
            ..repository.clone()
        };
        assert!(
            load_index(&dirs, &rekeyed, true).is_err(),
            "no cache under the new key"
        );
        assert!(
            load_index(&dirs, &repository, true)
                .unwrap()
                .stale
                .is_some(),
            "the old key's copy"
        );
    }

    #[test]
    fn the_official_name_is_reserved() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(temp.path());
        std::fs::create_dir_all(repositories_file(&dirs).parent().unwrap()).unwrap();
        std::fs::write(
            repositories_file(&dirs),
            format!(
                "[[repository]]\nname = \"concat extras\"\nindex = \"https://evil.example/index.json\"\nkey = \"{}\"\n",
                "ab".repeat(32)
            ),
        )
        .unwrap();
        let (found, problems) = repositories(&dirs);
        assert!(found.is_empty());
        assert!(
            problems
                .iter()
                .any(|p| p.contains("official repository's name")),
            "{problems:?}"
        );
    }

    /// Stickers and sounds an extra brings are listed by kind and type,
    /// with ids that name the extra, and stop being listed when it is
    /// switched off or removed.
    #[test]
    fn an_extras_files_are_listed_by_kind_until_it_is_switched_off() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("me.pack");
        std::fs::create_dir_all(source.join("stickers/stars")).unwrap();
        std::fs::create_dir_all(source.join("sounds")).unwrap();
        std::fs::write(
            source.join("stickers/stars/Gold Star.png"),
            b"\x89PNG\r\n\x1a\nrest",
        )
        .unwrap();
        std::fs::write(source.join("stickers/notes.txt"), b"credits").unwrap();
        std::fs::write(source.join("sounds/whoosh.wav"), b"RIFF....WAVE").unwrap();
        std::fs::write(
            source.join("extra.toml"),
            "format = 1\n[extra]\nid = \"me.pack\"\nname = \"Pack\"\nversion = \"1.0.0\"\nauthor = \"Me\"\nlicence = \"CC0-1.0\"\n[[provides]]\nkind = \"stickers\"\nroot = \"stickers\"\n[[provides]]\nkind = \"sounds\"\nroot = \"sounds\"\n",
        )
        .unwrap();
        let (dirs, _) = local_repository(temp.path(), &[source]);
        let (fetched, _) = load_all(&dirs, false);
        let entry = fetched[0].index.get("me.pack").unwrap().clone();
        install_entry(
            &dirs,
            &fetched[0],
            &entry,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();

        let stickers = extra_files(&dirs, Kind::Stickers, &["png"]);
        assert_eq!(stickers.len(), 1, "the text file is not a sticker");
        assert_eq!(stickers[0].id, "me.pack/stars/Gold Star.png");
        assert_eq!(
            (stickers[0].name.as_str(), stickers[0].extra.as_str()),
            ("Gold Star", "Pack")
        );
        assert!(stickers[0].path.is_file());
        assert_eq!(
            extra_files(&dirs, Kind::Sounds, &["wav"])[0].id,
            "me.pack/whoosh.wav"
        );
        assert!(extra_files(&dirs, Kind::Fonts, &["ttf"]).is_empty());

        concat_extras::install::set_enabled(&dir(&dirs), "me.pack", false, false).unwrap();
        assert!(extra_files(&dirs, Kind::Stickers, &["png"]).is_empty());
        concat_extras::install::set_enabled(&dir(&dirs), "me.pack", true, false).unwrap();
        concat_extras::install::uninstall(&dir(&dirs), "me.pack", false).unwrap();
        assert!(extra_files(&dirs, Kind::Sounds, &["wav"]).is_empty());
    }

    /// The headless tools and the window load effects from the same
    /// folders: the user's own first, then each enabled extra's.
    #[test]
    fn effect_roots_put_the_users_own_first() {
        let temp = tempfile::tempdir().unwrap();
        let source = effects_source(temp.path(), "me.looks", "me.soft", SHADER);
        let (dirs, _) = local_repository(temp.path(), &[source]);
        assert_eq!(effect_roots(&dirs), vec![user_effects_dir(&dirs)]);
        let (fetched, _) = load_all(&dirs, false);
        let entry = fetched[0].index.get("me.looks").unwrap().clone();
        install_entry(
            &dirs,
            &fetched[0],
            &entry,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let found = effect_roots(&dirs);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], user_effects_dir(&dirs));
        assert!(found[1].join("me.soft").join("effect.toml").is_file());
    }

    /// A cancel stops a download and installs nothing.
    #[test]
    fn a_cancelled_download_installs_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let source = effects_source(temp.path(), "me.looks", "me.soft", SHADER);
        let (dirs, _) = local_repository(temp.path(), &[source]);
        let (fetched, _) = load_all(&dirs, false);
        let entry = fetched[0].index.get("me.looks").unwrap().clone();
        let error = install_entry(
            &dirs,
            &fetched[0],
            &entry,
            &AtomicBool::new(true),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert!(registry(&dirs).records().is_empty());
    }

    #[test]
    fn an_unreachable_repository_falls_back_to_its_verified_cache() {
        let temp = tempfile::tempdir().unwrap();
        let source = effects_source(temp.path(), "me.looks", "me.soft", SHADER);
        let (dirs, repository) = local_repository(temp.path(), &[source]);
        load_index(&dirs, &repository, true).unwrap();
        let index_path = file_path(&repository.index).unwrap();
        std::fs::remove_file(&index_path).unwrap();
        let fetched = load_index(&dirs, &repository, true).unwrap();
        assert!(fetched.stale.is_some(), "the cached copy, marked stale");
        assert!(fetched.index.get("me.looks").is_some());
    }

    #[test]
    fn a_chosen_file_needs_a_vouching_index_or_a_yes() {
        let temp = tempfile::tempdir().unwrap();
        let source = effects_source(temp.path(), "me.side", "me.side-look", SHADER);
        let archive = temp.path().join("side.concatpkg");
        pack::pack(&source, &archive).unwrap();
        let dirs = AppDirs::under(&temp.path().join("app"));

        let inspected = inspect_file(&archive, &[]).unwrap();
        assert!(inspected.vouched_by.is_none());
        let error = install_file(
            &dirs,
            &archive,
            &[],
            false,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.contains("not vouched for"), "{error}");
        let record = install_file(
            &dirs,
            &archive,
            &[],
            true,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!(!record.origin.verified());

        // The same file listed by a signed index installs as verified.
        let (dirs2, _) = local_repository(temp.path(), &[source]);
        let (fetched, _) = load_all(&dirs2, false);
        let listed =
            file_path(&fetched[0].url_of(fetched[0].index.get("me.side").unwrap())).unwrap();
        let record = install_file(
            &dirs2,
            &listed,
            &fetched,
            false,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!(record.origin.verified());
    }

    #[test]
    fn fonts_and_stickers_are_vetted_by_their_contents() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(temp.path());
        let staged = temp.path().join("staged");
        std::fs::create_dir_all(staged.join("fonts")).unwrap();
        std::fs::create_dir_all(staged.join("stickers")).unwrap();
        std::fs::write(staged.join("fonts/Fake.ttf"), b"not a font").unwrap();
        std::fs::write(staged.join("stickers/star.png"), b"\x89PNG\r\n\x1a\nrest").unwrap();
        let mut manifest = concat_extras::Manifest::parse(
            "format = 1\n[extra]\nid = \"me.mixed\"\nname = \"Mixed\"\nversion = \"1.0.0\"\nauthor = \"Me\"\nlicence = \"MIT\"\n[[provides]]\nkind = \"stickers\"\nroot = \"stickers\"\n[[file]]\npath = \"stickers/star.png\"\nsize = 1\nsha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"\n",
        )
        .unwrap();
        vet(&dirs, &manifest, &staged).expect("a real PNG");
        std::fs::write(staged.join("stickers/star.png"), b"MZ\x90\x00 an exe").unwrap();
        assert!(
            vet(&dirs, &manifest, &staged)
                .unwrap_err()
                .contains("not the image")
        );
        manifest.provides[0] = concat_extras::manifest::Provide {
            kind: Kind::Fonts,
            root: "fonts".to_owned(),
        };
        assert!(
            vet(&dirs, &manifest, &staged)
                .unwrap_err()
                .contains("not a font")
        );
    }

    #[test]
    fn room_is_checked_against_the_disk() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(temp.path());
        assert!(free_space(temp.path()).is_some_and(|free| free > 0));
        check_room(&dirs, 1, 1).expect("a byte fits");
        let error = check_room(&dirs, u64::MAX / 4, u64::MAX / 4).unwrap_err();
        assert!(error.contains("not enough free space"), "{error}");
    }

    #[test]
    fn a_bad_repositories_file_is_reported_not_trusted() {
        let temp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::under(temp.path());
        std::fs::create_dir_all(repositories_file(&dirs).parent().unwrap()).unwrap();
        std::fs::write(
            repositories_file(&dirs),
            "[[repository]]\nname = \"NoKey\"\nindex = \"https://x/index.json\"\nkey = \"\"\n\n[[repository]]\nname = \"Plain\"\nindex = \"http://x/index.json\"\nkey = \"0000000000000000000000000000000000000000000000000000000000000000\"\n",
        )
        .unwrap();
        let (found, problems) = repositories(&dirs);
        assert!(found.is_empty());
        assert!(
            problems
                .iter()
                .any(|p| p.contains("NoKey has no valid key")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("Plain's index")),
            "{problems:?}"
        );
    }
}
