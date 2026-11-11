// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What a run writes down about itself.
//!
//! A packaged editor has no terminal, and Concat sends nothing anywhere: the
//! file a run leaves in the person's own data folder is the only account of
//! it there will ever be, and the one thing a report of "it stopped" can
//! bring with it. So that file is written to be read by whoever opens it -
//! the person whose machine it is, or the one trying to help them:
//!
//! - **Whole.** Every line is in the file before the call that logged it
//!   returns, so a crash takes nothing with it (`file.rs`).
//!   A run that ends in order says so on its last line, and the next run
//!   can tell one that did not ([`previous`]).
//! - **Readable.** A line is when, how bad, what about, and what happened,
//!   in columns (`line.rs`); in a terminal it is coloured.
//!   A piece of work is one line that times each of its steps ([`Op`]).
//! - **Bounded.** The same line over and over is counted, not repeated
//!   (`repeat.rs`); a long run keeps its start and its end; the folder
//!   keeps its newest ten runs.
//! - **Shareable.** The home folder in a path is written `~`, so a log can
//!   be attached to a public issue without the person's name in it.
//! - **Adjustable.** Concat's crates have one level and the libraries
//!   underneath another, and any module can be named ([`Filter`]).
//!
//! Everything goes through the `log` facade: a crate writes `log::warn!`
//! and neither knows nor cares what is on the other end, and so does every
//! library Concat is built on.

mod clock;
mod console;
mod file;
mod filter;
mod line;
mod op;
mod repeat;

use std::cell::RefCell;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub use clock::span;
pub use file::{CAP, HEAD, KEEP, PreviousRun, TAIL};
pub use filter::{Filter, level_name, level_named};
pub use op::Op;

use console::Console;
use file::{Limits, RunFile};
use line::{Dress, Line};
use repeat::{Owed, Repeats, Verdict};

/// The environment variable that says what is written; see [`Filter`].
pub const LEVEL_VARIABLE: &str = "CONCAT_LOG";

/// How a run's log is set up.
pub struct Config {
    /// Where the runs' files are kept, or `None` for a run that keeps no
    /// file - an agent's server, whose lines its client shows.
    pub folder: Option<PathBuf>,
    /// What is written; [`Filter::parse`] of [`LEVEL_VARIABLE`], usually.
    pub filter: Filter,
    /// Where lines go besides the file when it is not standard error:
    /// logcat on a phone. With one, standard error is left alone - on a
    /// phone it is a pipe into that same console, and a line written there
    /// would come back round and log itself forever.
    pub platform: Option<Box<dyn log::Log>>,
    /// The person's home folder, written `~` wherever it turns up in a line.
    pub home: Option<PathBuf>,
}

/// What [`open`] found and made.
#[derive(Debug)]
pub struct Opened {
    /// This run's file, when it has one.
    pub file: Option<PathBuf>,
    /// Why it has none, when a folder was given and no file could be made.
    /// Never a reason not to start: the console still has every line.
    pub file_error: Option<String>,
    /// How the run before this one ended.
    pub previous: Option<PreviousRun>,
}

/// The process's logger, once [`open`] has made it.
static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Opens this run's log and makes it where the `log` facade leads.
///
/// Call once, as early as a run can: anything logged before this is lost.
/// An `Err` means a logger was already installed, by this or by something
/// else, and nothing was changed.
pub fn open(config: Config) -> Result<Opened, String> {
    if LOGGER.get().is_some() {
        return Err("a logger was already installed".to_owned());
    }
    let at = clock::now();
    let (previous, file, file_error) = match &config.folder {
        None => (None, None, None),
        Some(folder) => {
            // Read before this run's own file exists to be mistaken for it.
            let previous = file::previous(folder);
            match RunFile::create(folder, at, Limits::default()) {
                Ok(file) => (previous, Some(file), None),
                Err(error) => (previous, None, Some(error)),
            }
        }
    };
    let path = file.as_ref().map(|file| file.path().to_path_buf());
    let logger = Logger::new(
        file,
        config.filter,
        config.platform,
        config.home.as_deref(),
        at,
    );
    let ceiling = logger.filter.ceiling();
    if LOGGER.set(logger).is_err() {
        return Err("a logger was already installed".to_owned());
    }
    let logger = LOGGER.get().expect("set on the line above");
    log::set_logger(logger)
        .map(|()| log::set_max_level(ceiling))
        .map_err(|_| "a logger was already installed".to_owned())?;
    Ok(Opened {
        file: path,
        file_error,
        previous,
    })
}

/// The file this run is writing to, or `None` when it has none.
pub fn current() -> Option<&'static Path> {
    LOGGER.get()?.path.as_deref()
}

/// This run's file once more, for a crash handler to write its line into:
/// a handle of its own, so a handler never touches the logger's, or its
/// lock.
pub fn duplicate() -> Option<std::fs::File> {
    let state = LOGGER.get()?.state.lock().ok()?;
    state.file.as_ref()?.duplicate()
}

/// The level Concat's own crates are written at.
pub fn level() -> log::LevelFilter {
    LOGGER
        .get()
        .map_or(log::LevelFilter::Off, |logger| logger.filter.own())
}

/// Whether this run was told its level - [`LEVEL_VARIABLE`] named one - and
/// so should not take a remembered one instead.
pub fn level_was_named() -> bool {
    LOGGER.get().is_some_and(|logger| logger.filter.named())
}

/// Sets the level Concat's own crates are written at for the rest of the
/// run, and says so. The libraries underneath stay where they were.
pub fn set_level(level: log::LevelFilter) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    logger.filter.set_own(level);
    log::set_max_level(logger.filter.ceiling());
    log::info!("log: level {}", level_name(level));
}

/// Ends the run's log in order: the counts still owed are written, then the
/// line that says the run ended cleanly, and the file is made safe on disk.
/// A file without that line is how the next run knows this one did not get
/// here.
pub fn end() {
    if let Some(logger) = LOGGER.get() {
        logger.end();
    }
}

/// Sends panics to the log before they reach wherever they were going.
///
/// A panic is the one failure that never passes through a `Result`, and in a
/// packaged build it is also the one that leaves nothing behind: the message
/// goes to a standard error nobody is reading and the window disappears. The
/// hook that was there still runs afterwards, so a terminal still prints the
/// panic exactly as before.
pub fn catch_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".to_owned());
        let place = info
            .location()
            .map_or_else(|| "somewhere".to_owned(), |at| at.to_string());
        // The frames, where the build can name them and the run asked for
        // them (`RUST_BACKTRACE`); a shipped build can do neither, and the
        // place above is what it has.
        let trace = std::backtrace::Backtrace::capture();
        if trace.status() == std::backtrace::BacktraceStatus::Captured {
            log::error!("panic: {message}, at {place}\n{trace}");
        } else {
            log::error!("panic: {message}, at {place}");
        }
        previous(info);
    }));
}

/// A size in bytes the way a person says it: `812 KB`, `41.3 MB`, `2.75 GB`.
pub fn bytes(count: u64) -> String {
    const KB: f64 = 1024.0;
    let size = count as f64;
    if size < KB {
        format!("{count} B")
    } else if size < KB * KB {
        format!("{:.0} KB", size / KB)
    } else if size < KB * KB * KB {
        format!("{:.1} MB", size / (KB * KB))
    } else {
        format!("{:.2} GB", size / (KB * KB * KB))
    }
}

/// How often the counts of lines that have stopped repeating are looked for.
const SETTLE_EVERY: Duration = Duration::from_secs(1);

/// The process's logger: a file, a console, and on a phone the system's own.
struct Logger {
    filter: Filter,
    /// The file's path, outside the lock: asked for from anywhere.
    path: Option<PathBuf>,
    state: Mutex<State>,
    /// Standard error; `None` when the platform has a console of its own.
    console: Option<Console>,
    platform: Option<Box<dyn log::Log>>,
    /// The home folder as it is spelt in paths, to be written `~`.
    homes: Vec<String>,
    started: Instant,
}

/// What a line changes, under one lock so the file reads in the order
/// things were said.
struct State {
    file: Option<RunFile>,
    repeats: Repeats,
    settled: Instant,
    /// The day the last line was written on; the file says the day once
    /// and its lines say only the time.
    day: Option<i64>,
}

/// A thread's own buffers, so a line costs no allocation once a thread has
/// logged before.
#[derive(Default)]
struct Scratch {
    body: String,
    plain: String,
    coloured: String,
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

impl Logger {
    fn new(
        file: Option<RunFile>,
        filter: Filter,
        platform: Option<Box<dyn log::Log>>,
        home: Option<&Path>,
        at: (i64, u32),
    ) -> Self {
        let mut state = State {
            file,
            repeats: Repeats::default(),
            settled: Instant::now(),
            day: Some(clock::day(at)),
        };
        if let Some(file) = &mut state.file {
            file.write(&format!(
                "Concat log · {} · times are UTC\n",
                clock::date(at)
            ));
        }
        Logger {
            filter,
            path: state.file.as_ref().map(|file| file.path().to_path_buf()),
            console: platform.is_none().then(Console::new),
            platform,
            homes: homes(home),
            state: Mutex::new(state),
            started: Instant::now(),
        }
    }

    /// One line through everything: counted, laid out, and written.
    fn write(
        &self,
        level: log::Level,
        target: &str,
        body: &str,
        record: Option<&log::Record<'_>>,
        plain: &mut String,
        coloured: &mut String,
    ) {
        let at = clock::now();
        let now = Instant::now();
        let thread = std::thread::current();
        let thread = match thread.name() {
            Some(name) => std::borrow::Cow::Borrowed(name),
            None => std::borrow::Cow::Owned(unnamed(thread.id())),
        };
        let colour = self.console.as_ref().is_some_and(Console::colour);
        plain.clear();
        coloured.clear();
        let mut owed: Vec<Owed> = Vec::new();
        let verdict = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let verdict = state.repeats.admit(level, target, body, now, &mut owed);
            if now.duration_since(state.settled) >= SETTLE_EVERY {
                owed.extend(state.repeats.settle(now, false));
                state.settled = now;
            }
            let today = clock::day(at);
            if state.day != Some(today) && (verdict != Verdict::Count || !owed.is_empty()) {
                state.day = Some(today);
                let _ = writeln!(plain, "---- {} · times are UTC ----", clock::date(at));
            }
            for owed in &owed {
                let suffix = format!(" (repeated {} more time{})", owed.more, plural(owed.more));
                let line = Line {
                    at,
                    level: owed.level,
                    target: &owed.target,
                    thread: "",
                    body: &owed.body,
                    suffix: &suffix,
                };
                line::compose(plain, &line, Dress::Plain);
                if colour {
                    line::compose(coloured, &line, Dress::Coloured);
                }
            }
            if verdict != Verdict::Count {
                let suffix = match verdict {
                    Verdict::WriteCounting(more) => {
                        format!(" (+{more} more in the last {})", span(repeat::WINDOW))
                    }
                    _ => String::new(),
                };
                let line = Line {
                    at,
                    level,
                    target,
                    thread: &thread,
                    body,
                    suffix: &suffix,
                };
                line::compose(plain, &line, Dress::Plain);
                if colour {
                    line::compose(coloured, &line, Dress::Coloured);
                }
            }
            if let Some(file) = &mut state.file
                && !plain.is_empty()
            {
                file.write(plain);
            }
            verdict
        };
        if let Some(console) = &self.console {
            let text = if colour { &*coloured } else { &*plain };
            if !text.is_empty() {
                console.write(text);
            }
        }
        if let (Some(platform), Some(record)) = (&self.platform, record)
            && verdict != Verdict::Count
        {
            platform.log(record);
        }
    }

    /// The home folder in `body`, written `~`.
    fn redact(&self, body: &mut String) {
        for home in &self.homes {
            if body.contains(home.as_str()) {
                *body = body.replace(home.as_str(), "~");
            }
        }
    }

    fn end(&self) {
        let owed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.repeats.settle(Instant::now(), true)
        };
        let (mut plain, mut coloured) = (String::new(), String::new());
        for owed in owed {
            let body = format!(
                "{} (repeated {} more time{})",
                owed.body,
                owed.more,
                plural(owed.more)
            );
            self.write(
                owed.level,
                &owed.target,
                &body,
                None,
                &mut plain,
                &mut coloured,
            );
        }
        let body = format!(
            "log: {} after {}",
            file::ENDED,
            span(self.started.elapsed())
        );
        self.write(
            log::Level::Info,
            "concat_log",
            &body,
            None,
            &mut plain,
            &mut coloured,
        );
        log::Log::flush(self);
    }
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.filter.allows(metadata.target(), metadata.level())
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // The thread's own buffers, taken for the length of the line and
        // put back, so a line costs no allocation once a thread has logged
        // before. A line logged from inside the writing of another - a
        // `Display` that logs - finds them gone and makes its own.
        let mut scratch = SCRATCH
            .try_with(|cell| {
                cell.try_borrow_mut()
                    .map(|mut kept| std::mem::take(&mut *kept))
            })
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        scratch.body.clear();
        let _ = write!(scratch.body, "{}", record.args());
        self.redact(&mut scratch.body);
        self.write(
            record.level(),
            record.target(),
            &scratch.body,
            Some(record),
            &mut scratch.plain,
            &mut scratch.coloured,
        );
        let _ = SCRATCH.try_with(|cell| {
            if let Ok(mut kept) = cell.try_borrow_mut() {
                *kept = scratch;
            }
        });
    }

    fn flush(&self) {
        // Nothing of ours is waiting: each line was handed to the system as
        // it was logged. This asks the system to put it on the disk, which
        // only a machine losing power needs.
        if let Ok(state) = self.state.lock()
            && let Some(file) = state.file.as_ref().and_then(RunFile::duplicate)
        {
            let _ = file.sync_all();
        }
        if let Some(platform) = &self.platform {
            platform.flush();
        }
    }
}

/// The spellings of the home folder worth looking for in a line: as the
/// system gives it, and on Windows with the slashes a URL or a library
/// would have turned them into. Nothing shorter than a real home - a
/// container's `/` would turn every path into `~`.
fn homes(home: Option<&Path>) -> Vec<String> {
    let Some(home) = home.and_then(Path::to_str) else {
        return Vec::new();
    };
    let home = home.trim_end_matches(['/', '\\']);
    if home.len() < 4 {
        return Vec::new();
    }
    let mut homes = vec![home.to_owned()];
    if home.contains('\\') {
        homes.push(home.replace('\\', "/"));
    }
    homes
}

/// A name for a thread that was given none: `thread-12`.
fn unnamed(id: std::thread::ThreadId) -> String {
    let debug = format!("{id:?}");
    let number: String = debug.chars().filter(char::is_ascii_digit).collect();
    format!("thread-{number}")
}

fn plural(count: u32) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("concat-log-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    /// A logger with a file and nothing else, driven directly: the facade
    /// has one logger per process, and these each want their own.
    fn logger(folder: &Path, home: Option<&Path>) -> Logger {
        let file = RunFile::create(folder, (1_726_316_096, 0), Limits::default()).expect("a file");
        let (filter, _) = Filter::parse("debug");
        let mut logger = Logger::new(Some(file), filter, None, home, (1_726_316_096, 0));
        logger.console = None;
        logger
    }

    fn say(logger: &Logger, level: log::Level, target: &str, body: &str) {
        log::Log::log(
            logger,
            &log::Record::builder()
                .level(level)
                .target(target)
                .args(format_args!("{body}"))
                .build(),
        );
    }

    fn read(logger: &Logger) -> String {
        std::fs::read_to_string(logger.path.as_ref().expect("a path")).expect("reads back")
    }

    #[test]
    fn a_line_is_in_the_file_the_moment_it_is_logged() {
        let folder = scratch_folder("whole");
        let logger = logger(&folder, None);
        say(
            &logger,
            log::Level::Warn,
            "concat_media::decode",
            "decode: the kettle is broken",
        );
        // No flush, no end: the file already has it.
        let text = read(&logger);
        assert!(
            text.starts_with("Concat log · 2024-09-14 · times are UTC\n"),
            "{text}"
        );
        // The file was opened as if on another day, so the day is said
        // again before the first line of this one.
        assert!(
            text.lines()
                .nth(1)
                .is_some_and(|line| line.starts_with("---- 20")),
            "{text}"
        );
        let line = text.lines().nth(2).expect("the line");
        assert!(
            line.contains(" WARN  decode     the kettle is broken  [concat_media::decode, "),
            "{line}"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_libraries_stay_quiet_when_concat_is_turned_up() {
        let folder = scratch_folder("quiet");
        let logger = logger(&folder, None);
        say(&logger, log::Level::Debug, "concat_render::gpu", "said");
        say(&logger, log::Level::Debug, "naga::front", "not said");
        say(&logger, log::Level::Warn, "naga::front", "said too");
        let text = read(&logger);
        assert!(text.contains(" said  ["), "{text}");
        assert!(!text.contains("not said"), "{text}");
        assert!(text.contains("said too"), "{text}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_storm_is_a_few_lines_and_a_count() {
        let folder = scratch_folder("storm");
        let logger = logger(&folder, None);
        for _ in 0..200 {
            say(
                &logger,
                log::Level::Error,
                "concat_render::gpu",
                "no software adapter",
            );
            say(
                &logger,
                log::Level::Info,
                "concat_render::gpu",
                "compositor: nothing to draw on",
            );
        }
        logger.end();
        let text = read(&logger);
        let storm = text.matches("no software adapter").count();
        // The burst, and once more with the count as the run ends.
        assert_eq!(storm, repeat::BURST as usize + 1, "{text}");
        let more = 200 - repeat::BURST;
        assert!(
            text.contains(&format!("no software adapter (repeated {more} more times)")),
            "{text}"
        );
        assert!(
            text.contains(&format!("nothing to draw on (repeated {more} more times)")),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_run_that_ends_in_order_says_so_and_the_next_can_tell() {
        let folder = scratch_folder("ended");
        let logger = logger(&folder, None);
        say(
            &logger,
            log::Level::Info,
            "concat",
            "start: the window is up",
        );
        assert!(!file::previous(&folder).expect("a run").ended_cleanly);
        logger.end();
        let text = read(&logger);
        let last = text.lines().last().expect("a last line");
        assert!(
            last.contains(" INFO  log        run ended cleanly after "),
            "{last}"
        );
        assert!(file::previous(&folder).expect("a run").ended_cleanly);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_home_folder_is_written_as_a_tilde() {
        let folder = scratch_folder("home");
        let logger = logger(&folder, Some(Path::new("/Users/someone/")));
        say(
            &logger,
            log::Level::Info,
            "concat",
            "project: opening /Users/someone/Desktop/Trip and /Users/someone/Movies/a.mov",
        );
        let text = read(&logger);
        assert!(
            text.contains("opening ~/Desktop/Trip and ~/Movies/a.mov"),
            "{text}"
        );
        assert!(!text.contains("someone"), "{text}");
        // Windows spells it two ways, and a home too short to be one is
        // left alone.
        assert_eq!(
            homes(Some(Path::new(r"C:\Users\someone"))),
            [r"C:\Users\someone", "C:/Users/someone"]
        );
        assert!(homes(Some(Path::new("/"))).is_empty());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn sizes_are_said_in_the_unit_they_are_about() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(831_488), "812 KB");
        assert_eq!(bytes(43_306_189), "41.3 MB");
        assert_eq!(bytes(2_952_790_016), "2.75 GB");
    }

    #[test]
    fn a_thread_with_no_name_is_named_for_its_number() {
        let name = std::thread::spawn(|| unnamed(std::thread::current().id()))
            .join()
            .expect("joins");
        assert!(name.starts_with("thread-"), "{name}");
        assert!(name["thread-".len()..].parse::<u64>().is_ok(), "{name}");
    }
}
