// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Shared plumbing for the FFmpeg children this crate spawns.
//!
//! Every invocation goes through [`base_command`], and every long-lived child
//! drains its stderr through [`StderrTail`]. The tail is why: a packaged app
//! has no terminal, so an inherited stderr sends FFmpeg's one useful line into
//! the void and leaves the user with "exited with status 1". Draining on a
//! thread also means the pipe can never fill up and stall the child.

use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::error::{Error, Result};

/// How much of the child's stderr to keep. At `-loglevel error` FFmpeg says
/// little; this holds the message that matters plus its context.
const STDERR_KEEP: usize = 8 * 1024;

/// A `Command` for `binary` that never flashes a console window.
///
/// On Windows, a GUI-subsystem parent has no console, so each console-subsystem
/// child (FFmpeg, ffprobe, ...) gets a fresh one that pops up over the app.
/// `CREATE_NO_WINDOW` suppresses that. Elsewhere this is just `Command::new`.
pub fn command(binary: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(binary.as_ref());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// A command with the flags every invocation wants: no banner, no terminal
/// interaction, errors only.
pub(crate) fn base_command(binary: &Path) -> Command {
    let mut command = command(binary);
    command.args(["-hide_banner", "-nostdin", "-loglevel", "error"]);
    command
}

/// Collects a child's stderr on a drain thread, keeping the tail for error
/// messages.
pub(crate) struct StderrTail {
    buffer: Arc<Mutex<Vec<u8>>>,
    handle: Option<JoinHandle<()>>,
}

impl StderrTail {
    /// Starts draining `child`'s stderr, which must have been spawned with
    /// `Stdio::piped()`. A child whose stderr was not piped yields an empty
    /// tail rather than an error.
    pub(crate) fn drain(child: &mut Child) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let handle = child.stderr.take().map(|mut stderr| {
            let buffer = Arc::clone(&buffer);
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                loop {
                    match stderr.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(read) => {
                            let mut buffer = buffer.lock().expect("stderr buffer poisoned");
                            buffer.extend_from_slice(&chunk[..read]);
                            let excess = buffer.len().saturating_sub(STDERR_KEEP);
                            if excess > 0 {
                                buffer.drain(..excess);
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
        });
        Self { buffer, handle }
    }

    /// The last few lines the child printed, joined onto one line.
    ///
    /// Call only after the child has been waited on: EOF on the pipe is what
    /// lets the drain thread finish, and joining it first is what makes the
    /// buffer complete.
    pub(crate) fn summary(&mut self) -> String {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let buffer = self.buffer.lock().expect("stderr buffer poisoned");
        summarize(&buffer)
    }
}

/// The last three non-empty lines, joined onto one. What an error message has
/// room for, and with `-loglevel error` usually the whole story.
pub(crate) fn summarize(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let mut lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    let tail = lines.split_off(lines.len().saturating_sub(3));
    if tail.is_empty() {
        "no error output captured".to_owned()
    } else {
        tail.join(" / ")
    }
}

/// Runs a prepared command to completion, capturing stderr, and turns a
/// non-zero exit into [`Error::Exited`] that carries what the child said.
pub(crate) fn run_to_completion(
    command: &mut Command,
    program: &'static str,
    path: &Path,
) -> Result<()> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| Error::Spawn { program, source })?;

    if output.status.success() {
        return Ok(());
    }
    Err(Error::Exited {
        program,
        path: path.to_path_buf(),
        status: output.status,
        stderr: summarize(&output.stderr),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_keeps_the_last_three_lines() {
        let text = b"one\ntwo\n\nthree\nfour\n";
        assert_eq!(summarize(text), "two / three / four");
    }

    #[test]
    fn summarize_of_nothing_says_so() {
        assert_eq!(summarize(b""), "no error output captured");
        assert_eq!(summarize(b"\n \n"), "no error output captured");
    }
}
