// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! Word-by-word dynamic karaoke subtitle generation for Threads vertical videos.
//!
//! Converts Edge TTS `WordBoundary` timings (with a proportional fallback when
//! boundaries are absent) into contiguous native `Command::AddTextClip` items
//! using `TextRun` character-range highlighting for the active spoken word.

use concat_project::commands::Command;
use concat_project::model::{RunStyle, TextAlign, TextRun, TextStyle};

use crate::edge_tts::WordTiming;

/// Bottom-third placement (below the centered Threads card, above TikTok chrome).
pub const KARAOKE_OFFSET_Y: f64 = 0.36;
const MIN_WORD_CLIP_SECS: f64 = 0.04;
const MAX_WORDS_PER_CHUNK: usize = 4;
const MAX_CHARS_PER_CHUNK: usize = 26;

/// Visual highlight style for dynamic karaoke captions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KaraokeStyle {
    Off,
    #[default]
    ClassicGold,
    NeonCyan,
    TikTokPink,
}

impl KaraokeStyle {
    pub const ALL: &'static [KaraokeStyle] = &[
        KaraokeStyle::Off,
        KaraokeStyle::ClassicGold,
        KaraokeStyle::NeonCyan,
        KaraokeStyle::TikTokPink,
    ];

    pub fn from_index(index: i32) -> Self {
        Self::ALL
            .get(usize::try_from(index).unwrap_or(1))
            .copied()
            .unwrap_or(Self::ClassicGold)
    }

    pub fn index(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::ClassicGold => 1,
            Self::NeonCyan => 2,
            Self::TikTokPink => 3,
        }
    }

    /// Active-word highlight hex color, or `None` when karaoke is disabled.
    pub fn highlight_color(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::ClassicGold => Some("#ffe033"),
            Self::NeonCyan => Some("#38bdf8"),
            Self::TikTokPink => Some("#ff4d8d"),
        }
    }
}

/// Builds contiguous `Command::AddTextClip` commands on `track_id` for one
/// scene's spoken words, highlighting each word as it is spoken.
pub fn build_scene_karaoke_commands(
    track_id: &str,
    text: &str,
    scene_start: f64,
    audio_duration: f64,
    word_timings: &[WordTiming],
    style: KaraokeStyle,
) -> Vec<Command> {
    let Some(highlight_hex) = style.highlight_color() else {
        return Vec::new();
    };
    let effective_dur = audio_duration.max(0.2);
    let words = if !word_timings.is_empty() {
        normalize_timings(word_timings, effective_dur)
    } else {
        estimate_word_timings(text, effective_dur)
    };
    if words.is_empty() {
        return Vec::new();
    }

    let chunks = group_into_chunks(&words);
    let mut commands = Vec::with_capacity(words.len());

    for (chunk_idx, chunk) in chunks.iter().enumerate() {
        let next_chunk_start = chunks
            .get(chunk_idx + 1)
            .and_then(|next| next.first())
            .map(|w| w.start);

        let (chunk_text, char_ranges) = join_chunk_words(chunk);
        if chunk_text.is_empty() {
            continue;
        }

        for (i, word) in chunk.iter().enumerate() {
            let start_rel = word.start.clamp(0.0, effective_dur);
            let natural_end = (word.start + word.duration).min(effective_dur);
            let end_rel = if let Some(next_word) = chunk.get(i + 1) {
                next_word.start.clamp(start_rel + MIN_WORD_CLIP_SECS, effective_dur)
            } else if let Some(next_c) = next_chunk_start {
                natural_end.min(next_c).max(start_rel + MIN_WORD_CLIP_SECS)
            } else {
                natural_end.max(start_rel + MIN_WORD_CLIP_SECS)
            };

            let duration = (end_rel - start_rel).max(MIN_WORD_CLIP_SECS);
            let (char_start, char_end) = char_ranges[i];

            let text_style = TextStyle {
                content: chunk_text.clone(),
                font_family: "Hanken Grotesk".to_owned(),
                font_size: 0.048,
                font_weight: 800.0,
                color: "#ffffff".to_owned(),
                align: TextAlign::Center,
                stroke_width: 0.0045,
                stroke_color: "#0b0d14".to_owned(),
                shadow: true,
                background: "#0b0d14b8".to_owned(),
                background_radius: 0.012,
                background_padding_x: 0.022,
                background_padding_y: 0.012,
                line_height: 1.15,
                max_width: 0.86,
                runs: vec![TextRun {
                    start: char_start,
                    end: char_end,
                    style: RunStyle {
                        color: Some(highlight_hex.to_owned()),
                        font_weight: Some(900.0),
                        font_scale: Some(1.08),
                        ..RunStyle::default()
                    },
                }],
                ..TextStyle::default()
            };

            commands.push(Command::AddTextClip {
                track_id: Some(track_id.to_owned()),
                above: false,
                start: scene_start + start_rel,
                style: Some(text_style),
                duration: Some(duration),
                offset_y: Some(KARAOKE_OFFSET_Y),
            });
        }
    }

    commands
}

fn normalize_timings(raw: &[WordTiming], max_duration: f64) -> Vec<WordTiming> {
    let mut out: Vec<WordTiming> = raw
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .map(|w| WordTiming {
            start: w.start.max(0.0),
            duration: w.duration.max(MIN_WORD_CLIP_SECS),
            text: w.text.trim().to_owned(),
        })
        .collect();
    out.sort_by(|a, b| a.start.total_cmp(&b.start));

    // Scale timings down smoothly only if Edge's reported end slightly exceeds probed duration.
    if let Some(last) = out.last() {
        let reported_end = last.start + last.duration;
        if reported_end > max_duration + 0.05 && reported_end > 0.0 {
            let scale = max_duration / reported_end;
            for w in &mut out {
                w.start *= scale;
                w.duration = (w.duration * scale).max(MIN_WORD_CLIP_SECS);
            }
        }
    }
    out
}

fn estimate_word_timings(text: &str, duration: f64) -> Vec<WordTiming> {
    let tokens: Vec<&str> = text.split_whitespace().filter(|w| !w.is_empty()).collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let weights: Vec<f64> = tokens
        .iter()
        .map(|word| {
            let base = word.chars().count().max(1) as f64;
            let pause = if word.ends_with(['.', '!', '?']) {
                2.5
            } else if word.ends_with([',', ';', ':']) {
                1.2
            } else {
                0.0
            };
            base + pause
        })
        .collect();
    let total_weight: f64 = weights.iter().sum::<f64>().max(1.0);
    let mut cursor = 0.0f64;
    let mut out = Vec::with_capacity(tokens.len());
    for (word, weight) in tokens.into_iter().zip(weights) {
        let word_dur = (duration * (weight / total_weight)).max(MIN_WORD_CLIP_SECS);
        out.push(WordTiming {
            start: cursor,
            duration: word_dur,
            text: word.to_owned(),
        });
        cursor += word_dur;
    }
    out
}

fn group_into_chunks(words: &[WordTiming]) -> Vec<Vec<WordTiming>> {
    let mut chunks: Vec<Vec<WordTiming>> = Vec::new();
    let mut current: Vec<WordTiming> = Vec::new();
    let mut current_chars = 0usize;

    for word in words {
        let w_chars = word.text.chars().count();
        let next_chars = if current.is_empty() {
            w_chars
        } else {
            current_chars + 1 + w_chars
        };
        if !current.is_empty()
            && (current.len() >= MAX_WORDS_PER_CHUNK || next_chars > MAX_CHARS_PER_CHUNK)
        {
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current_chars = if current.is_empty() {
            w_chars
        } else {
            current_chars + 1 + w_chars
        };
        let ends_sentence = word.text.ends_with(['.', '!', '?', ';']);
        let ends_clause = word.text.ends_with([',', ':']) && current.len() >= 2;
        current.push(word.clone());
        if ends_sentence || ends_clause {
            chunks.push(std::mem::take(&mut current));
            current_chars = 0;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Joins chunk words with spaces and returns `(joined_text, vec_of_char_ranges)`
/// where each range counts Unicode scalar values (`str::chars`) as required by
/// [`TextRun`].
fn join_chunk_words(chunk: &[WordTiming]) -> (String, Vec<(usize, usize)>) {
    let mut text = String::new();
    let mut ranges = Vec::with_capacity(chunk.len());
    let mut char_cursor = 0usize;

    for (i, word) in chunk.iter().enumerate() {
        if i > 0 {
            text.push(' ');
            char_cursor += 1;
        }
        let start = char_cursor;
        let len = word.text.chars().count();
        text.push_str(&word.text);
        char_cursor += len;
        ranges.push((start, char_cursor));
    }
    (text, ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_vietnamese_words_using_unicode_scalar_indices() {
        let timings = vec![
            WordTiming { start: 0.0, duration: 0.3, text: "Thật".into() },
            WordTiming { start: 0.3, duration: 0.3, text: "sự".into() },
            WordTiming { start: 0.6, duration: 0.4, text: "không".into() },
            WordTiming { start: 1.0, duration: 0.3, text: "tin".into() },
        ];
        let cmds = build_scene_karaoke_commands(
            "t5",
            "Thật sự không tin",
            2.0,
            1.5,
            &timings,
            KaraokeStyle::ClassicGold,
        );
        assert_eq!(cmds.len(), 4);

        // Verify the 3rd word ("không") has exact char offsets 8..13 in "Thật sự không tin".
        let Command::AddTextClip {
            start,
            style: Some(style),
            duration: Some(dur),
            ..
        } = &cmds[2]
        else {
            panic!("expected AddTextClip");
        };
        assert_eq!(style.content, "Thật sự không tin");
        assert!((*start - 2.6).abs() < 1e-6);
        assert!((*dur - 0.4).abs() < 1e-6);
        assert_eq!(style.runs.len(), 1);
        let run = &style.runs[0];
        let highlighted: String = style
            .content
            .chars()
            .skip(run.start)
            .take(run.end - run.start)
            .collect();
        assert_eq!(highlighted, "không");
        assert_eq!(run.style.color.as_deref(), Some("#ffe033"));
    }

    #[test]
    fn falls_back_to_estimated_timings_and_respects_off_style() {
        assert!(
            build_scene_karaoke_commands("t5", "Hello world", 0.0, 2.0, &[], KaraokeStyle::Off)
                .is_empty()
        );
        let cmds = build_scene_karaoke_commands(
            "t5",
            "Một hai ba bốn năm sáu",
            1.0,
            3.0,
            &[],
            KaraokeStyle::NeonCyan,
        );
        assert_eq!(cmds.len(), 6);
        // Starts should be strictly non-decreasing.
        let mut prev_start = 0.0;
        for cmd in cmds {
            if let Command::AddTextClip { start, .. } = cmd {
                assert!(start >= prev_start);
                prev_start = start;
            }
        }
    }
}
