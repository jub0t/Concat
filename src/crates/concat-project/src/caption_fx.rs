// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Motion and reveal for caption titles, built from what the document
//! already has.
//!
//! Two kinds of effect, because the title engine paints one picture per
//! clip and a clip has one colour:
//!
//! - **Motion** ([`Effect`]): fade, pop, slide and scale in. These are keys
//!   on the clip's own opacity, scale and offset, the same keys the
//!   inspector sets, so they play in the preview and the export alike and
//!   can be edited afterwards on the timeline.
//! - **Reveal** ([`Reveal`]): words or letters appearing over time. A title
//!   cannot hide part of its own words, so a reveal is a run of short title
//!   clips, each showing a little more (or one word). [`plan_reveal`] says
//!   what those pieces are; the caller lays them down.
//!
//! What a title cannot do - colour one word of a line differently while the
//! rest stays put - is not here, and not approximated: it needs per-word
//! styling in the text engine.

use crate::commands::Command;
use crate::model::ranges::MIN_CLIP_DURATION;
use crate::model::{Clip, ClipMask, KeyEase, KeyProperty, MaskShape};

/// Which way a slide comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    /// From the left edge.
    Left,
    /// From the right edge.
    Right,
    /// From above.
    Up,
    /// From below.
    Down,
}

/// A motion for a caption's entrance and, for fade, its exit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// No motion: any keys this module put on the clip are taken off.
    None,
    /// Opacity up over `fade_in` seconds and down over `fade_out`.
    Fade {
        /// Seconds to come in; zero for none.
        fade_in: f64,
        /// Seconds to go out; zero for none.
        fade_out: f64,
    },
    /// Grows from small, overshoots a little and settles.
    Pop {
        /// Seconds the whole move takes.
        seconds: f64,
        /// How far it overshoots, as a fraction of its size (0.1 is 10 %).
        overshoot: f64,
    },
    /// Slides in from a side while fading up.
    Slide {
        /// Seconds the move takes.
        seconds: f64,
        /// Where it comes from.
        from: Direction,
        /// How far it travels, as a fraction of the frame.
        distance: f64,
    },
    /// Grows from `from` times its size to its size while fading up.
    Scale {
        /// Seconds the move takes.
        seconds: f64,
        /// The starting size, as a fraction of its own (0.6 is 60 %).
        from: f64,
    },
    /// Drops in large, falls past its size and bounces back, settling.
    Bounce {
        /// Seconds the move takes.
        seconds: f64,
        /// How hard it bounces: 1 is the usual, 0.5 gentle, 2 wild.
        intensity: f64,
    },
    /// Springs in and rings about its size, each swing smaller.
    Elastic {
        /// Seconds the move takes.
        seconds: f64,
        /// How far it swings: 1 is the usual.
        intensity: f64,
    },
    /// A few quick sideways jumps and a jolt of size, then still: a signal
    /// glitching in.
    Glitch {
        /// Seconds the burst takes.
        seconds: f64,
        /// How far it jumps: 1 is the usual.
        intensity: f64,
    },
}

impl Effect {
    /// The motion a preset names, with its time and strength: "none",
    /// "fade", "pop", "slide" (or "slide-left", "slide-right", "slide-up",
    /// "slide-down"; plain "slide" comes from below), "scale", "bounce",
    /// "elastic" or "glitch". Anything else is no motion.
    pub fn named(name: &str, seconds: f64, intensity: f64) -> Effect {
        let seconds = seconds.clamp(0.05, 3.0);
        let intensity = intensity.clamp(0.1, 3.0);
        let slide = |from| Effect::Slide {
            seconds,
            from,
            distance: 0.06 * intensity,
        };
        match name.trim().to_ascii_lowercase().as_str() {
            "fade" => Effect::Fade {
                fade_in: seconds,
                fade_out: seconds,
            },
            "pop" => Effect::Pop {
                seconds,
                overshoot: 0.1 * intensity,
            },
            "slide" | "slide-up" => slide(Direction::Down),
            "slide-down" => slide(Direction::Up),
            "slide-left" => slide(Direction::Right),
            "slide-right" => slide(Direction::Left),
            "scale" => Effect::Scale {
                seconds,
                from: (1.0 - 0.3 * intensity).clamp(0.1, 1.0),
            },
            "bounce" => Effect::Bounce { seconds, intensity },
            "elastic" => Effect::Elastic { seconds, intensity },
            "glitch" => Effect::Glitch { seconds, intensity },
            _ => Effect::None,
        }
    }
}

/// The properties this module keys. Anything it puts on a clip is on these
/// alone, so taking them off takes off nothing else.
const KEYED: [KeyProperty; 4] = [
    KeyProperty::Opacity,
    KeyProperty::Scale,
    KeyProperty::OffsetX,
    KeyProperty::OffsetY,
];

/// How much of a clip an entrance may take: past this the title would never
/// sit still.
const MAX_SHARE: f64 = 0.45;

/// The commands that give `clip` the motion: first any keys on the four
/// properties are cleared, then the new ones are set, so applying an effect
/// twice, or another in its place, leaves exactly one.
pub fn commands(clip: &Clip, effect: &Effect) -> Vec<Command> {
    let mut out: Vec<Command> = KEYED
        .iter()
        .filter(|property| clip.keys.iter().any(|key| key.property == **property))
        .map(|property| Command::ClearClipKeys {
            clip_id: clip.id.clone(),
            property: *property,
        })
        .collect();
    let duration = clip.duration.max(MIN_CLIP_DURATION);
    let share = |seconds: f64| (seconds.max(0.0) / duration).min(MAX_SHARE);
    let mut key = |property: KeyProperty, at: f64, value: f64, ease: KeyEase| {
        out.push(Command::SetClipKey {
            clip_id: clip.id.clone(),
            property,
            at: at.clamp(0.0, 1.0),
            value,
            ease,
        });
    };
    let opacity = clip.opacity;
    let size = clip.scale;
    match *effect {
        Effect::None => {}
        Effect::Fade { fade_in, fade_out } => {
            let (a, b) = (share(fade_in), share(fade_out));
            if a > 0.0 {
                key(KeyProperty::Opacity, 0.0, 0.0, KeyEase::LINEAR);
                key(KeyProperty::Opacity, a, opacity, KeyEase::LINEAR);
            }
            if b > 0.0 {
                if a == 0.0 {
                    key(KeyProperty::Opacity, 0.0, opacity, KeyEase::LINEAR);
                }
                key(KeyProperty::Opacity, 1.0 - b, opacity, KeyEase::LINEAR);
                key(KeyProperty::Opacity, 1.0, 0.0, KeyEase::LINEAR);
            }
        }
        Effect::Pop { seconds, overshoot } => {
            let end = share(seconds);
            if end > 0.0 {
                let over = 1.0 + overshoot.clamp(0.0, 0.5);
                key(KeyProperty::Scale, 0.0, size * 0.6, KeyEase::OUT);
                key(KeyProperty::Scale, end * 0.65, size * over, KeyEase::OUT);
                key(KeyProperty::Scale, end, size, KeyEase::IN_OUT);
                key(KeyProperty::Opacity, 0.0, 0.0, KeyEase::LINEAR);
                key(KeyProperty::Opacity, end * 0.4, opacity, KeyEase::LINEAR);
            }
        }
        Effect::Slide {
            seconds,
            from,
            distance,
        } => {
            let end = share(seconds);
            if end > 0.0 {
                let d = distance.clamp(0.0, 1.0);
                let (property, base, start) = match from {
                    Direction::Left => (KeyProperty::OffsetX, clip.offset_x, clip.offset_x - d),
                    Direction::Right => (KeyProperty::OffsetX, clip.offset_x, clip.offset_x + d),
                    Direction::Up => (KeyProperty::OffsetY, clip.offset_y, clip.offset_y - d),
                    Direction::Down => (KeyProperty::OffsetY, clip.offset_y, clip.offset_y + d),
                };
                key(property, 0.0, start, KeyEase::OUT);
                key(property, end, base, KeyEase::OUT);
                key(KeyProperty::Opacity, 0.0, 0.0, KeyEase::LINEAR);
                key(KeyProperty::Opacity, end * 0.7, opacity, KeyEase::LINEAR);
            }
        }
        Effect::Bounce { seconds, intensity } => {
            let end = share(seconds);
            if end > 0.0 {
                let k = intensity.clamp(0.1, 3.0);
                // Large, then under its size, then over by less, then home.
                let steps = [
                    (0.0, 1.0 + 0.35 * k),
                    (0.35, 1.0 - 0.18 * k),
                    (0.6, 1.0 + 0.08 * k),
                    (0.8, 1.0 - 0.03 * k),
                    (1.0, 1.0),
                ];
                for (at, factor) in steps {
                    key(KeyProperty::Scale, end * at, size * factor, KeyEase::IN_OUT);
                }
                key(KeyProperty::Opacity, 0.0, 0.0, KeyEase::LINEAR);
                key(KeyProperty::Opacity, end * 0.25, opacity, KeyEase::LINEAR);
            }
        }
        Effect::Elastic { seconds, intensity } => {
            let end = share(seconds);
            if end > 0.0 {
                let k = intensity.clamp(0.1, 3.0);
                let steps = [
                    (0.0, 0.5),
                    (0.3, 1.0 + 0.25 * k),
                    (0.5, 1.0 - 0.12 * k),
                    (0.7, 1.0 + 0.06 * k),
                    (0.85, 1.0 - 0.02 * k),
                    (1.0, 1.0),
                ];
                for (at, factor) in steps {
                    key(KeyProperty::Scale, end * at, size * factor, KeyEase::OUT);
                }
                key(KeyProperty::Opacity, 0.0, 0.0, KeyEase::LINEAR);
                key(KeyProperty::Opacity, end * 0.2, opacity, KeyEase::LINEAR);
            }
        }
        Effect::Glitch { seconds, intensity } => {
            let end = share(seconds);
            if end > 0.0 {
                let d = 0.02 * intensity.clamp(0.1, 3.0);
                // Jumps, never glides: a straight line between near keys
                // is a jump at this speed.
                let steps = [
                    (0.0, -d),
                    (0.2, d * 1.5),
                    (0.4, -d * 0.5),
                    (0.6, d),
                    (0.8, -d * 0.25),
                    (1.0, 0.0),
                ];
                for (at, shift) in steps {
                    key(
                        KeyProperty::OffsetX,
                        end * at,
                        clip.offset_x + shift,
                        KeyEase::LINEAR,
                    );
                }
                key(KeyProperty::Scale, 0.0, size * 1.08, KeyEase::LINEAR);
                key(KeyProperty::Scale, end * 0.5, size * 0.97, KeyEase::LINEAR);
                key(KeyProperty::Scale, end, size, KeyEase::LINEAR);
                key(KeyProperty::Opacity, 0.0, 0.0, KeyEase::LINEAR);
                key(KeyProperty::Opacity, end * 0.1, opacity, KeyEase::LINEAR);
            }
        }
        Effect::Scale { seconds, from } => {
            let end = share(seconds);
            if end > 0.0 {
                key(
                    KeyProperty::Scale,
                    0.0,
                    size * from.clamp(0.1, 1.0),
                    KeyEase::OUT,
                );
                key(KeyProperty::Scale, end, size, KeyEase::OUT);
                key(KeyProperty::Opacity, 0.0, 0.0, KeyEase::LINEAR);
                key(KeyProperty::Opacity, end * 0.7, opacity, KeyEase::LINEAR);
            }
        }
    }
    out
}

/// How words or letters appear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reveal {
    /// The whole caption at once: one piece, no reveal.
    Whole,
    /// Each word appears and stays, so the line builds up as it is said.
    Build,
    /// One word at a time, each replacing the last.
    WordByWord,
    /// Letters appear one after another across the caption's time, at about
    /// `per_second` letters a second.
    Typewriter {
        /// Letters a second.
        per_second: f64,
    },
}

/// One title clip of a reveal: where it starts, how long it stays, and the
/// words it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Seconds on the timeline.
    pub start: f64,
    /// Seconds it is on screen.
    pub duration: f64,
    /// The words.
    pub content: String,
}

/// The name every clip of the highlight layer carries, so the tools that
/// work on the captions (export, motion, style) leave them alone, and
/// "none" can find them to take them off.
pub const HIGHLIGHT_NAME: &str = "caption-highlight";

/// A word's box on the title's canvas: `(x, y, width, height)` in pixels.
pub type WordBox = (i32, i32, u32, u32);

/// Whether a text is written right to left: its first letter decides, as
/// in the paragraph direction rule, so a Persian sentence with an English
/// word in it is right to left and an English one with a Persian word in
/// it is not.
pub fn is_rtl(text: &str) -> bool {
    text.chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(|c| matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}'))
}

/// The words' boxes in the order they are spoken. The layout's own order is
/// not relied on, only where the boxes are: lines top to bottom, and within
/// a line left to right for left-to-right writing, right to left for
/// Persian, Arabic and Hebrew. A word of the other script inside a line sits
/// where the shaper put it, so for a mixed line the order is the visual one.
pub fn spoken_order(boxes: &[WordBox], rtl: bool) -> Vec<WordBox> {
    let mut rows: Vec<Vec<WordBox>> = Vec::new();
    let mut sorted: Vec<WordBox> = boxes.to_vec();
    sorted.sort_by_key(|word| word.1);
    for word in sorted {
        match rows.last_mut() {
            Some(row) if (word.1 - row[0].1).unsigned_abs() * 2 < row[0].3.max(1) => row.push(word),
            _ => rows.push(vec![word]),
        }
    }
    let mut out = Vec::with_capacity(boxes.len());
    for mut row in rows {
        row.sort_by_key(|word| word.0);
        if rtl {
            row.reverse();
        }
        out.extend(row);
    }
    out
}

/// The mask that keeps one word of a title and nothing else: a rectangle
/// over the word, widened to halfway to its neighbours on the line so no
/// letter is clipped and no other word shows. `boxes` are in any order;
/// `word` indexes them.
pub fn word_mask(boxes: &[WordBox], word: usize, canvas: (u32, u32)) -> ClipMask {
    let (x, y, w, h) = boxes[word];
    let (x0, x1, h) = (f64::from(x), f64::from(x) + f64::from(w), f64::from(h));
    let mates = |other: &&WordBox| (other.1 - y).unsigned_abs() * 2 < boxes[word].3.max(1);
    let left = boxes
        .iter()
        .filter(mates)
        .map(|b| f64::from(b.0) + f64::from(b.2))
        .filter(|right| *right <= x0)
        .fold(f64::NEG_INFINITY, f64::max);
    let right = boxes
        .iter()
        .filter(mates)
        .map(|b| f64::from(b.0))
        .filter(|left| *left >= x1)
        .fold(f64::INFINITY, f64::min);
    let from = if left.is_finite() {
        (x0 + left) / 2.0
    } else {
        x0 - 0.3 * h
    };
    let to = if right.is_finite() {
        (x1 + right) / 2.0
    } else {
        x1 + 0.3 * h
    };
    let (top, bottom) = (f64::from(y) - 0.15 * h, f64::from(y) + h * 1.15);
    let (cw, ch) = (f64::from(canvas.0.max(1)), f64::from(canvas.1.max(1)));
    let short = cw.min(ch);
    let mut mask = ClipMask::of(MaskShape::Rectangle);
    mask.x = (from + to) / 2.0 / cw - 0.5;
    mask.y = (top + bottom) / 2.0 / ch - 0.5;
    mask.width = (to - from) / short;
    mask.height = (bottom - top) / short;
    mask.rotation = 0.0;
    mask.roundness = 0.0;
    mask.feather = 0.0;
    mask.invert = false;
    mask
}

/// When each word of `text` is lit, as `(start, duration)` on the
/// timeline: the caption's span shared out by each word's length, the way
/// a voice spends time on a long word. One per word, back to back.
pub fn word_slots(start: f64, duration: f64, text: &str) -> Vec<(f64, f64)> {
    let weights: Vec<f64> = text
        .split_whitespace()
        .map(|word| word.chars().count() as f64 + 1.0)
        .collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let mut at = start;
    weights
        .iter()
        .map(|weight| {
            let span = duration * weight / total;
            let slot = (at, span);
            at += span;
            slot
        })
        .collect()
}

/// The most pieces one caption is cut into, so a long caption cannot bury
/// the timeline in clips.
pub const MAX_PIECES: usize = 60;

/// A timed word: its text and when it is said, in seconds on the timeline.
pub type TimedWord = (String, f64, f64);

/// The pieces for one caption `start..end` whose `text` is `words` as timed.
/// Word timings are needed for [`Reveal::Build`] and [`Reveal::WordByWord`];
/// without them those fall back to [`Reveal::Whole`], never to invented
/// times. Pieces tile the caption's time exactly, never overlapping.
pub fn plan_reveal(
    start: f64,
    end: f64,
    text: &str,
    words: &[TimedWord],
    reveal: Reveal,
) -> Vec<Piece> {
    let end = end.max(start + MIN_CLIP_DURATION);
    let whole = || {
        vec![Piece {
            start,
            duration: end - start,
            content: text.to_owned(),
        }]
    };
    match reveal {
        Reveal::Whole => whole(),
        Reveal::Build | Reveal::WordByWord => {
            if words.is_empty() {
                return whole();
            }
            let words = thin(words);
            let mut out = Vec::with_capacity(words.len());
            for (at, (_, word_start, _)) in words.iter().enumerate() {
                let from = if at == 0 {
                    start
                } else {
                    word_start.max(start)
                };
                let to = words.get(at + 1).map_or(end, |next| next.1.min(end));
                if to - from < MIN_CLIP_DURATION {
                    continue;
                }
                let content = if reveal == Reveal::Build {
                    words[..=at]
                        .iter()
                        .map(|w| w.0.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    words[at].0.clone()
                };
                out.push(Piece {
                    start: from,
                    duration: to - from,
                    content,
                });
            }
            if out.is_empty() { whole() } else { out }
        }
        Reveal::Typewriter { per_second } => {
            let letters: Vec<char> = text.chars().collect();
            if letters.is_empty() {
                return whole();
            }
            let span = end - start;
            let wanted = (span * per_second.max(1.0)).round() as usize;
            let steps = wanted.clamp(1, MAX_PIECES).min(letters.len());
            // The last piece holds the full line for the last fifth of the
            // time, so the finished caption can be read.
            let typing = span * 0.8;
            let step = typing / steps as f64;
            let mut out = Vec::with_capacity(steps);
            for i in 0..steps {
                let shown = ((i + 1) * letters.len()).div_ceil(steps);
                let content: String = letters[..shown].iter().collect();
                let from = start + step * i as f64;
                let to = if i + 1 == steps { end } else { from + step };
                out.push(Piece {
                    start: from,
                    duration: to - from,
                    content: content.trim_end().to_owned(),
                });
            }
            out
        }
    }
}

/// At most [`MAX_PIECES`] words, evenly thinned if there are more: the
/// reveal then shows several words at a step rather than dropping the tail.
fn thin(words: &[TimedWord]) -> Vec<TimedWord> {
    if words.len() <= MAX_PIECES {
        return words.to_vec();
    }
    let per = words.len().div_ceil(MAX_PIECES);
    words
        .chunks(per)
        .map(|chunk| {
            (
                chunk
                    .iter()
                    .map(|w| w.0.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                chunk[0].1,
                chunk[chunk.len() - 1].2,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use crate::model::TextStyle;

    fn editor_with_clip(duration: f64) -> (Editor, String) {
        let mut editor = Editor::new();
        let id = editor
            .apply(Command::AddTextClip {
                track_id: None,
                above: true,
                start: 0.0,
                style: Some(TextStyle {
                    content: "سلام".into(),
                    ..TextStyle::default()
                }),
                duration: Some(duration),
                offset_y: Some(0.3),
            })
            .expect("adds")
            .created_id
            .expect("id");
        (editor, id)
    }

    fn apply(editor: &mut Editor, id: &str, effect: Effect) {
        let clip = editor.project().active().clip(id).expect("clip").clone();
        for command in commands(&clip, &effect) {
            editor.apply(command).expect("applies");
        }
    }

    fn keys(editor: &Editor, id: &str, property: KeyProperty) -> Vec<(f64, f64)> {
        let mut found: Vec<(f64, f64)> = editor
            .project()
            .active()
            .clip(id)
            .expect("clip")
            .keys
            .iter()
            .filter(|key| key.property == property)
            .map(|key| (key.at, key.value))
            .collect();
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found
    }

    #[test]
    fn fade_in_and_out_key_the_opacity_and_end_at_zero() {
        let (mut editor, id) = editor_with_clip(4.0);
        apply(
            &mut editor,
            &id,
            Effect::Fade {
                fade_in: 0.4,
                fade_out: 0.8,
            },
        );
        let opacity = keys(&editor, &id, KeyProperty::Opacity);
        assert_eq!(opacity.len(), 4);
        assert_eq!(opacity[0], (0.0, 0.0));
        assert!((opacity[1].0 - 0.1).abs() < 1e-9 && opacity[1].1 == 1.0);
        assert!((opacity[2].0 - 0.8).abs() < 1e-9 && opacity[2].1 == 1.0);
        assert_eq!(opacity[3], (1.0, 0.0));
    }

    #[test]
    fn pop_overshoots_then_settles_at_the_clips_own_size() {
        let (mut editor, id) = editor_with_clip(2.0);
        apply(
            &mut editor,
            &id,
            Effect::Pop {
                seconds: 0.4,
                overshoot: 0.1,
            },
        );
        let scale = keys(&editor, &id, KeyProperty::Scale);
        assert_eq!(scale.len(), 3);
        assert!((scale[0].1 - 0.6).abs() < 1e-9);
        assert!((scale[1].1 - 1.1).abs() < 1e-9, "overshoot {scale:?}");
        assert_eq!(scale[2].1, 1.0);
        assert!(scale[2].0 > scale[1].0 && scale[1].0 > scale[0].0);
    }

    #[test]
    fn slide_from_the_right_ends_on_the_clips_own_offset() {
        let (mut editor, id) = editor_with_clip(2.0);
        apply(
            &mut editor,
            &id,
            Effect::Slide {
                seconds: 0.5,
                from: Direction::Right,
                distance: 0.2,
            },
        );
        let x = keys(&editor, &id, KeyProperty::OffsetX);
        assert_eq!(x.len(), 2);
        assert!((x[0].1 - 0.2).abs() < 1e-9);
        assert_eq!(x[1].1, 0.0);
        // Down is on the vertical, from below the clip's own place.
        apply(
            &mut editor,
            &id,
            Effect::Slide {
                seconds: 0.5,
                from: Direction::Down,
                distance: 0.1,
            },
        );
        assert!(keys(&editor, &id, KeyProperty::OffsetX).is_empty());
        let y = keys(&editor, &id, KeyProperty::OffsetY);
        assert!(
            (y[0].1 - 0.4).abs() < 1e-9 && (y[1].1 - 0.3).abs() < 1e-9,
            "{y:?}"
        );
    }

    #[test]
    fn an_effect_replaces_the_last_and_none_takes_it_off() {
        let (mut editor, id) = editor_with_clip(2.0);
        apply(
            &mut editor,
            &id,
            Effect::Scale {
                seconds: 0.3,
                from: 0.5,
            },
        );
        assert!(!keys(&editor, &id, KeyProperty::Scale).is_empty());
        apply(
            &mut editor,
            &id,
            Effect::Fade {
                fade_in: 0.3,
                fade_out: 0.0,
            },
        );
        assert!(keys(&editor, &id, KeyProperty::Scale).is_empty());
        assert_eq!(keys(&editor, &id, KeyProperty::Opacity).len(), 2);
        apply(&mut editor, &id, Effect::None);
        assert!(editor.project().active().clip(&id).unwrap().keys.is_empty());
    }

    #[test]
    fn an_entrance_never_takes_more_than_its_share_of_a_short_clip() {
        let (mut editor, id) = editor_with_clip(0.5);
        apply(
            &mut editor,
            &id,
            Effect::Fade {
                fade_in: 5.0,
                fade_out: 0.0,
            },
        );
        let opacity = keys(&editor, &id, KeyProperty::Opacity);
        assert!(opacity[1].0 <= MAX_SHARE + 1e-9);
    }

    #[test]
    fn an_effect_leaves_the_words_and_place_alone() {
        let (mut editor, id) = editor_with_clip(2.0);
        apply(
            &mut editor,
            &id,
            Effect::Fade {
                fade_in: 0.3,
                fade_out: 0.3,
            },
        );
        let clip = editor.project().active().clip(&id).unwrap().clone();
        assert_eq!(clip.text.as_ref().unwrap().content, "سلام");
        assert_eq!(clip.offset_y, 0.3);
    }

    fn w(text: &str, start: f64, end: f64) -> TimedWord {
        (text.to_owned(), start, end)
    }

    fn tiles(pieces: &[Piece], start: f64, end: f64) {
        assert!(
            (pieces[0].start - start).abs() < 1e-9,
            "starts at the caption"
        );
        for pair in pieces.windows(2) {
            assert!(
                (pair[0].start + pair[0].duration - pair[1].start).abs() < 1e-9,
                "no gap or overlap: {pair:?}"
            );
        }
        let last = pieces.last().unwrap();
        assert!(
            (last.start + last.duration - end).abs() < 1e-9,
            "ends with it"
        );
    }

    #[test]
    fn building_a_line_adds_a_word_at_a_time_in_reading_order() {
        let words = [w("امروز", 1.0, 1.4), w("هوا", 1.5, 1.8), w("خوب", 1.9, 2.3)];
        let pieces = plan_reveal(1.0, 3.0, "امروز هوا خوب", &words, Reveal::Build);
        let shown: Vec<&str> = pieces.iter().map(|p| p.content.as_str()).collect();
        assert_eq!(shown, ["امروز", "امروز هوا", "امروز هوا خوب"]);
        tiles(&pieces, 1.0, 3.0);
    }

    #[test]
    fn word_by_word_replaces_each_word() {
        let words = [w("a", 0.0, 0.3), w("b", 0.5, 0.8), w("c", 1.0, 1.3)];
        let pieces = plan_reveal(0.0, 2.0, "a b c", &words, Reveal::WordByWord);
        let shown: Vec<&str> = pieces.iter().map(|p| p.content.as_str()).collect();
        assert_eq!(shown, ["a", "b", "c"]);
        tiles(&pieces, 0.0, 2.0);
    }

    #[test]
    fn without_word_times_a_word_reveal_is_the_whole_caption_not_a_guess() {
        let pieces = plan_reveal(0.0, 2.0, "a b c", &[], Reveal::WordByWord);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].content, "a b c");
        let pieces = plan_reveal(0.0, 2.0, "a b c", &[], Reveal::Whole);
        assert_eq!(pieces.len(), 1);
    }

    #[test]
    fn typing_reveals_letters_and_ends_on_the_full_line() {
        let text = "سلام دنیا";
        let pieces = plan_reveal(0.0, 3.0, text, &[], Reveal::Typewriter { per_second: 6.0 });
        assert!(pieces.len() > 3 && pieces.len() <= MAX_PIECES);
        assert_eq!(pieces.last().unwrap().content, text);
        for pair in pieces.windows(2) {
            assert!(
                pair[1].content.chars().count() >= pair[0].content.chars().count(),
                "never shrinks"
            );
            assert!(pair[1].content.starts_with(&pair[0].content));
        }
        tiles(&pieces, 0.0, 3.0);
    }

    #[test]
    fn spoken_order_follows_the_boxes_not_the_layouts_list() {
        let lines: Vec<WordBox> = vec![
            (10, 10, 30, 20),
            (50, 10, 30, 20),
            (90, 10, 30, 20),
            (10, 50, 30, 20),
            (50, 50, 30, 20),
            (90, 50, 30, 20),
        ];
        // Left to right: the same whichever way the layout listed them.
        let mut shuffled = lines.clone();
        shuffled.reverse();
        assert_eq!(spoken_order(&lines, false), lines);
        assert_eq!(spoken_order(&shuffled, false), lines);
        // Right to left: each line from its right edge, lines still top down.
        for given in [&lines, &shuffled] {
            let rtl = spoken_order(given, true);
            assert_eq!(rtl[0], (90, 10, 30, 20));
            assert_eq!(rtl[2], (10, 10, 30, 20));
            assert_eq!(rtl[3], (90, 50, 30, 20));
            assert_eq!(rtl[5], (10, 50, 30, 20));
        }
    }

    #[test]
    fn the_first_letter_decides_the_direction() {
        assert!(is_rtl("سلام دنیا"));
        assert!(is_rtl("123 سلام"));
        assert!(is_rtl("«سلام» hello"));
        assert!(is_rtl("שלום"));
        assert!(!is_rtl("Hello سلام"));
        assert!(!is_rtl("Hello world"));
        assert!(!is_rtl("123 ..."));
    }

    #[test]
    fn a_word_mask_covers_its_word_and_stops_halfway_to_the_next() {
        let boxes: Vec<WordBox> = vec![(100, 100, 100, 40), (240, 100, 100, 40)];
        let mask = word_mask(&boxes, 0, (1000, 500));
        // Left edge 100 - 12, right edge halfway to 240: 220.
        let left = (mask.x + 0.5) * 1000.0 - mask.width * 500.0 / 2.0;
        let right = (mask.x + 0.5) * 1000.0 + mask.width * 500.0 / 2.0;
        assert!((left - 88.0).abs() < 0.01, "left {left}");
        assert!((right - 220.0).abs() < 0.01, "right {right}");
        assert_eq!(mask.shape, MaskShape::Rectangle);
        assert_eq!(mask.feather, 0.0);
        // The second word starts where the first one stops.
        let next = word_mask(&boxes, 1, (1000, 500));
        let next_left = (next.x + 0.5) * 1000.0 - next.width * 500.0 / 2.0;
        assert!((next_left - 220.0).abs() < 0.01);
    }

    #[test]
    fn word_slots_share_the_caption_and_leave_no_gap() {
        let slots = word_slots(2.0, 3.0, "سلام به همه دوستان");
        assert_eq!(slots.len(), 4);
        assert!((slots[0].0 - 2.0).abs() < 1e-9);
        let end = slots.last().map(|slot| slot.0 + slot.1).unwrap();
        assert!((end - 5.0).abs() < 1e-9);
        for pair in slots.windows(2) {
            assert!((pair[0].0 + pair[0].1 - pair[1].0).abs() < 1e-9);
        }
        assert!(slots[3].1 > slots[1].1, "a longer word is lit longer");
        assert!(word_slots(0.0, 1.0, "   ").is_empty());
    }

    #[test]
    fn named_motions_end_on_the_clips_own_size_and_place() {
        for name in [
            "pop",
            "scale",
            "bounce",
            "elastic",
            "glitch",
            "slide-left",
            "fade",
        ] {
            let (mut editor, id) = editor_with_clip(3.0);
            let effect = Effect::named(name, 0.4, 1.0);
            assert_ne!(effect, Effect::None, "{name}");
            apply(&mut editor, &id, effect);
            let clip = editor.project().active().clip(&id).expect("clip").clone();
            assert!(!clip.keys.is_empty(), "{name} sets keys");
            if name != "fade" {
                let scale = keys(&editor, &id, KeyProperty::Scale);
                if let Some(last) = scale.last() {
                    assert!((last.1 - clip.scale).abs() < 1e-9, "{name} settles");
                }
            }
            let ox = keys(&editor, &id, KeyProperty::OffsetX);
            if let Some(last) = ox.last() {
                assert!((last.1 - clip.offset_x).abs() < 1e-9, "{name} lands");
            }
        }
        assert_eq!(Effect::named("nonsense", 1.0, 1.0), Effect::None);
        assert_eq!(Effect::named("none", 1.0, 1.0), Effect::None);
    }

    #[test]
    fn a_long_caption_is_capped_not_buried() {
        let long: String = "ب".repeat(500);
        let pieces = plan_reveal(
            0.0,
            100.0,
            &long,
            &[],
            Reveal::Typewriter { per_second: 30.0 },
        );
        assert!(pieces.len() <= MAX_PIECES);
        let many: Vec<TimedWord> = (0..200)
            .map(|i| w("x", i as f64 * 0.1, i as f64 * 0.1 + 0.05))
            .collect();
        let pieces = plan_reveal(0.0, 20.0, "x", &many, Reveal::WordByWord);
        assert!(pieces.len() <= MAX_PIECES);
        tiles(&pieces, 0.0, 20.0);
    }
}
