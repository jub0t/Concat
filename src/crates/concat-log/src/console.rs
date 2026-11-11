// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Standard error, for a run started from a terminal.
//!
//! The same lines the file gets, a line per write. In a terminal the level
//! is coloured and everything that is not the message is dimmed, so the eye
//! lands on what was said and on what went wrong; piped into a file or
//! another program they are plain, as `NO_COLOR` also asks
//! (<https://no-color.org>).

use std::io::{IsTerminal, Write};

/// Where console lines go, and whether they are coloured.
pub(crate) struct Console {
    colour: bool,
}

impl Console {
    pub(crate) fn new() -> Self {
        Console { colour: colour() }
    }

    pub(crate) fn colour(&self) -> bool {
        self.colour
    }

    /// Writes whole lines - `text` ends in a newline - in one write, so two
    /// threads' lines do not land inside each other.
    pub(crate) fn write(&self, text: &str) {
        let mut out = std::io::stderr().lock();
        let _ = out.write_all(text.as_bytes());
    }
}

/// Whether to colour: never when `NO_COLOR` is set to anything, always when
/// `CLICOLOR_FORCE` is, and otherwise when standard error is a terminal
/// that takes the escapes.
fn colour() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if set("NO_COLOR") {
        return false;
    }
    if set("CLICOLOR_FORCE") {
        return true;
    }
    std::io::stderr().is_terminal() && escapes_work()
}

/// A terminal anywhere but Windows reads the escapes as they are.
#[cfg(not(windows))]
fn escapes_work() -> bool {
    std::env::var_os("TERM").is_none_or(|term| term != "dumb")
}

/// A Windows console reads them once it has been asked to, and says no
/// when it is one that cannot.
#[cfg(windows)]
fn escapes_work() -> bool {
    use windows_sys::Win32::System::Console::{
        ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE,
        SetConsoleMode,
    };
    // SAFETY: the handle is the process's own standard error, asked for
    // and used at once; the mode is read into a local and written back
    // with one bit added.
    unsafe {
        let handle = GetStdHandle(STD_ERROR_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return false;
        }
        mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
            || SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

/// The escape that starts a level's colour: errors bold red, warnings
/// yellow, the ordinary green, and the two quiet levels blue and grey.
pub(crate) fn paint(level: log::Level) -> &'static str {
    match level {
        log::Level::Error => "\x1b[1;31m",
        log::Level::Warn => "\x1b[33m",
        log::Level::Info => "\x1b[32m",
        log::Level::Debug => "\x1b[34m",
        log::Level::Trace => "\x1b[90m",
    }
}

/// Dimmed, for what frames a line: its time, its thread, its module.
pub(crate) const DIM: &str = "\x1b[2m";

/// Back to the terminal's own colours.
pub(crate) const RESET: &str = "\x1b[0m";
