// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! SubRip (`.srt`) subtitles: read into cues, written from them, and the
//! cues a lane of title clips makes.

use thiserror::Error;

use crate::model::{ClipKind, Timeline};

/// One subtitle: when it shows, when it goes, and its words. A cue's text
/// may run over several lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    /// Seconds from the start.
    pub start: f64,
    /// Seconds from the start; never before `start`.
    pub end: f64,
    /// The words, a line break between lines.
    pub text: String,
}

/// Why a file could not be read as SubRip, and the line it stopped on,
/// counted from 1.
#[derive(Clone, Debug, Error, PartialEq)]
#[error("line {line}: {reason}")]
pub struct SrtError {
    /// The line, counted from 1.
    pub line: usize,
    /// What was wrong with it.
    pub reason: String,
}

/// Reads SubRip text. A byte-order mark, CRLF or CR line ends, a missing
/// blank line at the end, a missing cue number and a dot for the comma
/// before the milliseconds are all taken.
pub fn parse(text: &str) -> Result<Vec<Cue>, SrtError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = text
        .lines()
        .enumerate()
        .map(|(at, line)| (at + 1, line))
        .peekable();
    let mut cues = Vec::new();
    loop {
        while lines.next_if(|(_, line)| line.trim().is_empty()).is_some() {}
        let Some((at, first)) = lines.next() else {
            break;
        };
        // The cue's number is optional; its times are not.
        let (at, timing) = if first.contains("-->") {
            (at, first)
        } else if first.trim().parse::<u64>().is_ok() {
            match lines.next_if(|(_, line)| !line.trim().is_empty()) {
                Some(next) => next,
                None => return Err(error(at, "a cue number with no times after it")),
            }
        } else {
            return Err(error(at, "expected a cue number or its times"));
        };
        let (start, end) = times(timing)
            .ok_or_else(|| error(at, "expected times as 00:00:00,000 --> 00:00:00,000"))?;
        if end < start {
            return Err(error(at, "the cue ends before it starts"));
        }
        let mut words = Vec::new();
        while let Some((_, line)) = lines.next_if(|(_, line)| !line.trim().is_empty()) {
            words.push(line.trim_end());
        }
        cues.push(Cue {
            start,
            end,
            text: words.join("\n"),
        });
    }
    Ok(cues)
}

fn error(line: usize, reason: &str) -> SrtError {
    SrtError {
        line,
        reason: reason.to_owned(),
    }
}

/// A timing line's start and end, in seconds. Anything after the end time
/// - the position some writers add - is ignored.
fn times(line: &str) -> Option<(f64, f64)> {
    let (start, rest) = line.split_once("-->")?;
    let end = rest.split_whitespace().next()?;
    Some((timestamp(start.trim())?, timestamp(end)?))
}

/// `HH:MM:SS,mmm`, or with a dot, as seconds. Hours may run past two
/// digits; minutes and seconds stay under 60.
fn timestamp(text: &str) -> Option<f64> {
    let number = |part: &str| {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u64>().ok())
            .flatten()
    };
    let (clock, fraction) = text.split_once([',', '.'])?;
    let mut parts = clock.split(':');
    let (hours, minutes, seconds) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || fraction.len() > 3 {
        return None;
    }
    let (hours, minutes, seconds) = (number(hours)?, number(minutes)?, number(seconds)?);
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    // ",5" is half a second, as ".5" would be.
    let millis = number(fraction)? * 10u64.pow(3 - fraction.len() as u32);
    let total = hours
        .checked_mul(3_600_000)?
        .checked_add(minutes * 60_000 + seconds * 1_000 + millis)?;
    Some(total as f64 / 1_000.0)
}

/// Writes cues as SubRip, numbered from 1. A blank line inside a cue's
/// text would end the cue, so blank lines are left out.
pub fn write(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (index, cue) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n",
            index + 1,
            stamp(cue.start),
            stamp(cue.end)
        ));
        for line in cue.text.lines().map(str::trim_end) {
            if !line.is_empty() {
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push('\n');
    }
    out
}

/// Seconds as `HH:MM:SS,mmm`, to the nearest millisecond; below zero is
/// zero.
fn stamp(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1_000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        millis / 3_600_000,
        millis / 60_000 % 60,
        millis / 1_000 % 60,
        millis % 1_000
    )
}

/// The cues the title clips on `track_id` make - on every lane, for None -
/// in the order they play: each title's words, from where it starts to
/// where it ends.
pub fn title_cues(timeline: &Timeline, track_id: Option<&str>) -> Vec<Cue> {
    let mut cues: Vec<Cue> = timeline
        .clips
        .iter()
        .filter(|clip| clip.kind == ClipKind::Text)
        .filter(|clip| track_id.is_none_or(|id| clip.track_id == id))
        .filter_map(|clip| {
            Some(Cue {
                start: clip.start,
                end: clip.start + clip.duration,
                text: clip.text.as_ref()?.content.clone(),
            })
        })
        .collect();
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

/// The lane with the most titles on it - the first such lane, top down,
/// when two have as many - or None when the timeline has no title.
///
/// A title made as a caption is a title like any other: nothing in the
/// document says which lane is the captions'. But captions come many to
/// a lane, one after another, where a title card is one or two; so with
/// no title selected to name a lane, the busiest lane is the captions'.
/// A title card on a lane of its own is never written out with them.
pub fn busiest_title_lane(timeline: &Timeline) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for track in &timeline.tracks {
        let count = timeline
            .clips
            .iter()
            .filter(|clip| clip.kind == ClipKind::Text && clip.track_id == track.id)
            .count();
        if count > 0 && best.is_none_or(|(_, most)| count > most) {
            best = Some((&track.id, count));
        }
    }
    best.map(|(id, _)| id.to_owned())
}

/// How many lanes have titles on them.
pub fn title_lanes(timeline: &Timeline) -> usize {
    timeline
        .tracks
        .iter()
        .filter(|track| {
            timeline
                .clips
                .iter()
                .any(|clip| clip.kind == ClipKind::Text && clip.track_id == track.id)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::{Cue, busiest_title_lane, parse, title_cues, title_lanes, write};
    use crate::commands::Command;
    use crate::editor::Editor;
    use crate::model::TextStyle;

    fn cue(start: f64, end: f64, text: &str) -> Cue {
        Cue {
            start,
            end,
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_plain_file_reads_as_its_cues() {
        let cues = parse(
            "1\n00:00:01,000 --> 00:00:02,500\nHello there.\n\n\
             2\n00:01:02,250 --> 01:00:00,000\nTwo\nlines\n\n",
        )
        .expect("parses");
        assert_eq!(
            cues,
            [
                cue(1.0, 2.5, "Hello there."),
                cue(62.25, 3600.0, "Two\nlines")
            ]
        );
    }

    #[test]
    fn a_bom_crlf_and_no_trailing_blank_line_are_taken() {
        let cues = parse("\u{feff}1\r\n00:00:01,000 --> 00:00:02,000\r\nHi\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\nBye")
            .expect("parses");
        assert_eq!(cues, [cue(1.0, 2.0, "Hi"), cue(3.0, 4.0, "Bye")]);
    }

    #[test]
    fn a_dot_before_the_milliseconds_reads_like_a_comma() {
        let cues = parse("1\n00:00:01.200 --> 00:00:02.300\nDot\n").expect("parses");
        assert_eq!(cues, [cue(1.2, 2.3, "Dot")]);
    }

    #[test]
    fn extra_blank_lines_a_missing_number_and_trailing_coordinates_are_taken() {
        let cues = parse(
            "\n\n00:00:01,000 --> 00:00:02,000 X1:10 X2:20 Y1:5 Y2:9\nNo number\n\n\n\n\
             7\n  00:00:03,000-->00:00:04,000  \nSpaced\n",
        )
        .expect("parses");
        assert_eq!(cues, [cue(1.0, 2.0, "No number"), cue(3.0, 4.0, "Spaced")]);
    }

    #[test]
    fn an_empty_file_has_no_cues() {
        assert_eq!(parse(""), Ok(Vec::new()));
        assert_eq!(parse("\u{feff}\r\n  \n"), Ok(Vec::new()));
    }

    #[test]
    fn a_cue_with_no_words_is_kept_empty() {
        let cues = parse("1\n00:00:01,000 --> 00:00:02,000\n\n").expect("parses");
        assert_eq!(cues, [cue(1.0, 2.0, "")]);
    }

    #[test]
    fn malformed_files_are_errors_that_name_the_line() {
        let cases = [
            ("1\nnot a time\nHi\n", 2),
            ("1\n00:00:01,000 -> 00:00:02,000\nHi\n", 2),
            ("1\n00:00:01,000 --> \nHi\n", 2),
            ("1\n00:00:61,000 --> 00:01:02,000\nHi\n", 2),
            ("1\n00:60:00,000 --> 01:00:00,000\nHi\n", 2),
            ("1\n00:00:01,0000 --> 00:00:02,000\nHi\n", 2),
            ("1\n00:00:01 --> 00:00:02\nHi\n", 2),
            ("1\n00:00:-1,000 --> 00:00:02,000\nHi\n", 2),
            ("1\n00:00:03,000 --> 00:00:02,000\nHi\n", 2),
            ("1\n", 1),
            ("Hello\nworld\n", 1),
            ("1\n00:00:01,000 --> 00:00:02,000\nHi\n\nstray\n", 5),
            ("1\n00:00:01,000 --> 00:00:02,000\nHi\n\n2\n", 5),
            (
                "1\n99999999999999999999:00:00,000 --> 00:00:02,000\nHi\n",
                2,
            ),
        ];
        for (text, line) in cases {
            let error = parse(text).expect_err(text);
            assert_eq!(error.line, line, "{text:?}: {error}");
        }
    }

    #[test]
    fn odd_bytes_never_panic() {
        for text in [
            "-->",
            "1\n-->\n",
            "1\n:::,--> :::,\n",
            "1\n00:00:01,ü00 --> 00:00:02,000\n",
            "1\n0ü:00:01,000 --> 00:00:02,000\n",
            "\u{feff}",
            "\r\r\r",
        ] {
            let _ = parse(text);
        }
    }

    #[test]
    fn cues_are_written_numbered_with_comma_milliseconds() {
        let text = write(&[cue(1.0, 2.5, "Hello"), cue(3661.007, 3662.0, "Two\nlines")]);
        assert_eq!(
            text,
            "1\n00:00:01,000 --> 00:00:02,500\nHello\n\n\
             2\n01:01:01,007 --> 01:01:02,000\nTwo\nlines\n\n"
        );
    }

    #[test]
    fn writing_drops_blank_lines_inside_a_cue_and_negative_times() {
        let text = write(&[cue(-1.0, 0.5, "One\n\n  \nTwo\r\n")]);
        assert_eq!(text, "1\n00:00:00,000 --> 00:00:00,500\nOne\nTwo\n\n");
    }

    #[test]
    fn written_cues_read_back_the_same() {
        let cues = vec![
            cue(0.0, 1.234, "First"),
            cue(1.5, 4.0, "Second\nline two"),
            cue(59.999, 61.001, "Minute"),
            cue(7200.0, 7201.5, "Two hours"),
        ];
        assert_eq!(parse(&write(&cues)), Ok(cues));
    }

    /// A lane's titles are its cues in the order they play; titles on
    /// other lanes are not.
    #[test]
    fn a_lanes_titles_are_its_cues_in_order() {
        let mut editor = Editor::new();
        let mut title = |start: f64, words: &str| {
            let id = editor
                .apply(Command::AddTextClip {
                    track_id: None,
                    above: true,
                    start,
                    style: Some(TextStyle {
                        content: words.to_owned(),
                        ..TextStyle::default()
                    }),
                    duration: Some(1.5),
                    offset_y: None,
                })
                .expect("adds")
                .created_id
                .expect("id");
            editor
                .project()
                .active()
                .clip(&id)
                .expect("exists")
                .track_id
                .clone()
        };
        let lane = title(4.0, "Second");
        assert_eq!(title(1.0, "First"), lane);
        let other = title(1.0, "Elsewhere");
        assert_ne!(other, lane);

        let timeline = editor.project().active();
        assert_eq!(
            title_cues(timeline, Some(&lane)),
            [cue(1.0, 2.5, "First"), cue(4.0, 5.5, "Second")]
        );
        assert_eq!(title_cues(timeline, None).len(), 3);
        // The captions' lane is the busy one, not the title card's.
        assert_eq!(busiest_title_lane(timeline).as_deref(), Some(lane.as_str()));
        assert_eq!(title_lanes(timeline), 2);
        assert_eq!(busiest_title_lane(Editor::new().project().active()), None);
    }
}
