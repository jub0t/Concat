// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! One process at a time on a project folder: the window, `concat --mcp`,
//! `concat-cli`, a second window.
//!
//! Two processes editing one folder each save their own document over the
//! other's, and the last save wins whatever it lost. So a process that
//! opens a project takes `concat.lock` in its folder: a file it holds an
//! exclusive operating-system lock on (`flock` on Unix, `LockFileEx` on
//! Windows, through [`std::fs::File::try_lock`]), with its process id, its
//! role and when it started written inside for whoever is refused.
//!
//! The lock is the operating system's, not the file's: it lives exactly
//! as long as the process's handle, so a process that crashes or is killed
//! lets go with it, and the file it leaves behind is taken over by the
//! next opener without anybody deciding whether its pid is still running -
//! a question a reused pid answers wrongly. The words in the file are only
//! what a refusal says.
//!
//! Within one process the window and the API it serves share
//! `concat_api::OpenProjects`, which holds one of these per folder; this
//! is the line between processes.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The lock file's name in a project folder.
pub const FILE: &str = "concat.lock";

/// How many times an open races a holder that is letting go - the file
/// removed between this process opening it and locking it - before it
/// gives up.
const ATTEMPTS: usize = 4;

/// A project folder held by this process; let go when dropped.
#[derive(Debug)]
pub struct ProjectLock {
    file: Option<File>,
    path: PathBuf,
}

/// Who holds a folder, as the lock file says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    /// The holding process, when the file names one.
    pub pid: Option<u32>,
    /// What it is: "window", "api", or empty when the file does not say.
    pub role: String,
}

/// Why a folder could not be taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockError {
    /// Another process holds it.
    Held(Owner),
    /// The lock file could not be made or locked: a folder that cannot be
    /// written, a file system without locks.
    Io(String),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Held(owner) => match owner.pid {
                Some(pid) => write!(f, "open in another Concat (process {pid})"),
                None => f.write_str("open in another Concat"),
            },
            LockError::Io(why) => f.write_str(why),
        }
    }
}

impl ProjectLock {
    /// Takes `folder` for this process, as `role`. Refused with the owner
    /// when another live process holds it; a lock file left by a process
    /// that is gone is taken over.
    pub fn acquire(folder: &Path, role: &str) -> Result<ProjectLock, LockError> {
        let path = folder.join(FILE);
        let could_not =
            |error: &dyn std::fmt::Display| LockError::Io(format!("{}: {error}", path.display()));
        for attempt in 1..=ATTEMPTS {
            let opened = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path);
            let mut file = match opened {
                Ok(file) => file,
                // On Windows a file being removed cannot be opened until
                // its last handle closes, which is a moment away.
                Err(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        && attempt < ATTEMPTS =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    continue;
                }
                Err(error) => return Err(could_not(&error)),
            };
            match file.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => return Err(LockError::Held(owner_in(&mut file))),
                Err(TryLockError::Error(error)) => return Err(could_not(&error)),
            }
            // A holder letting go removes the file after its last write; a
            // lock taken on the file it removed guards nothing. Locked, the
            // path must still be this file, or the open starts over.
            if !same_file(&file, &path) {
                continue;
            }
            let started = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_millis())
                .unwrap_or(0);
            let text = format!("{}\n{role}\n{started}\n", std::process::id());
            file.set_len(0)
                .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
                .and_then(|()| file.write_all(text.as_bytes()))
                .and_then(|()| file.flush())
                .map_err(|error| could_not(&error))?;
            return Ok(ProjectLock {
                file: Some(file),
                path,
            });
        }
        Err(could_not(&"another process kept taking and letting go of it"))
    }

    /// The lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Who holds `folder`, if a live process does: the owner its lock file
    /// names. `None` when nobody does, or when this process does.
    pub fn holder(folder: &Path) -> Option<Owner> {
        let path = folder.join(FILE);
        let mut file = File::open(&path).ok()?;
        match file.try_lock_shared() {
            Ok(()) => None,
            Err(TryLockError::WouldBlock) => {
                let owner = owner_in(&mut file);
                (owner.pid != Some(std::process::id())).then_some(owner)
            }
            Err(TryLockError::Error(_)) => None,
        }
    }
}

impl Drop for ProjectLock {
    /// The file goes first, then the lock with the handle: a process that
    /// opened the file in between finds it gone once it has the lock, and
    /// starts over on a new one.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if let Some(file) = self.file.take() {
            let _ = file.unlock();
        }
    }
}

/// The owner a lock file names.
fn owner_in(file: &mut File) -> Owner {
    let mut text = String::new();
    let _ = file.seek(SeekFrom::Start(0));
    let _ = file.read_to_string(&mut text);
    let mut lines = text.lines();
    Owner {
        pid: lines.next().and_then(|line| line.trim().parse().ok()),
        role: lines.next().unwrap_or_default().trim().to_owned(),
    }
}

/// Whether `path` is still the file `file` has open.
#[cfg(unix)]
fn same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (file.metadata(), std::fs::metadata(path)) {
        (Ok(held), Ok(named)) => held.dev() == named.dev() && held.ino() == named.ino(),
        _ => false,
    }
}

/// Whether `path` is still the file `file` has open. On Windows a file
/// another process is removing cannot be opened at all until it is gone,
/// so a file that opened and locked is the one at the path.
#[cfg(not(unix))]
fn same_file(_file: &File, path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn a_lock_is_taken_named_and_given_back() {
        let scratch = tempfile::tempdir().expect("scratch");
        let lock = ProjectLock::acquire(scratch.path(), "window").expect("taken");
        let text = std::fs::read_to_string(lock.path()).expect("written");
        assert!(text.starts_with(&format!("{}\nwindow\n", std::process::id())));
        assert_eq!(ProjectLock::holder(scratch.path()), None, "ours is not another's");
        drop(lock);
        assert!(!scratch.path().join(FILE).exists(), "the file goes with it");
        let again = ProjectLock::acquire(scratch.path(), "api").expect("free again");
        drop(again);
    }

    #[test]
    fn a_file_left_by_a_process_that_died_is_taken_over() {
        let scratch = tempfile::tempdir().expect("scratch");
        // A crash leaves the file and its words, and no lock.
        std::fs::write(scratch.path().join(FILE), "999999\nwindow\n0\n").expect("written");
        assert_eq!(ProjectLock::holder(scratch.path()), None);
        let lock = ProjectLock::acquire(scratch.path(), "api").expect("taken over");
        let text = std::fs::read_to_string(lock.path()).expect("rewritten");
        assert!(text.contains("\napi\n"), "{text}");
    }

    /// Another process holds the folder: this test's own binary, run as a
    /// child that takes the lock and waits for its input to close.
    #[test]
    fn a_second_process_is_refused_and_told_who_holds_it() {
        if let Ok(folder) = std::env::var("CONCAT_LOCK_CHILD") {
            let _lock = ProjectLock::acquire(Path::new(&folder), "window").expect("child takes it");
            println!("held");
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            return;
        }
        let scratch = tempfile::tempdir().expect("scratch");
        let mut child = Command::new(std::env::current_exe().expect("this test"))
            .args([
                "--exact",
                "lock::tests::a_second_process_is_refused_and_told_who_holds_it",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("CONCAT_LOCK_CHILD", scratch.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("a child");
        let mut out = std::io::BufReader::new(child.stdout.take().expect("stdout"));
        let mut line = String::new();
        loop {
            line.clear();
            let read = std::io::BufRead::read_line(&mut out, &mut line).expect("reads");
            assert!(read > 0, "the child never held it");
            // The harness prints the test's name on the same line first.
            if line.trim_end().ends_with("held") {
                break;
            }
        }
        let refused = ProjectLock::acquire(scratch.path(), "api").expect_err("held elsewhere");
        let owner = Owner {
            pid: Some(child.id()),
            role: "window".to_owned(),
        };
        assert_eq!(refused, LockError::Held(owner.clone()));
        assert_eq!(ProjectLock::holder(scratch.path()), Some(owner));
        // The child goes - as a crash would - and the folder is free.
        let _ = child.kill();
        let _ = child.wait();
        let lock = ProjectLock::acquire(scratch.path(), "api").expect("free once it is gone");
        drop(lock);
    }
}
