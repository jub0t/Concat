// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Putting a document's file on the disk so that no moment of the save can
//! leave the person without one.
//!
//! A project file is written whole, never edited in place, in this order:
//!
//! 1. **Written** to a file of its own beside the project's, so the rename
//!    at the end stays on one disk.
//! 2. **Flushed to the device**, not only to the system's cache: on Apple
//!    systems `F_FULLFSYNC`, since a plain `fsync` there stops at the
//!    drive's own cache; elsewhere the system's full flush
//!    (`FlushFileBuffers` on Windows).
//! 3. **Read back and compared** with what was meant: a disk that took the
//!    bytes and returns others is found before the old save is touched.
//! 4. **The save before it kept** as `<name>.prev`, so the project's own
//!    file is whole throughout: a second name for the same file where the
//!    disk has them, which copies nothing and needs no flush of its own
//!    (a flushed copy otherwise). Measured on an Apple M5 for a 1.8 MB
//!    project, the name takes 0.4 to 1 ms and the copy with its flush 2.4
//!    to 13.7 ms; on a spinning disk a flush is worse still.
//! 5. **Swapped in** with one rename, which a reader sees as all old or
//!    all new. A rename refused because another program holds the file -
//!    an antivirus, a sync client - is tried again for about two seconds;
//!    then the checked file is left as `<name>.new`, the save says so, and
//!    [`pending`] finds it at the next open.
//! 6. **The folder flushed**, where the system allows it, so the rename
//!    itself survives a power cut.
//!
//! [`Disk`] is the seam the crash tests cut at: each step can be made to
//! fail, and every failure leaves the project's file as the whole old save
//! or the whole new one.
//!
//! # A document
//!
//! [`save_document`] writes a [`Document`] as its file (see [`file`](mod@crate::file)) and
//! saves it so. A document is a snapshot that costs one pointer to hold,
//! so the save is handed a copy and runs where it likes - another thread
//! included - while edits go on against the current one: a save never
//! holds an edit up. [`open`] is the other end: it reads the project's
//! file, after putting in place a save that was left waiting.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::document::Document;
use crate::file::{self, Note, Opened, Refused};
use crate::model::Model;

/// How long a replace refused by another program is tried again.
pub const PATIENCE: Duration = Duration::from_secs(2);

/// How long between those tries.
const RETRY_EVERY: Duration = Duration::from_millis(100);

/// The steps of a save, in order: what a failed save names, and what the
/// crash tests fail one at a time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveStep {
    /// Writing the new file beside the project's.
    Write,
    /// Flushing it to the device.
    Flush,
    /// Reading it back.
    ReadBack,
    /// Copying the save before it to `.prev`.
    KeepPrevious,
    /// Renaming the new file over the project's.
    Replace,
    /// Flushing the folder, so the rename lasts.
    FlushFolder,
}

impl SaveStep {
    /// Every step, in the order a save takes them.
    pub const ALL: [SaveStep; 6] = [
        SaveStep::Write,
        SaveStep::Flush,
        SaveStep::ReadBack,
        SaveStep::KeepPrevious,
        SaveStep::Replace,
        SaveStep::FlushFolder,
    ];

    fn says(self) -> &'static str {
        match self {
            SaveStep::Write => "writing the new file",
            SaveStep::Flush => "flushing it to the disk",
            SaveStep::ReadBack => "reading it back",
            SaveStep::KeepPrevious => "keeping the save before it",
            SaveStep::Replace => "putting it in place",
            SaveStep::FlushFolder => "flushing the folder",
        }
    }
}

/// Why a save did not land. In every case the project's file is as it was
/// before the save began, except [`SaveError::FolderNotFlushed`], where the
/// new save is in place and only its lasting through a power cut is not
/// assured.
#[derive(Debug)]
pub enum SaveError {
    /// A step failed; nothing of this save is in place.
    Failed {
        /// The step.
        step: SaveStep,
        /// What the system said.
        error: io::Error,
    },
    /// The disk gave back other bytes than were written.
    ReadBackDiffers,
    /// Another program held the project's file past [`PATIENCE`]. The new
    /// save is whole, checked and waiting at `waiting`; [`pending`] finds it
    /// at the next open.
    Held {
        /// Where the checked save is.
        waiting: PathBuf,
        /// The last refusal.
        error: io::Error,
    },
    /// The save is in place, but the folder could not be flushed.
    FolderNotFlushed(io::Error),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Failed { step, error } => {
                write!(f, "the save stopped at {}: {error}", step.says())
            }
            SaveError::ReadBackDiffers => {
                f.write_str("the disk gave back other bytes than were written")
            }
            SaveError::Held { waiting, error } => write!(
                f,
                "another program holds the project's file ({error}); the save waits at {}",
                waiting.display()
            ),
            SaveError::FolderNotFlushed(error) => {
                write!(
                    f,
                    "the save is in place, but the folder could not be flushed: {error}"
                )
            }
        }
    }
}

impl std::error::Error for SaveError {}

/// What a save that landed did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Saved {
    /// The save before it was kept as `.prev`; false for a first save.
    pub kept_previous: bool,
    /// How many times the replace was refused before it went through.
    pub refusals: u32,
}

/// The project's file's companions: `<name>.prev`, `<name>.new`, and the
/// file a save is written to before it is swapped in.
pub fn companion(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// A save left waiting by [`SaveError::Held`], when there is one: the next
/// open reads it, and when it reads, [`adopt`]s it.
pub fn pending(path: &Path) -> Option<PathBuf> {
    let waiting = companion(path, ".new");
    waiting.is_file().then_some(waiting)
}

/// Puts a waiting save ([`pending`]) in place of the project's file, the
/// project's own kept as `.prev`, as a save would.
pub fn adopt(path: &Path) -> Result<Saved, SaveError> {
    adopt_on(&mut RealDisk, path)
}

/// Saves `bytes` as the file at `path`, in the steps the module describes.
pub fn save(path: &Path, bytes: &[u8]) -> Result<Saved, SaveError> {
    save_on(&mut RealDisk, path, bytes)
}

/// Saves a document as the file at `path`: written from the snapshot
/// given, then saved in the steps the module describes.
pub fn save_document(path: &Path, document: &Document<Model>) -> Result<Saved, SaveError> {
    save_document_on(&mut RealDisk, path, document)
}

/// [`save_document`] on `disk`.
pub fn save_document_on(
    disk: &mut dyn Disk,
    path: &Path,
    document: &Document<Model>,
) -> Result<Saved, SaveError> {
    save_on(disk, path, file::write(document).as_bytes())
}

/// Why a project could not be opened.
#[derive(Debug)]
pub enum OpenError {
    /// The file could not be read from the disk.
    Unread(io::Error),
    /// It was read, and is no project this build opens.
    Refused(Refused),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Unread(error) => write!(f, "the project's file could not be read: {error}"),
            OpenError::Refused(refused) => refused.fmt(f),
        }
    }
}

impl std::error::Error for OpenError {}

/// Opens the project at `path`.
///
/// A save left waiting beside it ([`SaveError::Held`]) is the newer, and
/// is opened in its place: put where the project's file is, the
/// project's own kept as `.prev`, if the disk lets it be; opened from
/// where it waits otherwise. Either way a note says so. A waiting save
/// that does not open is left where it is, and said.
pub fn open(path: &Path) -> Result<Opened, OpenError> {
    open_on(&mut RealDisk, path)
}

/// [`open`] on `disk`.
pub fn open_on(disk: &mut dyn Disk, path: &Path) -> Result<Opened, OpenError> {
    let waiting = companion(path, ".new");
    let mut said = Vec::new();
    if disk.exists(&waiting) {
        match disk.read(&waiting).map(|bytes| file::read(&bytes)) {
            Ok(Ok(mut opened)) => {
                let what = match adopt_on(disk, path) {
                    Ok(_) => "the last save had been left waiting beside the project; it is \
                              now the project's file"
                        .to_owned(),
                    Err(error) => format!(
                        "the last save is waiting beside the project and is opened from \
                         there; it could not be put in place ({error})"
                    ),
                };
                opened.notes.insert(
                    0,
                    Note {
                        place: String::new(),
                        what,
                    },
                );
                return Ok(opened);
            }
            Ok(Err(refused)) => said.push(Note {
                place: String::new(),
                what: format!(
                    "a save waiting beside the project at {} does not open ({refused}); it is \
                     left as it is",
                    waiting.display()
                ),
            }),
            Err(error) => said.push(Note {
                place: String::new(),
                what: format!(
                    "a save waiting beside the project at {} could not be read ({error}); it \
                     is left as it is",
                    waiting.display()
                ),
            }),
        }
    }
    let bytes = disk.read(path).map_err(OpenError::Unread)?;
    let mut opened = file::read(&bytes).map_err(OpenError::Refused)?;
    said.append(&mut opened.notes);
    opened.notes = said;
    Ok(opened)
}

/// The file operations a save is made of: the real disk, or a test's that
/// fails a chosen step.
pub trait Disk {
    /// Makes `path` (which does not exist) holding `bytes`.
    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Flushes the file at `path` to the device.
    fn flush(&mut self, path: &Path) -> io::Result<()>;
    /// The whole file at `path`.
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>>;
    /// Whether the file at `path` holds `bytes` and nothing else.
    fn holds(&mut self, path: &Path, bytes: &[u8]) -> io::Result<bool> {
        Ok(self.read(path)? == bytes)
    }
    /// Keeps the file at `from` as `to` as well, replacing `to`: lasting
    /// once the folder is flushed, and whole whatever happens to `from`
    /// after.
    fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    /// Renames `from` over `to` in one step.
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    /// Flushes the folder at `path`, so renames in it last.
    fn flush_folder(&mut self, path: &Path) -> io::Result<()>;
    /// Removes the file at `path`, if it is there.
    fn remove(&mut self, path: &Path);
    /// Whether a file is at `path`.
    fn exists(&mut self, path: &Path) -> bool;
    /// Waits before a refused replace is tried again.
    fn wait(&mut self, how_long: Duration);
}

/// [`save`] on `disk`.
pub fn save_on(disk: &mut dyn Disk, path: &Path, bytes: &[u8]) -> Result<Saved, SaveError> {
    let saving = companion(path, &format!(".saving-{}", std::process::id()));
    let failed = |step| move |error| SaveError::Failed { step, error };
    // A file a crashed save of this process left is no save of anyone's.
    disk.remove(&saving);
    if let Err(error) = disk.write_new(&saving, bytes) {
        disk.remove(&saving);
        return Err(failed(SaveStep::Write)(error));
    }
    let checked = disk
        .flush(&saving)
        .map_err(failed(SaveStep::Flush))
        .and_then(|()| {
            disk.holds(&saving, bytes)
                .map_err(failed(SaveStep::ReadBack))
        })
        .and_then(|same| {
            if same {
                Ok(())
            } else {
                Err(SaveError::ReadBackDiffers)
            }
        });
    if let Err(error) = checked {
        disk.remove(&saving);
        return Err(error);
    }
    swap_in(disk, &saving, path)
}

/// [`adopt`] on `disk`.
pub fn adopt_on(disk: &mut dyn Disk, path: &Path) -> Result<Saved, SaveError> {
    swap_in(disk, &companion(path, ".new"), path)
}

/// Steps 4 to 6: the save before kept, `ready` renamed over `path`, the
/// folder flushed. `ready` is whole and checked.
fn swap_in(disk: &mut dyn Disk, ready: &Path, path: &Path) -> Result<Saved, SaveError> {
    let kept_previous = disk.exists(path);
    if kept_previous && let Err(error) = disk.copy(path, &companion(path, ".prev")) {
        // The project's file is untouched; a save it could not back up
        // waits as `.new` rather than replace it unbacked.
        return Err(hold(disk, ready, path, SaveStep::KeepPrevious, error));
    }
    let began = Instant::now();
    let mut refusals = 0;
    loop {
        match disk.rename(ready, path) {
            Ok(()) => break,
            Err(error) if held_by_another(&error) && began.elapsed() < PATIENCE => {
                refusals += 1;
                disk.wait(RETRY_EVERY);
            }
            Err(error) => return Err(hold(disk, ready, path, SaveStep::Replace, error)),
        }
    }
    if let Some(folder) = path.parent() {
        disk.flush_folder(folder)
            .map_err(SaveError::FolderNotFlushed)?;
    }
    Ok(Saved {
        kept_previous,
        refusals,
    })
}

/// A save that could not be put in place: kept as `.new`, checked and
/// whole, for the next open.
fn hold(
    disk: &mut dyn Disk,
    ready: &Path,
    path: &Path,
    step: SaveStep,
    error: io::Error,
) -> SaveError {
    let waiting = companion(path, ".new");
    if ready == waiting {
        return SaveError::Held { waiting, error };
    }
    match disk.rename(ready, &waiting) {
        Ok(()) => SaveError::Held { waiting, error },
        Err(_) => {
            disk.remove(ready);
            SaveError::Failed { step, error }
        }
    }
}

/// A refusal that passes: another program has the file open. Windows says
/// so as a sharing violation (32), a lock violation (33) or access denied
/// (5) while the other handle lives.
fn held_by_another(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
        || cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33))
}

/// The disk itself.
struct RealDisk;

impl Disk for RealDisk {
    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.flush()
    }

    fn flush(&mut self, path: &Path) -> io::Result<()> {
        flush_file(&OpenOptions::new().write(true).open(path)?)
    }

    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// A piece at a time, so the file is never held twice in memory.
    fn holds(&mut self, path: &Path, mut bytes: &[u8]) -> io::Result<bool> {
        let mut file = File::open(path)?;
        let mut piece = vec![0; 1 << 16];
        loop {
            let read = file.read(&mut piece)?;
            if read == 0 {
                return Ok(bytes.is_empty());
            }
            match bytes.split_at_checked(read) {
                Some((head, rest)) if head == &piece[..read] => bytes = rest,
                _ => return Ok(false),
            }
        }
    }

    /// A second name for the same file, where the disk has them: no byte
    /// is copied and no second flush is made, and replacing `from` later
    /// leaves `to` as it is. A disk without them (FAT, some shares) gets
    /// a flushed copy.
    fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let linking = companion(to, ".linking");
        let _ = fs::remove_file(&linking);
        if fs::hard_link(from, &linking).is_ok() {
            match fs::rename(&linking, to) {
                Ok(()) => return Ok(()),
                Err(_) => {
                    let _ = fs::remove_file(&linking);
                }
            }
        }
        let copying = companion(to, ".copying");
        let _ = fs::remove_file(&copying);
        fs::copy(from, &copying)?;
        flush_file(&OpenOptions::new().write(true).open(&copying)?)?;
        fs::rename(&copying, to)
    }

    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn flush_folder(&mut self, path: &Path) -> io::Result<()> {
        // A folder cannot be opened as a file on Windows; its renames are
        // made lasting by the file system's own journal there.
        if cfg!(windows) {
            return Ok(());
        }
        File::open(path)?.sync_all()
    }

    fn remove(&mut self, path: &Path) {
        let _ = fs::remove_file(path);
    }

    fn exists(&mut self, path: &Path) -> bool {
        path.is_file()
    }

    fn wait(&mut self, how_long: Duration) {
        std::thread::sleep(how_long);
    }
}

/// Flushes `file` to the device: through the drive's own cache on macOS,
/// where `fsync` stops short of it.
fn flush_file(file: &File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // A disk that has no such flush - a network share, a FAT stick -
        // gets the system's own.
        rustix::fs::fcntl_fullfsync(file).or_else(|_| file.sync_all())
    }
    #[cfg(not(target_os = "macos"))]
    {
        file.sync_all()
    }
}
