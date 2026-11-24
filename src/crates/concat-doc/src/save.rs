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
//! 4. **The save before it kept** as `<name>.prev`, copied, so the
//!    project's own file is whole throughout.
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

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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

/// The file operations a save is made of: the real disk, or a test's that
/// fails a chosen step.
pub trait Disk {
    /// Makes `path` (which does not exist) holding `bytes`.
    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;
    /// Flushes the file at `path` to the device.
    fn flush(&mut self, path: &Path) -> io::Result<()>;
    /// The whole file at `path`.
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>>;
    /// Copies `from` to `to`, flushed, replacing `to`.
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
        .and_then(|()| disk.read(&saving).map_err(failed(SaveStep::ReadBack)))
        .and_then(|read| {
            if read == bytes {
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

    fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()> {
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

/// Flushes `file` to the device: through the drive's own cache on Apple
/// systems, where `fsync` stops short of it.
fn flush_file(file: &File) -> io::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        // A disk that has no such flush - a network share, a FAT stick -
        // gets the system's own.
        rustix::fs::fcntl_fullfsync(file).or_else(|_| file.sync_all())
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        file.sync_all()
    }
}
