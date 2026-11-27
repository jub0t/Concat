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

use concat_doc::commands::{Command, Cue};
use concat_doc::model::{CaptionLook, Title};
use concat_doc::{Tick, TickLen};

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
pub fn heard_cues(heard: &[Heard], clip_start: f64, speed: f64) -> Vec<Cue> {
    let speed = if speed > 0.0 { speed } else { 1.0 };
    heard
        .iter()
        .filter(|stretch| !stretch.text.trim().is_empty())
        .filter_map(|stretch| {
            let start = Tick::from_display_seconds((clip_start + stretch.start / speed).max(0.0))?;
            let seconds = ((stretch.end - stretch.start) / speed).max(SHORTEST);
            let len = TickLen::try_from(Tick::from_display_seconds(seconds)?.get()).ok()?;
            Some(Cue {
                start,
                len,
                words: stretch.text.trim().to_owned(),
            })
        })
        .collect()
}

/// A caption lane's look: the kept look where there is one (family,
/// weight, colours, stroke, shadow, background) at the sheet's size and
/// place, the bundled face otherwise. `look` is (offset from the middle,
/// size).
pub fn caption_look(look: (f64, f64), base: Option<&Title>) -> CaptionLook {
    let (offset_y, font_size) = look;
    let title = match base {
        Some(base) => Title {
            content: String::new(),
            font_size,
            max_width: 0.0,
            max_height: 0.0,
            // Runs are some words' looks, and these are other words.
            runs: Vec::new(),
            ..base.clone()
        },
        None => Title {
            content: String::new(),
            font_family: "Hanken Grotesk".to_owned(),
            font_size,
            font_weight: 600.0,
            ..Title::default()
        },
    };
    CaptionLook { title, offset_y }
}

/// `cues` laid on a new caption lane in `look` over `base`, as one edit.
/// None when no cue has words.
pub fn place_captions(cues: Vec<Cue>, look: (f64, f64), base: Option<&Title>) -> Option<Command> {
    let cues: Vec<Cue> = cues
        .into_iter()
        .filter(|cue| !cue.words.trim().is_empty())
        .collect();
    (!cues.is_empty()).then(|| Command::PlaceCaptions {
        lane: None,
        look: caption_look(look, base),
        cues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_doc::Doc;
    use concat_doc::model::{Color, Content, LaneId};

    fn heard(start: f64, end: f64, text: &str) -> Heard {
        Heard {
            start,
            end,
            text: text.to_owned(),
        }
    }

    fn secs(len: TickLen) -> f64 {
        concat_doc::TickDur::from(len).display_seconds()
    }

    #[test]
    fn a_stretch_lands_where_it_was_said_at_the_clips_speed() {
        let cues = heard_cues(
            &[heard(2.0, 4.0, " hello "), heard(5.0, 5.05, "hi")],
            10.0,
            2.0,
        );
        assert_eq!(cues[0].start.display_seconds(), 11.0);
        assert_eq!(secs(cues[0].len), 1.0);
        assert_eq!(cues[0].words, "hello");
        assert_eq!(cues[1].start.display_seconds(), 12.5);
        assert_eq!(secs(cues[1].len), SHORTEST, "long enough to read");
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
        let mut doc = Doc::new();
        let lanes = doc.model().lanes.len();
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
        let lane: LaneId = doc
            .apply(command)
            .created
            .expect("the lane")
            .typed()
            .expect("a lane");
        assert_eq!(doc.model().lanes.len(), lanes + 1, "one new lane");
        let on = |doc: &Doc| -> Vec<(String, Color, f64)> {
            doc.model()
                .items
                .index()
                .on_lane(lane)
                .filter_map(|(id, _, _)| doc.model().items.get(id).cloned())
                .map(|item| match &item.content {
                    Content::Title(title) => (
                        title.content.clone(),
                        title.color,
                        item.picture.offset_y.value,
                    ),
                    _ => panic!("a title"),
                })
                .collect()
        };
        let captions = on(&doc);
        assert_eq!(captions.len(), 3);
        assert!(captions.iter().all(|c| c.2 == CAPTION_OFFSETS[0]));
        let mut look = doc
            .model()
            .lanes
            .get(lane)
            .and_then(|lane| lane.captions.clone())
            .expect("a caption lane");
        let gold = Color::rgb(0xff, 0xd6, 0x0a);
        look.title.color = gold;
        look.offset_y = -0.3;
        doc.apply(Command::SetCaptionLook {
            lane,
            look: Some(look),
        });
        assert_eq!(
            on(&doc),
            vec![
                ("one".to_owned(), gold, -0.3),
                ("two".to_owned(), gold, -0.3),
                ("three".to_owned(), gold, -0.3),
            ]
        );
    }
}
