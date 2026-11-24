// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A save, cut at every step: by an error the save sees, and by the
//! process dying there. Whatever happens, the project's file is the whole
//! old save or the whole new one.

use std::collections::BTreeMap;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::Duration;

use concat_doc::save::{self, Disk, SaveError, SaveStep, Saved, companion};

const OLD: &[u8] = b"{\"old\":true}";
const NEW: &[u8] = b"{\"new\":true,\"longer\":\"than the old one\"}";

/// How the fake disk fails a step.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Cut {
    /// The step returns an error.
    Error,
    /// The process dies inside the step, after doing half of it.
    Crash,
}

/// A disk in memory that fails one chosen step.
#[derive(Default)]
struct FakeDisk {
    files: BTreeMap<PathBuf, Vec<u8>>,
    cut: Option<(SaveStep, Cut)>,
    /// Renames over the project's file refused as held, before one passes.
    refuse_replace: u32,
    /// Bytes the disk gives back in place of the ones written.
    corrupt_reads: bool,
    waited: Duration,
}

impl FakeDisk {
    fn with(path: &Path, bytes: &[u8]) -> FakeDisk {
        let mut disk = FakeDisk::default();
        disk.files.insert(path.to_owned(), bytes.to_vec());
        disk
    }

    fn at(&mut self, step: SaveStep, half: impl FnOnce(&mut Self)) -> io::Result<()> {
        match self.cut {
            Some((cut, Cut::Error)) if cut == step => Err(io::Error::other("cut here")),
            Some((cut, Cut::Crash)) if cut == step => {
                half(self);
                panic!("the process died at {step:?}");
            }
            _ => Ok(()),
        }
    }
}

impl Disk for FakeDisk {
    fn write_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        assert!(!self.files.contains_key(path), "written over {path:?}");
        // Dying mid-write leaves half the bytes.
        self.at(SaveStep::Write, |disk| {
            disk.files
                .insert(path.to_owned(), bytes[..bytes.len() / 2].to_vec());
        })?;
        self.files.insert(path.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn flush(&mut self, _: &Path) -> io::Result<()> {
        self.at(SaveStep::Flush, |_| {})
    }

    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        self.at(SaveStep::ReadBack, |_| {})?;
        let mut bytes = self
            .files
            .get(path)
            .cloned()
            .ok_or(io::ErrorKind::NotFound)?;
        if self.corrupt_reads {
            bytes[0] ^= 1;
        }
        Ok(bytes)
    }

    fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let bytes = self
            .files
            .get(from)
            .cloned()
            .ok_or(io::ErrorKind::NotFound)?;
        self.at(SaveStep::KeepPrevious, |disk| {
            disk.files
                .insert(companion(to, ".copying"), bytes[..1].to_vec());
        })?;
        self.files.insert(to.to_owned(), bytes);
        Ok(())
    }

    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        if !to.to_string_lossy().ends_with(".new") {
            self.at(SaveStep::Replace, |_| {})?;
            if self.refuse_replace > 0 {
                self.refuse_replace -= 1;
                return Err(io::ErrorKind::PermissionDenied.into());
            }
        }
        let bytes = self.files.remove(from).ok_or(io::ErrorKind::NotFound)?;
        self.files.insert(to.to_owned(), bytes);
        Ok(())
    }

    fn flush_folder(&mut self, _: &Path) -> io::Result<()> {
        self.at(SaveStep::FlushFolder, |_| {})
    }

    fn remove(&mut self, path: &Path) {
        self.files.remove(path);
    }

    fn exists(&mut self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn wait(&mut self, how_long: Duration) {
        self.waited += how_long;
    }
}

fn project() -> PathBuf {
    PathBuf::from("/projects/film/concat.json")
}

#[test]
fn a_first_save_writes_the_file_and_keeps_no_previous() {
    let mut disk = FakeDisk::default();
    let saved = save::save_on(&mut disk, &project(), NEW).expect("saved");
    assert_eq!(
        saved,
        Saved {
            kept_previous: false,
            refusals: 0
        }
    );
    assert_eq!(disk.files.get(&project()).map(Vec::as_slice), Some(NEW));
    assert_eq!(
        disk.files.len(),
        1,
        "nothing beside it: {:?}",
        disk.files.keys()
    );
}

#[test]
fn a_save_keeps_the_one_before_it() {
    let mut disk = FakeDisk::with(&project(), OLD);
    let saved = save::save_on(&mut disk, &project(), NEW).expect("saved");
    assert!(saved.kept_previous);
    assert_eq!(disk.files[&project()], NEW);
    assert_eq!(disk.files[&companion(&project(), ".prev")], OLD);
    assert_eq!(disk.files.len(), 2, "{:?}", disk.files.keys());
}

#[test]
fn a_step_that_fails_leaves_the_old_save_whole() {
    for step in SaveStep::ALL {
        let mut disk = FakeDisk::with(&project(), OLD);
        disk.cut = Some((step, Cut::Error));
        let result = save::save_on(&mut disk, &project(), NEW);
        let file = &disk.files[&project()];
        match step {
            // The new save is in place; only its lasting is not assured.
            SaveStep::FlushFolder => {
                assert!(
                    matches!(result, Err(SaveError::FolderNotFlushed(_))),
                    "{step:?}"
                );
                assert_eq!(file, NEW, "{step:?}");
            }
            // The new save is whole and checked: it waits for the next open.
            SaveStep::KeepPrevious | SaveStep::Replace => {
                assert!(
                    matches!(result, Err(SaveError::Held { .. })),
                    "{step:?}: {result:?}"
                );
                assert_eq!(file, OLD, "{step:?}");
                assert_eq!(disk.files[&companion(&project(), ".new")], NEW, "{step:?}");
            }
            _ => {
                assert!(
                    matches!(result, Err(SaveError::Failed { step: s, .. }) if s == step),
                    "{step:?}"
                );
                assert_eq!(file, OLD, "{step:?}");
                assert_eq!(disk.files.len(), 1, "{step:?} left {:?}", disk.files.keys());
            }
        }
    }
}

#[test]
fn a_process_that_dies_at_any_step_leaves_one_whole_save() {
    for step in SaveStep::ALL {
        let mut disk = FakeDisk::with(&project(), OLD);
        disk.cut = Some((step, Cut::Crash));
        let died = catch_unwind(AssertUnwindSafe(|| {
            save::save_on(&mut disk, &project(), NEW)
        }));
        assert!(died.is_err(), "{step:?} was cut");
        let file = disk.files[&project()].as_slice();
        assert!(file == OLD || file == NEW, "{step:?} left {file:?}");
        // A half-made copy of the previous is never taken for it.
        if let Some(previous) = disk.files.get(&companion(&project(), ".prev")) {
            assert_eq!(previous, OLD, "{step:?}");
        }
        // The next save of the same process clears what this one left.
        disk.cut = None;
        save::save_on(&mut disk, &project(), NEW).expect("the next save lands");
        assert_eq!(disk.files[&project()], NEW, "{step:?}");
    }
}

#[test]
fn a_disk_that_gives_back_other_bytes_is_caught_before_the_old_save_is_touched() {
    let mut disk = FakeDisk::with(&project(), OLD);
    disk.corrupt_reads = true;
    let result = save::save_on(&mut disk, &project(), NEW);
    assert!(
        matches!(result, Err(SaveError::ReadBackDiffers)),
        "{result:?}"
    );
    assert_eq!(disk.files[&project()], OLD);
    assert_eq!(disk.files.len(), 1, "{:?}", disk.files.keys());
}

#[test]
fn a_file_held_for_a_moment_is_replaced_once_let_go() {
    let mut disk = FakeDisk::with(&project(), OLD);
    disk.refuse_replace = 3;
    let saved = save::save_on(&mut disk, &project(), NEW).expect("saved");
    assert_eq!(saved.refusals, 3);
    assert_eq!(disk.files[&project()], NEW);
}

#[test]
fn a_file_held_too_long_leaves_the_save_waiting_and_the_next_open_adopts_it() {
    let mut disk = FakeDisk::with(&project(), OLD);
    disk.refuse_replace = u32::MAX;
    // The fake's waits take no time, so the patience is spent in tries:
    // the real clock still bounds it.
    let started = std::time::Instant::now();
    let result = save::save_on(&mut disk, &project(), NEW);
    assert!(started.elapsed() < save::PATIENCE + Duration::from_secs(1));
    let Err(SaveError::Held { waiting, .. }) = result else {
        panic!("held: {result:?}");
    };
    assert_eq!(waiting, companion(&project(), ".new"));
    assert_eq!(disk.files[&project()], OLD);
    assert_eq!(disk.files[&waiting], NEW);

    disk.refuse_replace = 0;
    let adopted = save::adopt_on(&mut disk, &project()).expect("adopted");
    assert!(adopted.kept_previous);
    assert_eq!(disk.files[&project()], NEW);
    assert_eq!(disk.files[&companion(&project(), ".prev")], OLD);
    assert!(!disk.files.contains_key(&waiting));
}

#[test]
fn on_the_real_disk_two_saves_leave_the_file_and_its_previous() {
    let folder = tempfile::tempdir().expect("a folder");
    let path = folder.path().join("concat.json");
    save::save(&path, OLD).expect("first");
    let saved = save::save(&path, NEW).expect("second");
    assert!(saved.kept_previous);
    assert_eq!(std::fs::read(&path).expect("read"), NEW);
    assert_eq!(std::fs::read(companion(&path, ".prev")).expect("read"), OLD);
    assert!(save::pending(&path).is_none());
    let mut names: Vec<String> = std::fs::read_dir(folder.path())
        .expect("list")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, ["concat.json", "concat.json.prev"]);
}
