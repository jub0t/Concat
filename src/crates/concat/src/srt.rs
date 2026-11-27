// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! SubRip subtitles: the `.srt` files the Captions panel reads and writes.
//!
//! A file is a list of blocks parted by blank lines. Each block is a
//! counter, a timing line - `00:01:02,345 --> 00:01:04,000`, hours,
//! minutes, seconds and milliseconds - and the words, one or more lines.
//! Reading is lenient, as players are: a missing counter, a full stop for
//! the comma, a byte-order mark, Windows line ends and whatever follows
//! the end time on its line are all taken. A block with no timing line is
//! passed over.
//!
//! Times are kept in ticks; a millisecond is a whole number of them, so a
//! file read and written again keeps every time it had.

use concat_doc::model::{LaneId, Model};
use concat_doc::{TICKS_PER_SECOND, Tick};

use crate::doc_view::{self, Kind};

/// Ticks in a millisecond.
const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;

/// One subtitle: when it comes up and goes, and its words, a line break
/// between each of its lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: Tick,
    pub end: Tick,
    pub text: String,
}

/// Every cue in a file's text, in the file's order. Text with no timing
/// line in it has no cues.
pub fn parse(text: &str) -> Vec<Cue> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut cues = Vec::new();
    let mut lines = text
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .peekable();
    while lines.peek().is_some() {
        // One block: everything up to the next blank line.
        let mut block = Vec::new();
        for line in lines.by_ref() {
            if line.trim().is_empty() {
                if block.is_empty() {
                    continue;
                }
                break;
            }
            block.push(line);
        }
        let Some(at) = block.iter().position(|line| timing(line).is_some()) else {
            continue;
        };
        let Some((start, end)) = timing(block[at]) else {
            continue;
        };
        let words: Vec<&str> = block[at + 1..].iter().map(|line| line.trim_end()).collect();
        cues.push(Cue {
            start,
            end,
            text: words.join("\n"),
        });
    }
    cues
}

/// The two times of a timing line, if it is one.
fn timing(line: &str) -> Option<(Tick, Tick)> {
    let (from, rest) = line.split_once("-->")?;
    let to = rest.split_whitespace().next()?;
    Some((time(from.trim())?, time(to)?))
}

/// `HH:MM:SS,mmm` - or with a full stop - as a tick. The hours may be
/// any number of digits, and the fraction fewer than three.
fn time(text: &str) -> Option<Tick> {
    let (clock, fraction) = text.split_once([',', '.']).unwrap_or((text, "0"));
    let mut parts = clock.split(':');
    let (hours, minutes, seconds) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let number = |digits: &str| -> Option<i64> {
        (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
            .then(|| digits.parse().ok())
            .flatten()
    };
    let (hours, minutes, seconds) = (number(hours)?, number(minutes)?, number(seconds)?);
    let fraction = fraction.get(..fraction.len().min(3))?;
    let millis = number(fraction)? * 10_i64.pow(3 - fraction.len() as u32);
    let total = ((hours * 60 + minutes) * 60 + seconds)
        .checked_mul(1000)?
        .checked_add(millis)?;
    Some(Tick::new(total.checked_mul(TICKS_PER_MS)?))
}

/// A tick as `HH:MM:SS,mmm`, to the nearest millisecond; before zero is
/// zero.
fn clock(t: Tick) -> String {
    let millis = (t.get().max(0) + TICKS_PER_MS / 2) / TICKS_PER_MS;
    let (seconds, millis) = (millis / 1000, millis % 1000);
    format!(
        "{:02}:{:02}:{:02},{millis:03}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// Cues as a SubRip file, numbered from one. A blank line inside a cue's
/// words would end it early in any reader, so it is left out.
pub fn write(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (index, cue) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n",
            index + 1,
            clock(cue.start),
            clock(cue.end)
        ));
        for line in cue.text.lines().filter(|line| !line.trim().is_empty()) {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// The titles of the sequence showing as cues, in the order they come up:
/// those on `lane`, or on every lane with none named.
pub fn title_cues(doc: &Model, lane: Option<LaneId>) -> Vec<Cue> {
    let mut cues: Vec<Cue> = doc_view::items(doc)
        .into_iter()
        .filter(|(_, item)| lane.is_none_or(|lane| item.lane == lane))
        .filter_map(|(_, item)| {
            let title = doc_view::title(item)?;
            Some(Cue {
                start: item.start,
                end: doc_view::end(item),
                text: title.content.clone(),
            })
        })
        .collect();
    cues.sort_by_key(|cue| (cue.start, cue.end));
    cues
}

/// How many titles each lane of the sequence showing holds, bottom first,
/// leaving out the lanes with none.
fn title_counts(doc: &Model) -> Vec<(LaneId, usize)> {
    doc_view::lanes(doc)
        .into_iter()
        .map(|lane| {
            let titles = doc_view::on_lane(doc, lane)
                .filter(|(_, item)| doc_view::kind(doc, item) == Kind::Text)
                .count();
            (lane, titles)
        })
        .filter(|(_, titles)| *titles > 0)
        .collect()
}

/// How many lanes of the sequence showing hold a title.
pub fn title_lanes(doc: &Model) -> usize {
    title_counts(doc).len()
}

/// The lane whose titles are most likely the captions: a caption lane,
/// when one holds titles, else any lane; of those the one with the most
/// titles, and of lanes with as many the highest, where captions are
/// placed. None when no lane holds a title.
pub fn busiest_title_lane(doc: &Model) -> Option<LaneId> {
    let counts = title_counts(doc);
    let captions: Vec<(LaneId, usize)> = counts
        .iter()
        .copied()
        .filter(|(lane, _)| doc_view::lane(doc, *lane).is_some_and(|lane| lane.captions.is_some()))
        .collect();
    let pool = if captions.is_empty() {
        counts
    } else {
        captions
    };
    // Bottom first, so the last of the busiest is the highest.
    let most = pool.iter().map(|(_, titles)| *titles).max()?;
    pool.into_iter()
        .filter(|(_, titles)| *titles == most)
        .map(|(lane, _)| lane)
        .next_back()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: i64) -> Tick {
        Tick::new(millis * TICKS_PER_MS)
    }

    #[test]
    fn a_file_reads_as_its_cues() {
        let cues = parse(
            "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\nthere\r\n\r\n\
             2\r\n00:01:02.05 --> 00:01:03,000 X1:10 X2:20\r\nAgain\r\n",
        );
        assert_eq!(
            cues,
            vec![
                Cue {
                    start: ms(1000),
                    end: ms(2500),
                    text: "Hello\nthere".into(),
                },
                Cue {
                    start: ms(62_050),
                    end: ms(63_000),
                    text: "Again".into(),
                },
            ]
        );
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let cues = vec![
            Cue {
                start: ms(1000),
                end: ms(1100),
                text: "Olá\nAção".into(),
            },
            Cue {
                start: ms(3_725_000),
                end: ms(3_726_001),
                text: "Fim".into(),
            },
        ];
        let text = write(&cues);
        assert!(text.starts_with("1\n00:00:01,000 --> 00:00:01,100\nOlá\nAção\n\n2\n01:02:05,000"));
        assert_eq!(parse(&text), cues);
    }

    #[test]
    fn a_blank_line_inside_words_is_not_written_and_junk_has_no_cues() {
        let cue = Cue {
            start: ms(0),
            end: ms(1000),
            text: "One\n\nTwo".into(),
        };
        assert_eq!(parse(&write(&[cue]))[0].text, "One\nTwo");
        assert!(parse("not subtitles at all\n\nreally").is_empty());
        assert!(parse("").is_empty());
        assert!(parse("1\n00:00:xx,000 --> 00:00:01,000\nNo\n").is_empty());
    }

    #[test]
    fn the_busiest_lane_is_the_captions() {
        use concat_doc::commands::Command;
        let mut bench = doc_view::bench::Bench::new();
        assert_eq!(busiest_title_lane(bench.doc()), None);
        let (low, high) = (bench.main_lane(), bench.lane(0));
        let title = |bench: &mut doc_view::bench::Bench, lane, at: f64| {
            bench.apply(&Command::AddTextClip {
                start: doc_view::tick(at),
                len: None,
                lane: Some(lane),
                above: false,
                offset_y: None,
                title: None,
            });
        };
        title(&mut bench, low, 0.0);
        title(&mut bench, high, 0.0);
        assert_eq!(title_lanes(bench.doc()), 2);
        // As many on each: the higher.
        assert_eq!(busiest_title_lane(bench.doc()), Some(high));
        title(&mut bench, low, 10.0);
        assert_eq!(busiest_title_lane(bench.doc()), Some(low));
        assert_eq!(title_cues(bench.doc(), Some(low)).len(), 2);
        assert_eq!(title_cues(bench.doc(), None).len(), 3);
    }
}
