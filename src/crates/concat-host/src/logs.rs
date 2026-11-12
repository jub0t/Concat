// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Where this app's log is kept, and how each run's opens.
//!
//! The log itself - the file, the console, the layout of a line, what is
//! counted and what is kept - is `concat_log`. This is the app's end of it:
//! the folder under the data directory, the lines every run begins with,
//! and the hand-over to the crash handler.
//!
//! A run begins by answering what every report is asked first: which build
//! this is, on which system, which CPU, how much memory - and how the run
//! before this one ended, since "it closed on me" is so often about that
//! one. What ends up in the file is the app's own account of itself - not a
//! profile, not telemetry. Nothing is sent anywhere. The file is the
//! person's, in their own data folder, and Settings › About is where they
//! can go and find it. How lines are written is in `docs/LOGGING.md`.

use std::path::{Path, PathBuf};

pub use concat_log::{CAP, KEEP, LEVEL_VARIABLE, level_name, level_named};

use crate::dirs::AppDirs;

/// Where the logs are kept.
pub fn folder(dirs: &AppDirs) -> PathBuf {
    dirs.data.join("logs")
}

/// The file this run is writing to, or `None` when there is none - [`open`]
/// was never called, or it could not make one.
pub fn current() -> Option<&'static Path> {
    concat_log::current()
}

/// Opens this run's file and makes it the process's logger.
///
/// Call once, as early as a run can: anything logged before this is lost.
/// `extra` is where the lines go besides the file - logcat on a phone - and
/// `None` on a desktop, which puts them on standard error instead, so a run
/// from a terminal reads them as they come.
///
/// Returns the file it opened. An `Err` means there is no file this run -
/// an unwritable data folder, a second call - and is worth reporting, but it
/// is never worth refusing to start over: the logger is installed either
/// way, and the console still has everything.
pub fn open(dirs: &AppDirs, extra: Option<Box<dyn log::Log>>) -> Result<PathBuf, String> {
    let opened = install(Some(folder(dirs)), extra)?;
    match (opened.file, opened.file_error) {
        (Some(path), _) => Ok(path),
        (None, Some(error)) => Err(error),
        (None, None) => Err("no log file was opened".to_owned()),
    }
}

/// The logger with no file: the console alone.
///
/// For a run with nowhere to put a file - a machine whose data directory
/// cannot be located at all - and for an agent's server, whose lines its
/// client shows. The facade still has to lead somewhere, or every line the
/// engine writes disappears silently instead of loudly.
pub fn open_console(extra: Option<Box<dyn log::Log>>) -> Result<(), String> {
    install(None, extra).map(|_| ())
}

/// Opens the log and writes the lines every run begins with.
fn install(
    folder: Option<PathBuf>,
    extra: Option<Box<dyn log::Log>>,
) -> Result<concat_log::Opened, String> {
    let spec = std::env::var(LEVEL_VARIABLE).unwrap_or_default();
    let (filter, skipped) = concat_log::Filter::parse(&spec);
    let opened = concat_log::open(concat_log::Config {
        folder,
        filter,
        platform: extra,
        home: home(),
    })?;
    if let Some(file) = concat_log::duplicate() {
        crate::crash::write_to(&file);
    }
    // What every report is asked first, answered before it is asked.
    log::info!(
        "build: {} · {}",
        crate::machine::build(),
        if cfg!(debug_assertions) {
            "debug build"
        } else {
            "optimised build"
        }
    );
    log::info!("system: {}", crate::machine::system());
    log::info!("cpu: {}", crate::machine::cpu());
    if let Some(total) = crate::memory::total() {
        log::info!("memory: {}", concat_log::bytes(total));
    }
    match &opened.file {
        Some(path) => log::info!(
            "log: {} · Concat at {}",
            path.display(),
            level_name(concat_log::level())
        ),
        None => log::info!(
            "log: no file this run · Concat at {}",
            level_name(concat_log::level())
        ),
    }
    for word in skipped {
        log::warn!(
            "log: {LEVEL_VARIABLE} has \"{word}\", which is neither a level nor module=level; skipped"
        );
    }
    if let Some(previous) = &opened.previous {
        let _ = PREVIOUS.set(previous.clone());
        let name = previous
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match (previous.ended_cleanly, &previous.last_error) {
            (true, _) => log::debug!("last run: {name} ended cleanly"),
            (false, Some(error)) => log::warn!(
                "last run: {name} did not end cleanly (it crashed, was killed, or the machine went down); the last thing that went wrong in it: {error}"
            ),
            (false, None) => log::warn!(
                "last run: {name} did not end cleanly (it crashed, was killed, or the machine went down), with no error in its log"
            ),
        }
    }
    Ok(opened)
}

/// The run before this one, as [`open`] found it: kept for the window,
/// which on a phone says when it died and shares its file.
static PREVIOUS: std::sync::OnceLock<concat_log::PreviousRun> = std::sync::OnceLock::new();

/// The run before this one, when there was one; see [`concat_log::PreviousRun`].
pub fn previous() -> Option<&'static concat_log::PreviousRun> {
    PREVIOUS.get()
}

/// The person's home folder, which the log writes as `~`. A phone has none
/// worth the name: its paths are the app's own sandbox.
fn home() -> Option<PathBuf> {
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return None;
    }
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(name)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// Sends panics to the log before they reach wherever they were going; see
/// [`concat_log::catch_panics`].
pub fn catch_panics() {
    concat_log::catch_panics();
}

/// The level `CONCAT_LOG` names for Concat's own crates, when it names one:
/// a run's own ask, over the one the Settings sheet remembers.
pub fn environment_level() -> Option<log::LevelFilter> {
    let (filter, _) = concat_log::Filter::parse(&std::env::var(LEVEL_VARIABLE).unwrap_or_default());
    filter.named().then(|| filter.own())
}

/// The level Concat's own crates are written at.
pub fn level() -> log::LevelFilter {
    concat_log::level()
}

/// Sets the level for the rest of the run - the Settings sheet's choice,
/// or the remembered one applied at launch - and says so in the log.
///
/// The level is Concat's own: what the app says about what it is doing,
/// for whoever is chasing a bug with it. The crates underneath - wgpu,
/// naga, Slint, fontdb, the decoders - stay at warnings and errors
/// whatever the sheet says, since at Debug they bury a run in lines
/// nobody asked for. `CONCAT_LOG` can name any of them; see
/// [`concat_log::Filter`].
pub fn set_level(level: log::LevelFilter) {
    concat_log::set_level(level);
}

/// Ends the run's log in order; see [`concat_log::end`]. Every way out of
/// the app that is not a crash calls it, so a file without its last line
/// is a run that did not get the chance.
pub fn end() {
    concat_log::end();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one test that installs the process logger, because there is only
    /// one to install: everything from the facade down to a line in a file.
    #[test]
    fn a_run_opens_with_what_a_report_is_asked_first_and_ends_in_order() {
        let root = std::env::temp_dir().join(format!("concat-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a temp dir");
        let dirs = AppDirs::under(&root);

        let path = open(&dirs, None).expect("a log file");
        assert_eq!(path.parent(), Some(folder(&dirs).as_path()));
        assert_eq!(current(), Some(path.as_path()));

        log::warn!("the kettle is broken");
        // No flush: a line is in the file when the call that logged it
        // returns.
        let written = std::fs::read_to_string(&path).expect("reads back");
        assert!(written.contains(env!("CARGO_PKG_VERSION")), "{written}");
        for topic in ["build", "system", "cpu", "log"] {
            assert!(
                written
                    .lines()
                    .any(|line| line.contains(&format!(" INFO  {topic:<10} "))),
                "no {topic} line in {written}"
            );
        }
        // Every line says when, how bad, what about, and where from.
        let line = written
            .lines()
            .find(|line| line.contains("the kettle is broken"))
            .expect("the line");
        assert!(
            line.contains(" WARN  tests      the kettle is broken  [concat_host::logs::tests, "),
            "{line}"
        );

        // A second call cannot take the facade a second time, and says so
        // rather than pretending.
        assert!(open(&dirs, None).is_err());

        // A panic is in the file, with its message and its place, by the
        // time the hook hands on to whatever ends the process: with
        // `panic = "abort"` that is the very next thing.
        catch_panics();
        let line = line!() + 1;
        let _ = std::thread::spawn(|| panic!("the kettle boiled dry")).join();
        let written = std::fs::read_to_string(&path).expect("reads back");
        let place = format!("{}:{line}:", file!());
        assert!(
            written.lines().any(|text| text.contains(" ERROR panic ")
                && text.contains("the kettle boiled dry, at ")
                && text.contains(&place)),
            "{written}"
        );

        end();
        let written = std::fs::read_to_string(&path).expect("reads back");
        assert!(
            written
                .lines()
                .last()
                .is_some_and(|line| line.contains("run ended cleanly")),
            "{written}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_hangs_off_the_data_directory() {
        let dirs = AppDirs::under(Path::new("/tmp/concat-test"));
        assert_eq!(folder(&dirs), Path::new("/tmp/concat-test/logs"));
    }
}
