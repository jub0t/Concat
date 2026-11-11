// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How one line is laid out.
//!
//! ```text
//! 05:22:33.776 INFO  gpu        Apple M5 on Metal  [concat::gpu, main]
//! 05:23:10.551 INFO  captions   done in 43.2 s · small model · transcribe 40.9 s · 120 captions  [concat_speech::transcribe, concat-worker-2]
//! 05:23:11.900 ERROR gpu        the compositor's device was lost: the driver stopped answering  [concat_render::gpu, concat-worker-0]
//! ```
//!
//! Read from the left, a line is when, how bad, what about, and what
//! happened - the four things a person skimming a run wants, in columns, so
//! the messages start under one another. What it is *about* is the topic:
//! one word for the part of the app speaking (`gpu`, `export`, `captions`).
//! A message says its own topic by starting with it - `"export: started"` -
//! which is how the lines were already written; one that does not is filed
//! under the last name of the module it came from.
//!
//! Where it came from - the module and the thread - is at the far end, in
//! brackets: the first thing wanted when chasing a bug and the last thing
//! wanted when reading a story, so it is there and out of the way.

use std::fmt::Write as _;

use crate::console::{DIM, RESET, paint};

/// How wide the topic's column is. A longer topic pushes its own line out
/// and is not cut.
const TOPIC: usize = 10;

/// How far in a message's second and later lines sit: under the first.
const INDENT: usize = 12 + 1 + 5 + 1 + TOPIC + 1;

/// One line to lay out.
pub(crate) struct Line<'a> {
    pub(crate) at: (i64, u32),
    pub(crate) level: log::Level,
    pub(crate) target: &'a str,
    /// The thread that said it; empty when that is no longer known.
    pub(crate) thread: &'a str,
    pub(crate) body: &'a str,
    /// What the logger adds after the message: a count of repeats.
    pub(crate) suffix: &'a str,
}

/// How a line is dressed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dress {
    /// For a file: no escapes, and where it came from on every line.
    Plain,
    /// For a terminal: colours, and where it came from only on the lines
    /// that are chased - warnings, errors, and the two quiet levels.
    Coloured,
}

/// Lays `line` out onto the end of `out`, newline included.
pub(crate) fn compose(out: &mut String, line: &Line<'_>, dress: Dress) {
    let (topic, message) = topic(line.target, line.body);
    let coloured = dress == Dress::Coloured;
    if coloured {
        out.push_str(DIM);
    }
    crate::clock::time_into(out, line.at);
    if coloured {
        out.push_str(RESET);
        out.push(' ');
        out.push_str(paint(line.level));
        let _ = write!(out, "{:<5}", line.level);
        out.push_str(RESET);
        out.push(' ');
        out.push_str(tint(topic));
        let _ = write!(out, "{topic:<TOPIC$}");
        out.push_str(RESET);
        out.push(' ');
    } else {
        let _ = write!(out, " {:<5} {topic:<TOPIC$} ", line.level);
    }
    // An error is the line the eye should stop on: its words are painted too.
    let loud = coloured && line.level == log::Level::Error;
    if loud {
        out.push_str("\x1b[31m");
    }
    let mut pieces = message.split('\n');
    out.push_str(pieces.next().unwrap_or_default());
    out.push_str(line.suffix);
    if loud {
        out.push_str(RESET);
    }
    let sourced = !coloured || line.level != log::Level::Info;
    if sourced {
        if coloured {
            out.push_str(DIM);
        }
        out.push_str("  [");
        out.push_str(line.target);
        if !line.thread.is_empty() {
            out.push_str(", ");
            out.push_str(line.thread);
        }
        out.push(']');
        if coloured {
            out.push_str(RESET);
        }
    }
    out.push('\n');
    // A message of several lines - a panic with its backtrace - stays one
    // block, the rest of it under its first line.
    for piece in pieces {
        let _ = writeln!(out, "{:INDENT$}{piece}", "");
    }
}

/// The topic of a line and the message without it.
///
/// A message that opens `"export: "` - a few lowercase words and a colon -
/// has said its topic. Anything else is filed under where it came from.
pub(crate) fn topic<'a>(target: &'a str, body: &'a str) -> (&'a str, &'a str) {
    if let Some((head, rest)) = body.split_once(": ")
        && (2..=14).contains(&head.len())
        && head.starts_with(|first: char| first.is_ascii_lowercase())
        && head
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' || c == '-')
        && !rest.is_empty()
    {
        return (head, rest);
    }
    (place(target), body)
}

/// One word for where a line came from: the module's own name for one of
/// Concat's (`concat_media::hardware` is `hardware`, `concat_export` is
/// `export`), and the crate for anything else (`wgpu_hal::dx12` is
/// `wgpu_hal`), since a library's module names mean nothing to the reader.
fn place(target: &str) -> &str {
    if !target.starts_with("concat") {
        return target.split("::").next().unwrap_or(target);
    }
    let last = target.rsplit("::").next().unwrap_or(target);
    match last.strip_prefix("concat_") {
        Some(part) => part,
        None if last == "concat" => "app",
        None => last,
    }
}

/// A colour for a topic, the same one every time: a part of the app is
/// recognised down the column by its colour before its name is read. Red
/// and yellow are left to the levels.
fn tint(topic: &str) -> &'static str {
    const TINTS: [&str; 6] = [
        "\x1b[36m", "\x1b[35m", "\x1b[94m", "\x1b[96m", "\x1b[95m", "\x1b[92m",
    ];
    let sum = topic.bytes().fold(0usize, |sum, byte| {
        sum.wrapping_mul(31).wrapping_add(usize::from(byte))
    });
    TINTS[sum % TINTS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(level: log::Level, target: &str, thread: &str, body: &str, suffix: &str) -> String {
        let mut out = String::new();
        compose(
            &mut out,
            &Line {
                at: (1_726_316_096, 789),
                level,
                target,
                thread,
                body,
                suffix,
            },
            Dress::Plain,
        );
        out
    }

    #[test]
    fn a_line_is_when_how_bad_what_about_and_what_happened() {
        assert_eq!(
            plain(
                log::Level::Info,
                "concat::studio::edit",
                "main",
                "export: started",
                ""
            ),
            "12:14:56.789 INFO  export     started  [concat::studio::edit, main]\n"
        );
        assert_eq!(
            plain(
                log::Level::Warn,
                "concat_media::hardware",
                "concat-decode-1",
                "video decode on the CPU",
                " (+7 more in the last 5 s)"
            ),
            "12:14:56.789 WARN  hardware   video decode on the CPU (+7 more in the last 5 s)  [concat_media::hardware, concat-decode-1]\n"
        );
    }

    #[test]
    fn a_message_names_its_topic_or_its_module_does() {
        assert_eq!(topic("concat", "smoke: ok"), ("smoke", "ok"));
        assert_eq!(topic("concat", "log level: debug"), ("log level", "debug"));
        // Not a topic: a sentence that happens to have a colon in it.
        assert_eq!(
            topic(
                "concat::gpu",
                "CONCAT_GPU=software: the window draws in software"
            ),
            ("gpu", "CONCAT_GPU=software: the window draws in software")
        );
        assert_eq!(
            topic("concat_host::logs", "panic at src/lib.rs:12:5: it broke"),
            ("logs", "panic at src/lib.rs:12:5: it broke")
        );
        assert_eq!(topic("concat_export", "previews draw on Metal").0, "export");
        assert_eq!(topic("concat", "the window is up").0, "app");
        assert_eq!(
            topic("wgpu_hal::auxil::dxgi", "Map buffer failed").0,
            "wgpu_hal"
        );
    }

    #[test]
    fn a_message_of_several_lines_stays_one_block() {
        let out = plain(
            log::Level::Error,
            "concat",
            "main",
            "panic: it broke\n  at a\n  at b",
            "",
        );
        let mut lines = out.lines();
        assert_eq!(
            lines.next(),
            Some("12:14:56.789 ERROR panic      it broke  [concat, main]")
        );
        let under = " ".repeat(INDENT);
        assert_eq!(lines.next(), Some(format!("{under}  at a").as_str()));
        assert_eq!(lines.next(), Some(format!("{under}  at b").as_str()));
    }

    #[test]
    fn a_terminal_gets_colour_and_an_info_line_without_its_source() {
        let mut out = String::new();
        let line = Line {
            at: (0, 0),
            level: log::Level::Info,
            target: "concat::gpu",
            thread: "main",
            body: "gpu: Apple M5 on Metal",
            suffix: "",
        };
        compose(&mut out, &line, Dress::Coloured);
        assert!(out.contains("\x1b["), "{out:?}");
        assert!(out.contains("Apple M5 on Metal"));
        assert!(!out.contains("[concat::gpu"), "{out:?}");
        // The same topic is the same colour every time.
        assert_eq!(tint("gpu"), tint("gpu"));
    }
}
