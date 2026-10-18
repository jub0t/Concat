// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Project folders on disk, and the list of recently opened ones.
//!
//! A project is a directory containing `concat.json`. That file is written the
//! moment the project is created, so a project is a real thing from the start
//! rather than a promise the app keeps only if you remember to save.
//!
//! The recents list lives in the app's config directory, not in any project.
//! It describes this machine's history, and copying a project folder to
//! another machine should not drag one person's recent files along with it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MANIFEST: &str = "concat.json";
/// The manifest's earlier names, newest first. Projects created before a
/// rename keep theirs forever: the manifest is read and written under the
/// name it has, and only brand-new projects get the current one. A rename
/// must never orphan an edit.
const LEGACY_MANIFESTS: &[&str] = &["wolfcut.json"];

/// The manifest file this project actually uses: the current name, a legacy
/// name if that is what exists, the current name for a fresh folder.
pub fn manifest_path(root: &Path) -> PathBuf {
    let current = root.join(MANIFEST);
    if current.is_file() {
        return current;
    }
    for legacy in LEGACY_MANIFESTS {
        let candidate = root.join(legacy);
        if candidate.is_file() {
            return candidate;
        }
    }
    current
}

/// Whether `root` holds a project: any manifest, current or legacy.
pub fn is_project(root: &Path) -> bool {
    manifest_path(root).is_file()
}

const RECENTS: &str = "recents.json";
/// Long enough to be a working set of projects, short enough that the
/// launch screen's grid stays scannable - and past a page of it, the screen
/// has a search and a sort to find one by.
const MAX_RECENTS: usize = 40;

/// Everything needed to reopen a project.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// The project folder.
    pub path: String,
    /// The name shown in the title bar and the recents list.
    pub name: String,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Frame rate as an exact fraction; the decimal is never stored.
    pub rate_num: i64,
    /// Denominator of the frame rate.
    pub rate_den: i64,
    /// Milliseconds since the epoch.
    pub opened_at: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    /// The version of the app that wrote the file, under whichever name the
    /// app had at the time. Informational: nothing reads it back.
    #[serde(alias = "wolfcut")]
    concat: String,
    name: String,
    video: Video,
    /// Everything else in the file is ignored when reading settings for the
    /// recents list. `flatten` keeps it from being dropped on a rewrite.
    #[serde(flatten)]
    #[allow(dead_code)]
    rest: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Video {
    width: u32,
    height: u32,
    rate_num: i64,
    rate_den: i64,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Creates the project folder and writes its manifest.
///
/// Refuses to touch a folder that already holds a manifest. Overwriting
/// someone's edit because they reused a name is not a recoverable mistake.
pub fn create(
    location: &str,
    name: &str,
    width: u32,
    height: u32,
    rate_num: i64,
    rate_den: i64,
) -> Result<ProjectInfo, String> {
    let root = Path::new(location).join(folder_name(name));
    let manifest = root.join(MANIFEST);

    // Any name counts as an existing project - a legacy folder must be
    // just as safe from being clobbered as a new one.
    if is_project(&root) {
        return Err(format!(
            "a Concat project already exists at {}",
            root.display()
        ));
    }

    std::fs::create_dir_all(&root)
        .map_err(|error| format!("could not create {}: {error}", root.display()))?;

    // Settings only: a fresh project has no edit yet. The full document -
    // timelines included - is written by the session's save from the first
    // change.
    let document = Manifest {
        concat: env!("CARGO_PKG_VERSION").to_owned(),
        name: name.to_owned(),
        video: Video {
            width,
            height,
            rate_num,
            rate_den,
        },
        rest: serde_json::Map::new(),
    };

    let encoded = serde_json::to_vec_pretty(&document)
        .map_err(|error| format!("could not encode the manifest: {error}"))?;
    std::fs::write(&manifest, encoded)
        .map_err(|error| format!("could not write {}: {error}", manifest.display()))?;

    Ok(ProjectInfo {
        path: root.to_string_lossy().into_owned(),
        name: name.to_owned(),
        width,
        height,
        rate_num,
        rate_den,
        opened_at: now_millis(),
    })
}

/// A snapshot's place in this process's write order.
///
/// Minted by [`ticket`] where a document is captured - on the thread that
/// captures it, before the write is handed to a worker - and carried to
/// [`save_ordered`], where the write happens. Two snapshots of one project
/// are written in ticket order, so a worker that finishes late cannot put an
/// older document back on disk after a newer one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticket(u64);

/// Mints the next ticket; see [`Ticket`].
///
/// The counter is process-wide, so tickets are monotonic for every project,
/// and therefore for each project on its own. It is not shared with another
/// process; see [`save_ordered`] for what that leaves unordered.
pub fn ticket() -> Ticket {
    Ticket(NEXT_TICKET.fetch_add(1, Ordering::Relaxed))
}

static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);

/// What an ordered save did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    /// The document is on disk.
    Written,
    /// A snapshot minted after this one was already written, so this older
    /// document was left alone rather than put back on disk. Nothing went
    /// wrong: the project is saved, to the newer document.
    Superseded,
}

/// One project's writes: the lock that runs them one at a time, and the
/// highest ticket already on disk.
struct Writes {
    /// Held for a whole write - the temporary file, its flush and the
    /// rename - so two workers never write one manifest at once.
    writing: Mutex<()>,
    /// The highest ticket whose document reached the manifest. Read and
    /// written under `writing`, so the check that skips an older snapshot
    /// and the write that follows it cannot be split by another writer.
    written: AtomicU64,
}

/// The per-project write state, keyed by the folder as the filesystem
/// spells it; see [`project_key`].
fn writes() -> &'static Mutex<HashMap<PathBuf, Arc<Writes>>> {
    static WRITES: OnceLock<Mutex<HashMap<PathBuf, Arc<Writes>>>> = OnceLock::new();
    WRITES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// One key per project folder: the real path where the filesystem will say
/// it, so `/var` and `/private/var`, a trailing slash, a symlinked parent
/// and a different case on a case-insensitive volume all name one project.
/// The path as it was given when it cannot be resolved, which still names
/// the folder it was given.
fn project_key(root: &Path) -> PathBuf {
    std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

/// The write state for one project folder.
fn write_state(key: &Path) -> Arc<Writes> {
    let mut all = writes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(all.entry(key.to_path_buf()).or_insert_with(|| {
        Arc::new(Writes {
            writing: Mutex::new(()),
            written: AtomicU64::new(0),
        })
    }))
}

/// Writes the whole project document to the project's manifest, now.
///
/// The document is passed through as opaque JSON rather than being mirrored
/// into Rust types: the engine (`concat-project`) owns the canonical model and
/// produced this document, and the host has no decisions to make about its
/// contents beyond writing it safely.
///
/// The write is ordered like [`save_ordered`], with its ticket minted here: a
/// caller that captured the document and writes it at once - the API, a
/// template - is in call order already. A caller that captures a document on
/// one thread and writes it on another (the window's autosave) mints the
/// ticket where it captures the document and calls [`save_ordered`] instead.
pub fn save(path: &str, document: &serde_json::Value) -> Result<(), String> {
    save_ordered(path, document, ticket()).map(|_| ())
}

/// Writes a document captured at `ticket`, in this process's write order.
///
/// A snapshot older than the one already on disk is skipped whole - see
/// [`Saved`] - so a worker that finishes late cannot put an older document
/// back over a newer one, whatever order the two are scheduled in. A write
/// that fails is always a failure: a document that should have been written
/// and could not returns the error, never [`Saved::Superseded`].
///
/// The temporary file is this call's own (`create_new`), so two saves -
/// including ones in different processes on the same folder - never write one
/// another's bytes, and the rename over the manifest is atomic on a local
/// disk (not always on FAT or a network share, where it is still one call
/// that replaces or fails). What a folder shared between processes does not get is the
/// order: each process counts tickets on its own, so a stale snapshot in one
/// process can still land after a newer document written by another.
/// Ordering across processes would need a lock both honour, and a project
/// folder has none today.
pub fn save_ordered(
    path: &str,
    document: &serde_json::Value,
    ticket: Ticket,
) -> Result<Saved, String> {
    let root = PathBuf::from(path);
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("could not create {}: {error}", root.display()))?;

    let state = write_state(&project_key(&root));
    // Held across the write: the check below and the write it decides are
    // one step, so two saves cannot both decide to write and have the older
    // one land second.
    let _writing = state
        .writing
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if ticket.0 < state.written.load(Ordering::Acquire) {
        return Ok(Saved::Superseded);
    }

    let encoded = serde_json::to_vec_pretty(document)
        .map_err(|error| format!("could not encode the project: {error}"))?;
    write_manifest(&root, &encoded)?;
    state.written.store(ticket.0, Ordering::Release);
    Ok(Saved::Written)
}

/// The temporary file a save writes: flushed and renamed into place on the
/// way through, removed on every other way out, so a failure leaves no
/// `.saving` file behind. The name is this call's own - process id and a
/// counter - so no two saves share it.
struct Temporary {
    path: PathBuf,
    file: Option<std::fs::File>,
    renamed: bool,
}

impl Temporary {
    /// Creates a fresh temporary beside the manifest. `create_new` never
    /// reuses or follows a name already there, so it cannot write through a
    /// symlink someone planted and fails rather than sharing a temporary
    /// with another save; a leftover from a crashed run makes it try another
    /// name.
    fn open(root: &Path) -> Result<Temporary, String> {
        let mut attempts = 0;
        loop {
            let nonce = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!("{MANIFEST}.saving.{}.{nonce}", std::process::id()));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Temporary {
                        path,
                        file: Some(file),
                        renamed: false,
                    });
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::AlreadyExists && attempts < 16 =>
                {
                    attempts += 1;
                }
                Err(error) => return Err(format!("could not write {}: {error}", path.display())),
            }
        }
    }

    /// Writes `encoded` and flushes its contents before the atomic rename.
    /// Atomic replacement and durability are distinct: flushing the file
    /// reduces power-loss risk, but does not also sync the parent directory.
    fn write(&mut self, encoded: &[u8]) -> Result<(), String> {
        let file = self.file.as_mut().expect("a fresh temporary");
        std::io::Write::write_all(file, encoded)
            .map_err(|error| format!("could not write {}: {error}", self.path.display()))?;
        file.sync_all()
            .map_err(|error| format!("could not flush {}: {error}", self.path.display()))
    }

    /// Closes the file and renames it over the manifest. Every path out but
    /// this one removes the temporary when the value drops.
    fn commit(&mut self, manifest: &Path) -> Result<(), String> {
        // Closed first, so the name is free for Windows to replace.
        self.file.take();
        // A scanner, a sync client or a backup tool holding the manifest a
        // moment makes Windows refuse the replace (access denied, sharing
        // violation). That passes; a close that failed on it would not.
        let mut wait = std::time::Duration::from_millis(50);
        let mut tries = 0;
        loop {
            match std::fs::rename(&self.path, manifest) {
                Ok(()) => break,
                Err(error) if tries < 5 && transient(&error) => {
                    tries += 1;
                    std::thread::sleep(wait);
                    wait *= 2;
                }
                Err(error) => {
                    return Err(format!("could not replace {}: {error}", manifest.display()));
                }
            }
        }
        self.renamed = true;
        Ok(())
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        // The handle goes first: Windows will not take a file back while it
        // is open.
        self.file.take();
        if !self.renamed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Whether a failed replace is another program holding the file for a
/// moment: on Windows, access denied (5) or a sharing violation (32).
/// Elsewhere a rename is not refused that way, so nothing is retried.
fn transient(error: &std::io::Error) -> bool {
    cfg!(windows)
        && (error.kind() == std::io::ErrorKind::PermissionDenied
            || matches!(error.raw_os_error(), Some(5 | 32)))
}

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// Writes `encoded` to the project's manifest: a temporary of its own,
/// flushed to the disk, then renamed over the manifest, because a save
/// interrupted halfway is worse than no save at all - a truncated manifest
/// loses the project, while a failed rename leaves the previous one intact.
fn write_manifest(root: &Path, encoded: &[u8]) -> Result<(), String> {
    let manifest = manifest_path(root);
    let mut temporary = Temporary::open(root)?;
    temporary.write(encoded)?;
    temporary.commit(&manifest)
}

/// True when a document carries no edit state at all - the settings-only
/// manifest `create` writes, in a project closed before its first save.
/// Such a document is a legitimately empty project. A document that claims
/// edit state the build then cannot load is a different thing: corrupt.
pub fn is_settings_only(document: &serde_json::Value) -> bool {
    document.is_object()
        && ["timelines", "tracks", "clips", "media"]
            .iter()
            .all(|key| document.get(key).is_none())
}

/// Reads the whole project document back: `None` when the folder has no
/// document yet, an error for one that is there and cannot be read or
/// parsed. The two are kept apart because a caller that treats them alike
/// opens a half-written document as an empty project and saves over it
/// (audit 2026-09-23, #1).
pub fn read_document(path: &str) -> Result<Option<serde_json::Value>, String> {
    let manifest = manifest_path(Path::new(path));
    let bytes = match std::fs::read(&manifest) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read {}: {error}", manifest.display())),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("{} is not a Concat project: {error}", manifest.display()))
}

/// Reads an existing project's settings.
pub fn open(path: &str) -> Result<ProjectInfo, String> {
    let root = PathBuf::from(path);
    let manifest = manifest_path(&root);

    let bytes = std::fs::read(&manifest)
        .map_err(|error| format!("could not read {}: {error}", manifest.display()))?;
    let document: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not a Concat project: {error}", manifest.display()))?;

    if document.video.rate_den == 0 {
        return Err(format!("{} has an invalid frame rate", manifest.display()));
    }

    Ok(ProjectInfo {
        path: root.to_string_lossy().into_owned(),
        name: document.name,
        width: document.video.width,
        height: document.video.height,
        rate_num: document.video.rate_num,
        rate_den: document.video.rate_den,
        opened_at: now_millis(),
    })
}

/// Moves a project to the front of the recents list.
pub fn remember(config: &Path, project: &ProjectInfo) -> Result<(), String> {
    let mut entries = read_recents(config);
    entries.retain(|entry| !same_path(&entry.path, &project.path));
    entries.insert(0, project.clone());
    entries.truncate(MAX_RECENTS);
    write_recents(config, &entries)
}

/// The recents list, most recent first, with vanished folders dropped.
///
/// Filtering on read rather than pruning on write means a project on a drive
/// that happens to be unplugged comes back when the drive does.
pub fn list(config: &Path) -> Vec<ProjectInfo> {
    read_recents(config)
        .into_iter()
        .filter(|entry| is_project(Path::new(&entry.path)))
        .collect()
}

/// One row of the recents list as the launch screen shows it.
#[derive(Clone, Debug)]
pub struct Recent {
    /// What the list remembers of it.
    pub info: ProjectInfo,
    /// The folder is there and still holds a project. One on a drive that
    /// is unplugged is not, and the launch screen shows it as missing
    /// rather than dropping it, so the person sees where it went and can
    /// forget it - and it comes back by itself when the drive does.
    pub present: bool,
    /// When the project was last written, in milliseconds since the epoch:
    /// the manifest's own time, which every autosave moves. None for a
    /// missing folder, or a file system that keeps no times.
    pub edited_at: Option<u64>,
}

/// The recents list, most recent first, missing folders included and
/// marked. One stat per project, so it is read when the list changes and
/// not on every frame.
pub fn recents(config: &Path) -> Vec<Recent> {
    read_recents(config)
        .into_iter()
        .map(|info| {
            let manifest = manifest_path(Path::new(&info.path));
            let meta = std::fs::metadata(&manifest)
                .ok()
                .filter(|meta| meta.is_file());
            let edited_at = meta
                .as_ref()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|elapsed| elapsed.as_millis() as u64);
            Recent {
                present: meta.is_some(),
                edited_at,
                info,
            }
        })
        .collect()
}

/// Removes one project from the list. The folder itself is left alone.
pub fn forget(config: &Path, path: &str) -> Result<(), String> {
    let mut entries = read_recents(config);
    entries.retain(|entry| !same_path(&entry.path, path));
    write_recents(config, &entries)
}

fn read_recents(config: &Path) -> Vec<ProjectInfo> {
    // A missing or corrupt list is not an error worth showing anyone; it just
    // means there is no history yet.
    std::fs::read(config.join(RECENTS))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_recents(config: &Path, entries: &[ProjectInfo]) -> Result<(), String> {
    std::fs::create_dir_all(config)
        .map_err(|error| format!("could not create {}: {error}", config.display()))?;

    let encoded = serde_json::to_vec_pretty(entries)
        .map_err(|error| format!("could not encode the recents list: {error}"))?;
    std::fs::write(config.join(RECENTS), encoded)
        .map_err(|error| format!("could not write the recents list: {error}"))
}

/// Compares paths case-insensitively, because Windows does, and on Windows
/// with either slash, because a path typed or joined with `/` names the same
/// folder as the one the file dialog hands back with `\`. A trailing
/// separator names the same folder too.
fn same_path(left: &str, right: &str) -> bool {
    fn spelled(path: &str) -> String {
        let path = if cfg!(windows) {
            path.replace('/', "\\")
        } else {
            path.to_owned()
        };
        path.trim_end_matches(['/', '\\']).to_owned()
    }
    spelled(left).eq_ignore_ascii_case(&spelled(right))
}

/// Turns a project name into something a filesystem will accept. Also used
/// for template bundle folders and the files inside them.
pub fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() || r#"<>:"/\|?*"#.contains(character) {
                '-'
            } else {
                character
            }
        })
        .collect();

    // Windows refuses names ending in a dot or a space.
    let trimmed = cleaned.trim_matches(['.', ' ']);
    if trimmed.is_empty() {
        "Untitled project".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Removes all cached artwork and waveforms from a project folder.
///
/// The cache is entirely regenerable, so the whole folder goes, subfolders
/// (`art/`, peaks and all) included. `remove_file` alone would skip them:
/// it refuses directories.
pub fn clear_cache(path: &str) -> Result<usize, String> {
    let cache_dir = Path::new(path).join("cache");
    if !cache_dir.exists() {
        return Ok(0);
    }
    let count = count_files(&cache_dir);
    std::fs::remove_dir_all(&cache_dir)
        .map_err(|error| format!("cannot remove {}: {error}", cache_dir.display()))?;
    Ok(count)
}

/// Recursively counts the files under a directory, for the clear-cache toast.
fn count_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() { count_files(&path) } else { 1 }
        })
        .sum()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitises_names_windows_would_reject() {
        assert_eq!(folder_name("My: Project?"), "My- Project-");
        assert_eq!(folder_name("  trailing.  "), "trailing");
        assert_eq!(folder_name("..."), "Untitled project");
        assert_eq!(folder_name(""), "Untitled project");
    }

    #[test]
    fn a_legacy_manifest_keeps_its_name() {
        for legacy in LEGACY_MANIFESTS {
            let scratch = std::env::temp_dir().join(format!("concat-legacy-{legacy}-test"));
            let _ = std::fs::remove_dir_all(&scratch);
            std::fs::create_dir_all(&scratch).expect("scratch dir");
            // A fresh folder resolves to the current name...
            assert!(manifest_path(&scratch).ends_with(MANIFEST));
            assert!(!is_project(&scratch));
            // ...a pre-rename project keeps the manifest it has, forever...
            std::fs::write(scratch.join(legacy), b"{}").expect("writes");
            assert!(manifest_path(&scratch).ends_with(legacy));
            assert!(is_project(&scratch));
            // ...and the current name wins only where it actually exists.
            std::fs::write(scratch.join(MANIFEST), b"{}").expect("writes");
            assert!(manifest_path(&scratch).ends_with(MANIFEST));
            let _ = std::fs::remove_dir_all(&scratch);
        }
    }

    #[test]
    fn a_manifest_under_any_of_its_names_opens() {
        let scratch = std::env::temp_dir().join("concat-manifest-key-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        for key in ["concat", "wolfcut"] {
            let document = serde_json::json!({
                key: "0.2.0",
                "name": "Old",
                "video": { "width": 1920, "height": 1080, "rateNum": 30, "rateDen": 1 },
            });
            std::fs::write(
                scratch.join(MANIFEST),
                serde_json::to_vec(&document).unwrap(),
            )
            .expect("writes");
            let info = open(&scratch.to_string_lossy()).expect("opens");
            assert_eq!((info.width, info.height, info.rate_num), (1920, 1080, 30));
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_fresh_manifest_is_settings_only_and_a_saved_edit_is_not() {
        // Exactly what `create` writes: settings, no edit state.
        let fresh = serde_json::json!({
            "concat": "0.2.0",
            "name": "Untitled project",
            "video": { "width": 1080, "height": 1920, "rateNum": 30, "rateDen": 1 },
        });
        assert!(is_settings_only(&fresh));
        // Any edit state present means the document must load or fail loudly.
        assert!(!is_settings_only(&serde_json::json!({ "tracks": [] })));
        assert!(!is_settings_only(
            &serde_json::json!({ "timelines": "garbage" })
        ));
        assert!(!is_settings_only(&serde_json::json!("nonsense")));
    }

    #[test]
    fn paths_compare_case_insensitively() {
        assert!(same_path("D:\\Work\\Film", "d:\\work\\film"));
        assert!(!same_path("D:\\Work\\Film", "D:\\Work\\Other"));
        assert!(same_path("/work/film/", "/work/film"));
    }

    #[cfg(windows)]
    #[test]
    fn either_slash_names_the_same_folder_on_windows() {
        assert!(same_path(
            "C:\\Temp\\.tmp1\\Round trip",
            "C:\\Temp\\.tmp1/Round trip"
        ));
        assert!(same_path("C:/Work/Film/", "c:\\work\\film"));
    }

    /// A project folder of this test's own, swept if it is already there.
    fn scratch(name: &str) -> String {
        let root =
            std::env::temp_dir().join(format!("concat-projects-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch dir");
        root.to_string_lossy().into_owned()
    }

    /// A whole document, as a save writes one.
    fn document(marker: &str) -> serde_json::Value {
        serde_json::json!({ "marker": marker, "tracks": [] })
    }

    /// How many temporary files a folder holds: none, after any save.
    fn temporaries(root: &str) -> usize {
        std::fs::read_dir(root)
            .expect("reads the scratch")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{MANIFEST}.saving"))
            })
            .count()
    }

    #[test]
    fn a_late_snapshot_never_lands_after_a_newer_one() {
        let root = scratch("late-snapshot");
        let older = document("older");
        let newer = document("newer");
        // Minted where each document is captured: the older one first.
        let older_ticket = ticket();
        let newer_ticket = ticket();
        // The newer worker finishes first, the older one arrives after it -
        // the order two workers on the pool can take.
        assert_eq!(
            save_ordered(&root, &newer, newer_ticket).expect("writes"),
            Saved::Written
        );
        assert_eq!(
            save_ordered(&root, &older, older_ticket).expect("skips, it does not fail"),
            Saved::Superseded
        );
        assert_eq!(
            read_document(&root).expect("reads"),
            Some(newer),
            "the newer document stands"
        );
        assert_eq!(temporaries(&root), 0, "no temporary is left behind");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn concurrent_saves_keep_the_document_readable() {
        let root = scratch("concurrent");
        const SAVES: usize = 8;
        let documents: Vec<serde_json::Value> = (0..SAVES)
            .map(|index| document(&format!("doc-{index}")))
            .collect();
        let tickets: Vec<Ticket> = (0..SAVES).map(|_| ticket()).collect();
        std::thread::scope(|scope| {
            for (document, ticket) in documents.iter().zip(&tickets) {
                let root = root.clone();
                scope.spawn(move || {
                    save_ordered(&root, document, *ticket).expect("a save that cannot fail here");
                });
            }
        });
        assert_eq!(
            read_document(&root).expect("reads"),
            Some(documents[SAVES - 1].clone()),
            "the last ticket's document stands, whatever order the workers took"
        );
        assert_eq!(temporaries(&root), 0, "no temporary is left behind");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_older_ticket_is_skipped_and_a_real_failure_is_not() {
        let root = scratch("skip-or-fail");
        let first = ticket();
        let second = ticket();
        save_ordered(&root, &document("one"), first).expect("writes");
        save_ordered(&root, &document("two"), second).expect("writes");
        // An older ticket is a skip, told apart from a failure.
        assert_eq!(
            save_ordered(&root, &document("stale"), Ticket(first.0)).expect("not an error"),
            Saved::Superseded
        );
        assert_eq!(read_document(&root).expect("reads"), Some(document("two")));

        // A write that truly cannot happen is an error, never a skip.
        let blocked = scratch("skip-or-fail-blocked");
        let manifest = Path::new(&blocked).join(MANIFEST);
        std::fs::create_dir(&manifest).expect("the manifest spot");
        std::fs::write(manifest.join("kept"), b"kept").expect("something worth keeping");
        let error = save_ordered(&blocked, &document("three"), ticket())
            .expect_err("cannot replace a directory");
        assert!(!error.is_empty());
        assert!(
            manifest.join("kept").exists(),
            "what was there is untouched"
        );
        assert_eq!(
            temporaries(&blocked),
            0,
            "the temporary is cleaned up on failure"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&blocked);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_save_keeps_the_document_it_had() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("keeps-document");
        let saved = document("saved");
        save(&root, &saved).expect("first save");
        let manifest = Path::new(&root).join(MANIFEST);
        let before = std::fs::read(&manifest).expect("reads");

        // A folder that refuses new files: the temporary cannot be created.
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555))
            .expect("read-only folder");
        let probe = Path::new(&root).join("probe");
        if std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .is_ok()
        {
            // A privileged account ignores the permission; nothing to test.
            let _ = std::fs::remove_file(&probe);
            let _ = std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755));
            let _ = std::fs::remove_dir_all(&root);
            return;
        }

        let error = save(&root, &document("next")).expect_err("the folder refuses a temporary");
        assert!(!error.is_empty());
        assert_eq!(
            std::fs::read(&manifest).expect("reads"),
            before,
            "the document that was saved stands"
        );
        assert_eq!(temporaries(&root), 0, "no temporary is left behind");

        let _ = std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&root);
    }
}
