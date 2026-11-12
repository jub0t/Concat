// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! One run's file.
//!
//! One file per run, named for the instant it opened: a run is the unit a
//! person reports. Every line is handed to the system whole, in one write,
//! before the call that logged it returns - so whatever ends the process,
//! a crash in a driver included, the file already holds everything said up
//! to that moment. Nothing waits in a buffer of ours.
//!
//! A run that writes more than [`CAP`] keeps its beginning and its end: the
//! first [`HEAD`] say what machine this is and how the run started, the
//! last [`TAIL`] say what it was doing when it stopped, and the middle is
//! what gives way. The newest [`KEEP`] runs stay in the folder.
//!
//! A run that ends in order says so on its last line. A file without that
//! line is a run that crashed, was killed, or lost its machine, and the
//! next run can tell.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// How many runs' logs are kept. Ten covers "it did it again yesterday"
/// without the folder ever being something a person has to think about.
pub const KEEP: usize = 10;

/// How much one run's file may hold, in bytes.
pub const CAP: u64 = 16 * 1024 * 1024;

/// How much of the start of a long run is kept.
pub const HEAD: u64 = 2 * 1024 * 1024;

/// How much of the end of a long run is kept when the middle is dropped.
pub const TAIL: u64 = 6 * 1024 * 1024;

/// What the last line of a run that ended in order says.
pub(crate) const ENDED: &str = "run ended cleanly";

/// The open file, and how much it holds.
pub(crate) struct RunFile {
    path: PathBuf,
    file: std::fs::File,
    written: u64,
    limits: Limits,
}

/// The sizes a file is held to: the constants, except in a test.
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) cap: u64,
    pub(crate) head: u64,
    pub(crate) tail: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            cap: CAP,
            head: HEAD,
            tail: TAIL,
        }
    }
}

impl RunFile {
    /// This run's file in `folder`: the folder made, the older runs pruned,
    /// the file created. `at` is the instant the run is named for.
    pub(crate) fn create(folder: &Path, at: (i64, u32), limits: Limits) -> Result<Self, String> {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("could not create {}: {error}", folder.display()))?;
        prune(folder);
        let stamp = crate::clock::for_file(at);
        // Two runs opened in one second - a window and a second one beside
        // it - each get a file of their own, not one file with both in it.
        let mut last = None;
        for attempt in 1..=9 {
            let name = match attempt {
                1 => format!("concat-{stamp}.log"),
                _ => format!("concat-{stamp}-{attempt}.log"),
            };
            let path = folder.join(name);
            match std::fs::OpenOptions::new()
                .create_new(true)
                .append(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(RunFile {
                        path,
                        file,
                        written: 0,
                        limits,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    last = Some(format!("could not open {}: {error}", path.display()));
                    break;
                }
            }
        }
        Err(last.unwrap_or_else(|| format!("nine logs already opened at {stamp}")))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The file again, for the crash handler to write its one line into
    /// without touching this one or its lock.
    pub(crate) fn duplicate(&self) -> Option<std::fs::File> {
        self.file.try_clone().ok()
    }

    /// Writes whole lines - `text` ends in a newline - in one write.
    pub(crate) fn write(&mut self, text: &str) {
        if self.written + text.len() as u64 > self.limits.cap {
            self.drop_the_middle();
        }
        if self.file.write_all(text.as_bytes()).is_ok() {
            self.written += text.len() as u64;
        }
    }

    /// Makes room in a file at its cap by cutting out the middle: what is
    /// left is the first `head`, a line saying how much went, and the last
    /// `tail`. Done through a second handle, since the one lines are
    /// written through can only append.
    fn drop_the_middle(&mut self) {
        let Limits { head, tail, .. } = self.limits;
        let outcome = (|| -> std::io::Result<u64> {
            let mut whole = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.path)?;
            let length = whole.metadata()?.len();
            if length <= head + tail {
                return Ok(length);
            }
            // Both cuts fall after a newline, so no line is left in halves.
            let mut start = vec![0u8; head as usize];
            whole.read_exact(&mut start)?;
            let keep_head = start
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |at| at as u64 + 1);
            let mut end = vec![0u8; tail as usize];
            whole.seek(SeekFrom::Start(length - tail))?;
            whole.read_exact(&mut end)?;
            let skip = end
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(0, |at| at + 1);
            let kept_tail = &end[skip..];
            let dropped = length - keep_head - kept_tail.len() as u64;
            let note = format!(
                "[{:.1} MB from the middle of this run are not kept: a run's file holds its start and its end]\n",
                dropped as f64 / (1024.0 * 1024.0)
            );
            whole.seek(SeekFrom::Start(keep_head))?;
            whole.write_all(note.as_bytes())?;
            whole.write_all(kept_tail)?;
            let kept = keep_head + note.len() as u64 + kept_tail.len() as u64;
            whole.set_len(kept)?;
            Ok(kept)
        })();
        match outcome {
            Ok(kept) => self.written = kept,
            // A file that cannot be cut is left to grow one more cap's
            // worth before this is tried again: slower to bound, never a
            // reason to stop writing a run down.
            Err(_) => self.written = 0,
        }
    }
}

/// Deletes all but the newest [`KEEP`] runs, less the one about to open.
///
/// By name, which for this name is by time: nothing depends on a file system
/// that reports modification times, and a file copied out and back keeps its
/// place in the order.
fn prune(folder: &Path) {
    let mut ours = runs(folder);
    if ours.len() < KEEP {
        return;
    }
    ours.sort();
    let doomed = ours.len().saturating_sub(KEEP.saturating_sub(1));
    for path in ours.into_iter().take(doomed) {
        let _ = std::fs::remove_file(path);
    }
}

/// Every run's file in the folder, in no order.
fn runs(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("concat-") && name.ends_with(".log"))
        })
        .collect()
}

/// How the run before this one ended, as its file tells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviousRun {
    /// That run's file.
    pub path: PathBuf,
    /// Whether its last line is the one a run writes on its way out. When
    /// it is not, the run crashed, was killed, or lost its machine.
    pub ended_cleanly: bool,
    /// The last error or crash line in it, for saying what went wrong
    /// without anyone opening the file.
    pub last_error: Option<String>,
}

/// The newest run in `folder`, read before this run's own file is made.
pub(crate) fn previous(folder: &Path) -> Option<PreviousRun> {
    let path = runs(folder).into_iter().max()?;
    let mut file = std::fs::File::open(&path).ok()?;
    let length = file.metadata().ok()?.len();
    // The end of the file is where both answers are.
    let from = length.saturating_sub(64 * 1024);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut end = Vec::new();
    file.read_to_end(&mut end).ok()?;
    let end = String::from_utf8_lossy(&end);
    let ended_cleanly = end.lines().rev().take(8).any(|line| line.contains(ENDED));
    let last_error = end
        .lines()
        .rev()
        .find(|line| line.contains(" ERROR ") || line.starts_with("FATAL "))
        .map(|line| line.trim().to_owned());
    Some(PreviousRun {
        path,
        ended_cleanly,
        last_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("concat-log-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a temp dir");
        folder
    }

    #[test]
    fn pruning_keeps_the_newest_and_leaves_everything_else_alone() {
        let folder = scratch("prune");
        for index in 0..KEEP + 5 {
            std::fs::write(
                folder.join(format!("concat-2026091{index:02}-000000.log")),
                "x",
            )
            .expect("a log");
        }
        // Not ours, and not to be touched.
        std::fs::write(folder.join("notes.txt"), "x").expect("a file");
        prune(&folder);

        let mut left: Vec<String> = std::fs::read_dir(&folder)
            .expect("reads")
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        left.sort();
        assert!(left.contains(&"notes.txt".to_owned()));
        let ours: Vec<&String> = left
            .iter()
            .filter(|name| name.starts_with("concat-"))
            .collect();
        assert_eq!(ours.len(), KEEP - 1, "{ours:?}");
        // The ones kept are the last ones written.
        assert!(ours.iter().all(|name| name.as_str() >= "concat-20260906"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn two_runs_in_one_second_get_a_file_each() {
        let folder = scratch("twins");
        let at = (1_726_316_096, 0);
        let first = RunFile::create(&folder, at, Limits::default()).expect("a file");
        let second = RunFile::create(&folder, at, Limits::default()).expect("another");
        assert_ne!(first.path(), second.path());
        assert!(first.path().ends_with("concat-20240914-121456.log"));
        assert!(second.path().ends_with("concat-20240914-121456-2.log"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_long_run_keeps_its_start_and_its_end() {
        let folder = scratch("middle");
        let limits = Limits {
            cap: 4000,
            head: 1000,
            tail: 1500,
        };
        let mut run = RunFile::create(&folder, (0, 0), limits).expect("a file");
        for index in 0..400 {
            run.write(&format!(
                "line {index:04} of a run that says a great deal\n"
            ));
        }
        let text = std::fs::read_to_string(run.path()).expect("reads back");
        assert!(text.len() as u64 <= limits.cap, "{}", text.len());
        // The start is there, the end is there, and the cut says so.
        assert!(text.starts_with("line 0000 "), "{}", &text[..40]);
        assert!(
            text.trim_end()
                .ends_with("line 0399 of a run that says a great deal")
        );
        assert!(text.contains("from the middle of this run are not kept"));
        // No line was left in halves.
        assert!(
            text.lines()
                .all(|line| line.starts_with("line ") || line.starts_with('[')),
            "{text}"
        );
        // And the lines after a cut are in order, each one after the last.
        let numbers: Vec<u32> = text
            .lines()
            .filter_map(|line| line.strip_prefix("line ")?.get(..4)?.parse().ok())
            .collect();
        assert!(numbers.windows(2).all(|pair| pair[0] < pair[1]));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_run_before_is_read_for_how_it_ended() {
        let folder = scratch("previous");
        assert_eq!(previous(&folder), None);
        std::fs::write(
            folder.join("concat-20260101-000000.log"),
            "12:00:00.000 INFO  [main] concat: hello\n12:00:01.000 INFO  [main] concat_log: run ended cleanly after 1.00 s\n",
        )
        .expect("a log");
        let run = previous(&folder).expect("a run");
        assert!(run.ended_cleanly);
        assert_eq!(run.last_error, None);

        // A newer one that stopped mid-sentence, with an error and a crash.
        std::fs::write(
            folder.join("concat-20260102-000000.log"),
            "12:00:00.000 ERROR [worker] concat_render::gpu: device lost\nFATAL [main] crash: SIGSEGV at 0x10\n",
        )
        .expect("a log");
        let run = previous(&folder).expect("a run");
        assert!(run.path.ends_with("concat-20260102-000000.log"));
        assert!(!run.ended_cleanly);
        assert_eq!(
            run.last_error.as_deref(),
            Some("FATAL [main] crash: SIGSEGV at 0x10")
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
