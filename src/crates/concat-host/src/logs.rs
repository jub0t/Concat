// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What the app writes down about its own run.
//!
//! A packaged editor has no terminal. The `.app` a person double-clicks, the
//! `.exe` a launcher starts and the window a desktop entry opens all have
//! their standard error wired to nowhere, so every line the engine writes
//! about a failed decode, a lost device or a refused file went straight into
//! the dark, and a report of "it stopped" came with nothing behind it. This
//! is where those lines go instead.
//!
//! One file per run, in `<app data>/logs/`, named for the instant it opened.
//! A run is the unit because that is the unit a person reports: *this* time I
//! opened it, *this* is what it did. The newest [`KEEP`] files stay and the
//! rest are deleted as each run starts, so the folder is bounded without a
//! sweeper, and each file stops growing at [`CAP`] so a loop that logs on
//! every frame cannot fill a disk.
//!
//! Everything goes through the `log` facade, so a crate writes `log::warn!`
//! and neither knows nor cares that a file is on the other end. The lines
//! reach a console as well as the file - standard error on a desktop, so a
//! run from a terminal reads exactly as it did, and logcat on a phone.
//!
//! What ends up here is the app's own account of itself - not a profile, not
//! telemetry. Nothing is sent anywhere. The file is the user's, in their own
//! data folder, and Settings › About is where they can go and find it.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::dirs::AppDirs;

/// How many runs' logs are kept. Ten covers "it did it again yesterday"
/// without the folder ever being something a person has to think about.
pub const KEEP: usize = 10;

/// How much one run may write before the file stops growing, in bytes.
pub const CAP: u64 = 16 * 1024 * 1024;

/// The environment variable that overrides the level: `error`, `warn`,
/// `info`, `debug` or `trace`. Info without one.
pub const LEVEL_VARIABLE: &str = "CONCAT_LOG";

/// The file this run is writing to, once [`open`] has been called.
static CURRENT: OnceLock<PathBuf> = OnceLock::new();

/// Where the logs are kept.
pub fn folder(dirs: &AppDirs) -> PathBuf {
    dirs.data.join("logs")
}

/// The file this run is writing to, or `None` when there is none - [`open`]
/// was never called, or it could not make one.
pub fn current() -> Option<&'static Path> {
    CURRENT.get().map(PathBuf::as_path)
}

/// Opens this run's file and makes it the process's logger.
///
/// Call once, as early as a run can: anything logged before this is lost.
/// `extra` is where the lines go besides the file - logcat on a phone - and
/// `None` on a desktop, which puts them on standard error instead, so a run
/// from a terminal reads the way it always did.
///
/// Returns the file it opened. An `Err` means there is no file this run -
/// an unwritable data folder, a second call - and is worth reporting, but it
/// is never worth refusing to start over: the logger is installed either
/// way, and standard error still has everything.
pub fn open(dirs: &AppDirs, extra: Option<Box<dyn log::Log>>) -> Result<PathBuf, String> {
    let opened = begin(dirs);
    let sink = match &opened {
        Ok((path, file)) => match file.try_clone() {
            Ok(file) => {
                let _ = CURRENT.set(path.clone());
                Some(Sink {
                    file,
                    written: 0,
                    stopped: false,
                })
            }
            Err(_) => None,
        },
        Err(_) => None,
    };
    install(sink, extra)?;
    opened.map(|(path, _)| path)
}

/// The logger with no file: the console alone.
///
/// For a run with nowhere to put a file - a machine whose data directory
/// cannot be located at all. The facade still has to lead somewhere, or
/// every line the engine writes disappears silently instead of loudly.
pub fn open_console(extra: Option<Box<dyn log::Log>>) -> Result<(), String> {
    install(None, extra)
}

/// Makes a logger out of the pieces and hands it to the facade.
fn install(sink: Option<Sink>, extra: Option<Box<dyn log::Log>>) -> Result<(), String> {
    let level = level_from_environment();
    let logger = Logger {
        sink: sink.map(Mutex::new),
        extra,
    };
    log::set_boxed_logger(Box::new(logger))
        .map(|()| log::set_max_level(level))
        .map_err(|_| "a logger was already installed".to_owned())?;
    log::info!(
        "Concat {} · {} {} · {level}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    Ok(())
}

/// Sends panics to the log before they reach wherever they were going.
///
/// A panic is the one failure that never passes through a `Result`, and in a
/// packaged build it is also the one that leaves nothing behind: the message
/// goes to a standard error nobody is reading and the window disappears. The
/// hook that was there still runs afterwards, so a terminal still prints the
/// panic and a backtrace exactly as before.
pub fn catch_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let where_ = info
            .location()
            .map_or_else(|| "somewhere".to_owned(), |at| at.to_string());
        log::error!("panic at {where_}: {}", info);
        previous(info);
    }));
}

/// This run's file: the folder made, the older runs pruned, the file opened.
fn begin(dirs: &AppDirs) -> Result<(PathBuf, std::fs::File), String> {
    let folder = folder(dirs);
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("could not create {}: {error}", folder.display()))?;
    prune(&folder);
    let path = folder.join(format!("concat-{}.log", stamp(now(), Stamp::File)));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    Ok((path, file))
}

/// Deletes all but the newest [`KEEP`] runs.
///
/// By name, which for this name is by time: nothing depends on a file system
/// that reports modification times, and a file copied out and back keeps its
/// place in the order.
fn prune(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut ours: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("concat-") && name.ends_with(".log"))
        })
        .collect();
    if ours.len() < KEEP {
        return;
    }
    ours.sort();
    // One short of KEEP, because the run doing the pruning is about to
    // make its own.
    let doomed = ours.len().saturating_sub(KEEP.saturating_sub(1));
    for path in ours.into_iter().take(doomed) {
        let _ = std::fs::remove_file(path);
    }
}

fn level_from_environment() -> log::LevelFilter {
    match std::env::var(LEVEL_VARIABLE)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "error" => log::LevelFilter::Error,
        "warn" => log::LevelFilter::Warn,
        "debug" => log::LevelFilter::Debug,
        "trace" => log::LevelFilter::Trace,
        "off" => log::LevelFilter::Off,
        _ => log::LevelFilter::Info,
    }
}

/// The open file and how much of its allowance is gone.
struct Sink {
    file: std::fs::File,
    written: u64,
    /// Set once the cap is reached; the line that says so is written once.
    stopped: bool,
}

/// The process's logger: a file, standard error, and on a phone one more.
struct Logger {
    /// `None` when this run has no file; standard error still has it all.
    sink: Option<Mutex<Sink>>,
    extra: Option<Box<dyn log::Log>>,
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format(record);
        // Standard error keeps its copy, so a run from a terminal reads the
        // way it always did - but only when the caller named nowhere else.
        // A caller that hands over a console of its own already has one, and
        // on a phone that console is fed by a pipe off standard error: a line
        // written there would come back through it and write itself forever.
        if self.extra.is_none() {
            eprintln!("{line}");
        }
        if let Some(sink) = &self.sink {
            let mut sink = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if sink.stopped {
                return;
            }
            if sink.written >= CAP {
                sink.stopped = true;
                let _ = writeln!(
                    sink.file,
                    "[this run passed {CAP} bytes of log; the rest is not written down]"
                );
                return;
            }
            if writeln!(sink.file, "{line}").is_ok() {
                sink.written += line.len() as u64 + 1;
            }
        }
        if let Some(extra) = &self.extra {
            extra.log(record);
        }
    }

    fn flush(&self) {
        if let Some(sink) = &self.sink {
            let mut sink = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = sink.file.flush();
        }
        if let Some(extra) = &self.extra {
            extra.flush();
        }
    }
}

/// One line: when, how bad, which thread, which module, and what happened.
///
/// The thread is in there because the answer to a good half of what gets
/// logged is "which thread was that on" - a decode worker, the artwork lane,
/// the event loop - and a line that does not say cannot be asked later.
fn format(record: &log::Record<'_>) -> String {
    let thread = std::thread::current();
    let who = thread.name().unwrap_or("?").to_owned();
    let mut line = String::with_capacity(128);
    let _ = write!(
        line,
        "{} {:<5} [{}] {}: {}",
        stamp(now(), Stamp::Line),
        record.level(),
        who,
        record.target(),
        record.args()
    );
    line
}

/// Seconds and milliseconds since the epoch, or zero if the clock is before
/// it - which is not a case worth a branch anywhere else.
fn now() -> (i64, u32) {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => (since.as_secs() as i64, since.subsec_millis()),
        Err(_) => (0, 0),
    }
}

/// Which of the two shapes a stamp is written in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stamp {
    /// `20260914-123456`, for a file name: sorts as it reads, and has
    /// nothing in it a file system anywhere objects to.
    File,
    /// `2026-09-14 12:34:56.789Z`, for a line in one.
    Line,
}

/// UTC, always: a log read by somebody else is a log whose times mean the
/// same thing in both places.
fn stamp((seconds, millis): (i64, u32), shape: Stamp) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (hour, minute, second) = (rest / 3600, (rest % 3600) / 60, rest % 60);
    let (year, month, day) = civil(days);
    match shape {
        Stamp::File => format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}"),
        Stamp::Line => {
            format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03}Z")
        }
    }
}

/// The calendar date `days` after 1970-01-01, by Howard Hinnant's
/// `civil_from_days`: the years are shifted to start in March so that the
/// leap day lands at the end of one and the month lengths fall into a
/// pattern with no table in it.
fn civil(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * march_month + 2) / 5 + 1) as u32;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    } as u32;
    (year + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_and_a_leap_day_come_out_right() {
        assert_eq!(civil(0), (1970, 1, 1));
        // 2024-02-29, a leap day, and the day after it.
        assert_eq!(civil(19_782), (2024, 2, 29));
        assert_eq!(civil(19_783), (2024, 3, 1));
        // 2000 is a leap year and 1900 was not; the era arithmetic is what
        // gets that pair wrong when it is wrong at all.
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(civil(-25_508), (1900, 3, 1));
    }

    #[test]
    fn a_stamp_reads_as_a_date_in_both_shapes() {
        let at = (1_726_316_096, 789);
        assert_eq!(stamp(at, Stamp::Line), "2024-09-14 12:14:56.789Z");
        assert_eq!(stamp(at, Stamp::File), "20240914-121456");
        // A file name sorts the way the runs happened.
        assert!(stamp((0, 0), Stamp::File) < stamp(at, Stamp::File));
    }

    #[test]
    fn pruning_keeps_the_newest_and_leaves_everything_else_alone() {
        let folder = std::env::temp_dir().join(format!("concat-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a temp dir");
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

    /// The one test that installs the process logger, because there is only
    /// one to install: everything from the facade down to a line in a file.
    #[test]
    fn a_line_written_through_the_facade_lands_in_the_file() {
        let root = std::env::temp_dir().join(format!("concat-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a temp dir");
        let dirs = AppDirs::under(&root);

        let path = open(&dirs, None).expect("a log file");
        assert_eq!(path.parent(), Some(folder(&dirs).as_path()));
        assert_eq!(current(), Some(path.as_path()));

        log::warn!("the kettle is broken");
        log::logger().flush();

        let written = std::fs::read_to_string(&path).expect("reads back");
        // The header the install writes, and then the line.
        assert!(written.contains(env!("CARGO_PKG_VERSION")), "{written}");
        assert!(written.contains("WARN"), "{written}");
        assert!(written.contains("the kettle is broken"), "{written}");
        // Every line says when, on which thread, and out of which module.
        let line = written
            .lines()
            .find(|line| line.contains("the kettle is broken"))
            .expect("the line");
        assert!(line.starts_with("20"), "{line}");
        assert!(line.contains("concat_host::logs"), "{line}");

        // A second call cannot take the facade a second time, and says so
        // rather than pretending.
        assert!(open(&dirs, None).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_hangs_off_the_data_directory() {
        let dirs = AppDirs::under(Path::new("/tmp/concat-test"));
        assert_eq!(folder(&dirs), Path::new("/tmp/concat-test/logs"));
    }
}
