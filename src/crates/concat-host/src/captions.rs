// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What was said in a clip, as captions: the one way words heard become
//! title clips, for every caller - the desktop's Captions sheet, the phone
//! app and an agent through the API - so captions look and time the same
//! whoever asked for them.

use concat_project::Command;
use concat_project::model::TextStyle;

/// Where a caption sits, by the sheet's row: a frame-height fraction from
/// the centre, positive down. Bottom, centre, top.
pub const CAPTION_OFFSETS: [f64; 3] = [0.35, 0.0, -0.35];

/// A caption's cap height by the sheet's row, as a fraction of the frame:
/// small, medium, large.
pub const CAPTION_SIZES: [f64; 3] = [0.04, 0.05, 0.065];

/// The shortest a caption stays up, in seconds: a word on its own is not a
/// flicker.
const SHORTEST: f64 = 0.2;

/// One stretch of speech: when it starts and ends in the clip's own
/// source, in seconds, and its words.
#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    /// Seconds into the clip's sound.
    pub start: f64,
    /// Seconds into the clip's sound.
    pub end: f64,
    /// What was said.
    pub text: String,
}

/// The captions for what was heard in a clip that starts at `clip_start` on
/// the timeline and plays at `speed`: one title clip a stretch, in `look`
/// (offset from the middle, size), over `base` where there is one. Stretches
/// with no words are left out.
pub fn caption_commands(
    heard: &[Heard],
    clip_start: f64,
    speed: f64,
    look: (f64, f64),
    base: Option<&TextStyle>,
) -> Vec<Command> {
    let speed = if speed > 0.0 { speed } else { 1.0 };
    heard
        .iter()
        .filter_map(|stretch| {
            let text = stretch.text.trim();
            (!text.is_empty()).then(|| {
                caption_clip(
                    text.to_owned(),
                    clip_start + stretch.start / speed,
                    ((stretch.end - stretch.start) / speed).max(SHORTEST),
                    look,
                    base,
                )
            })
        })
        .collect()
}

/// One caption as a title clip: its words, when and for how long, and its
/// look - the kept look where there is one (family, weight, colours,
/// stroke, shadow, background) at the caption's own size and place, the
/// bundled face otherwise.
pub fn caption_clip(
    text: String,
    start: f64,
    duration: f64,
    look: (f64, f64),
    base: Option<&TextStyle>,
) -> Command {
    let (offset_y, font_size) = look;
    let style = match base {
        Some(base) => TextStyle {
            content: text,
            font_size,
            max_width: 0.0,
            max_height: 0.0,
            // Runs are some words' looks, and these are other words.
            runs: Vec::new(),
            ..base.clone()
        },
        None => TextStyle {
            content: text,
            font_family: "Hanken Grotesk".to_owned(),
            font_size,
            font_weight: 600.0,
            ..TextStyle::default()
        },
    };
    Command::AddTextClip {
        track_id: None,
        above: true,
        start,
        style: Some(style),
        duration: Some(duration),
        offset_y: Some(offset_y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(start: f64, end: f64, text: &str) -> Heard {
        Heard {
            start,
            end,
            text: text.to_owned(),
        }
    }

    #[test]
    fn a_stretch_lands_where_it_was_said_at_the_clips_speed() {
        let commands = caption_commands(
            &[heard(2.0, 4.0, " hello "), heard(5.0, 5.05, "hi")],
            10.0,
            2.0,
            (CAPTION_OFFSETS[2], CAPTION_SIZES[1]),
            None,
        );
        let placed: Vec<(f64, f64, String)> = commands
            .iter()
            .map(|command| match command {
                Command::AddTextClip {
                    start,
                    duration,
                    style,
                    ..
                } => (
                    *start,
                    duration.expect("a duration"),
                    style.as_ref().expect("a style").content.clone(),
                ),
                other => panic!("not a title: {other:?}"),
            })
            .collect();
        assert_eq!(placed[0], (11.0, 1.0, "hello".to_owned()));
        assert_eq!(placed[1].0, 12.5);
        assert_eq!(
            placed[1].1, SHORTEST,
            "a short word stays up long enough to read"
        );
    }

    #[test]
    fn a_stretch_with_no_words_is_left_out() {
        let commands = caption_commands(&[heard(0.0, 1.0, "   ")], 0.0, 1.0, (0.0, 0.05), None);
        assert!(commands.is_empty());
    }
}
