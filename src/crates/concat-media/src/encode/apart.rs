// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The system's encoders asked whether they open, in a process of their
//! own.
//!
//! Whether a chip's encoder opens is found out only by opening it, and
//! opening it runs the graphics driver's code in whatever process asks. A
//! driver that crashes there takes that process with it, and it used to be
//! the editor, at launch, before the window had answered a click. So the
//! asking is done by a helper: the same program started again with a flag,
//! which opens each encoder in turn and says on its output what it is
//! about to try and what it found. If the helper dies or hangs, the
//! encoder it was trying is the one that did it: that one is put down as
//! not opening, and a new helper asks the rest. The editor only ever
//! reads lines.
//!
//! The lines, one a line:
//!
//! ```text
//! concat-probe 1        the helper is up, and speaks this
//! try h264_mf           about to open it
//! is h264_mf 1          it opened (0: it did not)
//! end                   every question is answered
//! ```

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// The flag a program is started with to be the helper: it then calls
/// [`answer`] and exits.
pub const FLAG: &str = "--probe-encoders";

/// The helper's first line.
const GREETING: &str = "concat-probe 1";

/// The environment variable that names, comma between them, the encoders
/// a helper is not to open: the ones that took an earlier helper down.
pub const SKIP: &str = "CONCAT_PROBE_SKIP";

/// How long a helper may say nothing before it is taken for hung: a
/// phone's slowest encoder opens in a second or two.
const QUIET: Duration = Duration::from_secs(20);

/// The helper's side: opens every encoder the export sheet asks about,
/// but the ones [`SKIP`] names, and says each on standard output as
/// [`ask`] reads it. The program calls this and exits when started with
/// its probe flag, before it opens a window or anything else.
pub fn answer() {
    fn say(line: std::fmt::Arguments) {
        let mut out = std::io::stdout().lock();
        // A reader that has gone leaves nobody to tell.
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
    for name in std::env::var(SKIP).unwrap_or_default().split(',') {
        if let Some(name) = super::linked_name(name) {
            super::remember(name, false);
        }
    }
    say(format_args!("{GREETING}"));
    let _ = super::TOLD.set(|name, answer| match answer {
        None => say(format_args!("try {name}")),
        Some(opened) => say(format_args!("is {name} {}", u8::from(opened))),
    });
    super::probe_encoders();
    say(format_args!("end"));
}

/// What [`ask`] found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// The encoders whose opening took a helper down, or hung it: put
    /// down as not opening here.
    pub took_down: Vec<String>,
}

/// The editor's side: has `program`, started with `arguments`, answer for
/// every encoder, and remembers what it says as if this process had asked
/// ([`super::probe_encoders`] after this opens nothing). An encoder that
/// takes the helper down is remembered as not opening, and the rest are
/// asked of a new helper. An error when no helper could be started or
/// none spoke: nothing is remembered then, and the caller asks the old
/// way or not at all.
pub fn ask(program: &Path, arguments: &[&str]) -> Result<Asked, String> {
    let took_down = ask_with(
        |skip| {
            let mut command = Command::new(program);
            command.args(arguments).env(SKIP, skip);
            command
        },
        QUIET,
        |name, opened| {
            if let Some(name) = super::linked_name(name) {
                super::remember(name, opened);
            }
        },
    )?;
    for name in &took_down {
        log::error!(
            "export: asking whether {name} opens took the helper down; it is left out of this run's exports"
        );
    }
    Ok(Asked { took_down })
}

/// [`ask`] with the helper's command, the wait and the remembering given:
/// `helper` makes the command for a round from the names to skip, and
/// `heard` is told each answer. The names that took a helper down.
fn ask_with(
    mut helper: impl FnMut(&str) -> Command,
    quiet: Duration,
    mut heard: impl FnMut(&str, bool),
) -> Result<Vec<String>, String> {
    let mut took_down: Vec<String> = Vec::new();
    // A helper is lost at most once per encoder: more rounds than there
    // are encoders anywhere is a helper that dies for another reason.
    for _ in 0..64 {
        let mut command = helper(&took_down.join(","));
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // No console window for the moment the helper lives.
            command.creation_flags(0x0800_0000);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("the helper did not start: {error}"))?;
        let output = child.stdout.take().ok_or("the helper has no output")?;
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });

        let mut greeted = false;
        let mut trying: Option<String> = None;
        let mut ended = false;
        // Until it is gone, or silent too long to be opening anything.
        while let Ok(line) = lines.recv_timeout(quiet) {
            let mut words = line.split_whitespace();
            match (words.next(), words.next(), words.next()) {
                _ if line == GREETING => greeted = true,
                (Some("try"), Some(name), None) if greeted => trying = Some(name.to_owned()),
                (Some("is"), Some(name), Some(opened)) if greeted => {
                    heard(name, opened == "1");
                    trying = None;
                }
                (Some("end"), None, None) if greeted => {
                    ended = true;
                    break;
                }
                // Whatever else a library prints is not for us.
                _ => {}
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        if ended {
            return Ok(took_down);
        }
        if !greeted {
            return Err("the helper said nothing".to_owned());
        }
        match trying {
            Some(name) => {
                heard(&name, false);
                took_down.push(name);
            }
            None => return Err("the helper stopped between two encoders".to_owned()),
        }
    }
    Err("the helper kept stopping".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How the stand-in helper below is told to behave.
    const PLAY: &str = "CONCAT_PROBE_TEST_HELPER";

    /// Not a test of anything: the helper the tests below start, which is
    /// this test binary run again for this one test with [`PLAY`] set. It
    /// answers for three encoders, a, b and c, and does to b what [`PLAY`]
    /// says, unless told to skip it.
    #[test]
    fn stand_in_helper() {
        let Ok(play) = std::env::var(PLAY) else {
            return;
        };
        let skip = std::env::var(SKIP).unwrap_or_default();
        let say = |line: &str| {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        };
        if play == "silent" {
            std::process::exit(0);
        }
        say(GREETING);
        for name in ["a", "b", "c"] {
            if skip.split(',').any(|skipped| skipped == name) {
                continue;
            }
            say(&format!("try {name}"));
            if name == "b" {
                match play.as_str() {
                    "crash" => std::process::abort(),
                    "hang" => std::thread::sleep(Duration::from_secs(600)),
                    _ => {}
                }
            }
            say(&format!("is {name} 1"));
        }
        if play == "stop" {
            std::process::exit(0);
        }
        say("end");
        std::process::exit(0);
    }

    /// What was heard of the encoders, in order.
    type Heard = Vec<(String, bool)>;

    /// Asks the stand-in helper, playing `play`; what was heard, in order,
    /// and how it ended.
    fn asked(play: &str) -> (Heard, Result<Vec<String>, String>) {
        let program = std::env::current_exe().expect("the test binary");
        let mut heard = Vec::new();
        let ended = ask_with(
            |skip| {
                let mut command = Command::new(&program);
                command
                    .args([
                        "--exact",
                        "encode::apart::tests::stand_in_helper",
                        "--nocapture",
                    ])
                    .env(PLAY, play)
                    .env(SKIP, skip);
                command
            },
            Duration::from_millis(1500),
            |name, opened| heard.push((name.to_owned(), opened)),
        );
        (heard, ended)
    }

    fn heard(pairs: &[(&str, bool)]) -> Heard {
        pairs
            .iter()
            .map(|(name, opened)| ((*name).to_owned(), *opened))
            .collect()
    }

    #[test]
    fn a_helper_that_answers_everything_is_heard_in_full() {
        let (answers, ended) = asked("well");
        assert_eq!(ended, Ok(Vec::new()));
        assert_eq!(answers, heard(&[("a", true), ("b", true), ("c", true)]));
    }

    /// The driver under encoder b crashes the process that opens it. The
    /// helper dies, b is put down as not opening, and a second helper
    /// answers for the rest: the process that asked is still here.
    #[test]
    fn an_encoder_that_crashes_the_helper_is_put_down_and_the_rest_are_asked() {
        let (answers, ended) = asked("crash");
        assert_eq!(ended, Ok(vec!["b".to_owned()]));
        assert_eq!(
            answers,
            heard(&[("a", true), ("b", false), ("a", true), ("c", true)])
        );
    }

    #[test]
    fn an_encoder_that_hangs_the_helper_is_put_down_the_same_way() {
        let (answers, ended) = asked("hang");
        assert_eq!(ended, Ok(vec!["b".to_owned()]));
        assert_eq!(answers.last(), Some(&("c".to_owned(), true)));
        assert!(answers.contains(&("b".to_owned(), false)));
    }

    #[test]
    fn a_helper_that_never_speaks_or_stops_short_is_an_error() {
        assert!(asked("silent").1.is_err());
        let (answers, ended) = asked("stop");
        assert!(ended.is_err(), "{ended:?}");
        assert_eq!(answers.len(), 3);
    }
}
