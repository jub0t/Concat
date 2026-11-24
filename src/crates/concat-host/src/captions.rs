// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What was said in a clip, as captions: the one way words heard become
//! captions, for every caller - the desktop's Captions sheet, the phone app
//! and an agent through the API - so captions look and time the same
//! whoever asked for them.
//!
//! Captions go on a caption lane, laid by one `PlaceCaptions`: one edit
//! however many there are, and one look the lane keeps, so restyling them
//! all is one `SetCaptionLook` rather than an edit to every caption.

use concat_project::Command;
use concat_project::model::{CaptionCue, CaptionLook, TextStyle};

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

/// The cues for what was heard in a clip that starts at `clip_start` on
/// the timeline and plays at `speed`: one a stretch, where it was said and
/// never shorter than a word can be read. Stretches with no words are left
/// out.
pub fn heard_cues(heard: &[Heard], clip_start: f64, speed: f64) -> Vec<CaptionCue> {
    let speed = if speed > 0.0 { speed } else { 1.0 };
    heard
        .iter()
        .filter(|stretch| !stretch.text.trim().is_empty())
        .map(|stretch| CaptionCue {
            start: clip_start + stretch.start / speed,
            duration: ((stretch.end - stretch.start) / speed).max(SHORTEST),
            text: stretch.text.trim().to_owned(),
        })
        .collect()
}

/// A caption lane's look: the kept look where there is one (family,
/// weight, colours, stroke, shadow, background) at the sheet's size and
/// place, the bundled face otherwise. `look` is (offset from the middle,
/// size).
pub fn caption_look(look: (f64, f64), base: Option<&TextStyle>) -> CaptionLook {
    let (offset_y, font_size) = look;
    let style = match base {
        Some(base) => TextStyle {
            content: String::new(),
            font_size,
            max_width: 0.0,
            max_height: 0.0,
            // Runs are some words' looks, and these are other words.
            runs: Vec::new(),
            ..base.clone()
        },
        None => TextStyle {
            content: String::new(),
            font_family: "Hanken Grotesk".to_owned(),
            font_size,
            font_weight: 600.0,
            ..TextStyle::default()
        },
    };
    CaptionLook { style, offset_y }
}

/// `cues` laid on a new caption lane in `look` over `base`, as one edit.
/// None when no cue has words.
pub fn place_captions(
    cues: Vec<CaptionCue>,
    look: (f64, f64),
    base: Option<&TextStyle>,
) -> Option<Command> {
    let cues: Vec<CaptionCue> = cues
        .into_iter()
        .filter(|cue| !cue.text.trim().is_empty())
        .collect();
    (!cues.is_empty()).then(|| Command::PlaceCaptions {
        track_id: None,
        look: caption_look(look, base),
        cues,
    })
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
        let cues = heard_cues(
            &[heard(2.0, 4.0, " hello "), heard(5.0, 5.05, "hi")],
            10.0,
            2.0,
        );
        assert_eq!(
            (cues[0].start, cues[0].duration, cues[0].text.as_str()),
            (11.0, 1.0, "hello")
        );
        assert_eq!(cues[1].start, 12.5);
        assert_eq!(
            cues[1].duration, SHORTEST,
            "a short word stays up long enough to read"
        );
    }

    #[test]
    fn a_stretch_with_no_words_is_left_out() {
        assert!(heard_cues(&[heard(0.0, 1.0, "   ")], 0.0, 1.0).is_empty());
        assert!(place_captions(Vec::new(), (0.0, 0.05), None).is_none());
    }

    /// Placed captions are one edit on one new caption lane, each a title
    /// in the lane's look with its own words; one change to the look
    /// restyles them all and keeps their words.
    #[test]
    fn captions_are_one_lane_one_edit_and_one_look() {
        let mut editor = concat_project::Editor::new();
        let lanes = editor.project().active().tracks.len();
        let cues = heard_cues(
            &[
                heard(0.0, 1.5, "one"),
                heard(1.0, 3.0, "two"),
                heard(4.0, 5.0, "three"),
            ],
            0.0,
            1.0,
        );
        let command = place_captions(cues, (CAPTION_OFFSETS[0], CAPTION_SIZES[1]), None)
            .expect("words to place");
        let lane = editor
            .apply(command)
            .expect("placed")
            .created_id
            .expect("the lane");
        let timeline = editor.project().active();
        assert_eq!(timeline.tracks.len(), lanes + 1, "one new lane");
        let captions: Vec<_> = timeline
            .clips
            .iter()
            .filter(|clip| clip.track_id == lane)
            .collect();
        assert_eq!(captions.len(), 3);
        assert_eq!(
            captions[0].duration, 1.0,
            "the first ends where the second begins"
        );
        assert!(
            captions
                .iter()
                .all(|clip| clip.offset_y == CAPTION_OFFSETS[0])
        );

        let mut look = timeline
            .track(&lane)
            .and_then(|track| track.captions.clone())
            .expect("a caption lane");
        look.style.color = "#ffd60a".to_owned();
        look.offset_y = -0.3;
        editor
            .apply(Command::SetCaptionLook {
                track_id: lane.clone(),
                look: Some(look),
            })
            .expect("restyled");
        let timeline = editor.project().active();
        let words: Vec<(String, String, f64)> = timeline
            .clips
            .iter()
            .filter(|clip| clip.track_id == lane)
            .map(|clip| {
                let text = clip.text.as_ref().expect("a title");
                (text.content.clone(), text.color.clone(), clip.offset_y)
            })
            .collect();
        assert_eq!(
            words,
            vec![
                ("one".to_owned(), "#ffd60a".to_owned(), -0.3),
                ("two".to_owned(), "#ffd60a".to_owned(), -0.3),
                ("three".to_owned(), "#ffd60a".to_owned(), -0.3),
            ]
        );
        assert!(editor.undo(), "one undo step");
        assert!(
            editor
                .project()
                .active()
                .clips
                .iter()
                .filter(|clip| clip.track_id == lane)
                .all(|clip| clip.offset_y == CAPTION_OFFSETS[0])
        );
    }
}
