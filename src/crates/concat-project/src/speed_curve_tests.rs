// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A clip's speed curve through the editor: setting and removing one, what
//! it does to the clip's length and the lane behind it, what trims, splits,
//! freezes, merges and copies do to it, the presets it can be given, and
//! how it is written to and read back from a project file.

use serde_json::{Value, json};

use crate::commands::{NewMedia, TrimEdge};
use crate::doc::DocumentSettings;
use crate::model::{Clip, MediaKind, SpeedPoint};
use crate::speed::{PRESETS, curve_of, is_smooth, mean_of, preset, preset_of};
use crate::{Command, CommandError, Editor};

const TIGHT: f64 = 1e-9;

fn close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < TIGHT,
        "{what}: got {actual}, expected {expected}"
    );
}

fn footage(path: &str, seconds: f64) -> NewMedia {
    NewMedia {
        path: path.to_owned(),
        name: path.trim_start_matches('/').to_owned(),
        duration: Some(seconds),
        kind: MediaKind::Video,
        width: Some(1280),
        height: Some(720),
        frame_rate: Some(25.0),
        frame_rate_fraction: Some("25/1".to_owned()),
        video_codec: Some("h264".to_owned()),
        audio_codec: Some("aac".to_owned()),
        has_audio: true,
        audio_tracks: Vec::new(),
        origin: None,
        color_space: Default::default(),
    }
}

/// An editor holding one ten-second file, and a clip of all of it at 0 on
/// the first lane at normal speed.
fn one_clip() -> (Editor, String, String) {
    let mut editor = Editor::new();
    let media = editor
        .apply(Command::AddMedia {
            item: footage("/ramp.mp4", 10.0),
        })
        .unwrap()
        .created_id
        .unwrap();
    let clip = add_at(&mut editor, &media, 0.0);
    (editor, media, clip)
}

fn add_at(editor: &mut Editor, media: &str, start: f64) -> String {
    let track_id = editor.project().active().tracks[0].id.clone();
    editor
        .apply(Command::AddClip {
            media_id: media.to_owned(),
            track_id,
            start,
            ripple: false,
        })
        .unwrap()
        .created_id
        .unwrap()
}

fn straight(raw: &[(f64, f64)]) -> Vec<SpeedPoint> {
    raw.iter()
        .map(|&(at, speed)| SpeedPoint {
            at,
            speed,
            slope: None,
        })
        .collect()
}

fn eased(raw: &[(f64, f64)]) -> Vec<SpeedPoint> {
    raw.iter()
        .map(|&(at, speed)| SpeedPoint {
            at,
            speed,
            slope: Some(0.0),
        })
        .collect()
}

fn set_curve(
    editor: &mut Editor,
    clip: &str,
    curve: Option<Vec<SpeedPoint>>,
    ripple: bool,
) -> bool {
    editor
        .apply(Command::SetClipSpeedCurve {
            clip_id: clip.to_owned(),
            curve,
            ripple,
        })
        .unwrap()
        .applied
}

fn clip(editor: &Editor, id: &str) -> Clip {
    editor.project().active().clip(id).unwrap().clone()
}

/// Source seconds `t` timeline seconds into a clip.
fn source_at(clip: &Clip, t: f64) -> f64 {
    match clip.speed_curve.as_deref().and_then(curve_of) {
        Some(curve) => clip.source_start + clip.duration * curve.consumed(t / clip.duration),
        None => clip.source_start + t * clip.speed,
    }
}

/// Half speed rising in a straight line to double: mean 1.25.
const RAMP: [(f64, f64); 2] = [(0.0, 0.5), (1.0, 2.0)];

#[test]
fn a_curve_keeps_the_footage_and_sets_the_length_and_rate_to_its_mean() {
    let (mut editor, _, id) = one_clip();
    assert!(set_curve(&mut editor, &id, Some(straight(&RAMP)), false));
    let ramped = clip(&editor, &id);
    close(ramped.speed, 1.25, "the rate is the curve's mean");
    close(
        ramped.duration,
        8.0,
        "ten seconds of footage at a mean of 1.25",
    );
    close(ramped.source_start, 0.0, "the in-point stays");
    assert_eq!(ramped.speed_curve, Some(straight(&RAMP)));
    // The same curve again changes nothing and is no undo step.
    assert!(!set_curve(&mut editor, &id, Some(straight(&RAMP)), false));
    // An eased curve of the same points has the same mean.
    assert!(set_curve(&mut editor, &id, Some(eased(&RAMP)), false));
    close(clip(&editor, &id).duration, 8.0, "same footage, same mean");
}

#[test]
fn a_curve_on_a_retimed_clip_keeps_the_footage_the_rate_covered() {
    let (mut editor, _, id) = one_clip();
    editor
        .apply(Command::SetClipSpeed {
            clip_id: id.clone(),
            speed: 2.0,
            ripple: false,
        })
        .unwrap();
    close(clip(&editor, &id).duration, 5.0, "ten seconds at 2x");
    set_curve(
        &mut editor,
        &id,
        Some(straight(&[(0.0, 4.0), (1.0, 1.0)])),
        false,
    );
    let ramped = clip(&editor, &id);
    close(ramped.speed, 2.5, "mean");
    close(ramped.duration, 4.0, "ten seconds of footage at 2.5");
}

#[test]
fn taking_the_curve_away_leaves_a_constant_rate_at_its_mean() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    assert!(set_curve(&mut editor, &id, None, false));
    let flat = clip(&editor, &id);
    assert!(flat.speed_curve.is_none());
    close(flat.speed, 1.25, "the mean stays the rate");
    close(flat.duration, 8.0, "the length does not jump");
}

#[test]
fn an_empty_curve_is_no_curve() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    set_curve(&mut editor, &id, Some(Vec::new()), false);
    let flat = clip(&editor, &id);
    assert!(flat.speed_curve.is_none());
    close(flat.speed, 1.25, "rate kept");
}

#[test]
fn speeds_past_the_range_play_at_its_ends() {
    let (mut editor, _, id) = one_clip();
    set_curve(
        &mut editor,
        &id,
        Some(straight(&[(0.0, 40.0), (1.0, 40.0)])),
        false,
    );
    let fast = clip(&editor, &id);
    close(fast.speed, 16.0, "held to 16x");
    close(fast.duration, 10.0 / 16.0, "length at 16x");
    set_curve(
        &mut editor,
        &id,
        Some(straight(&[(0.0, 0.0), (1.0, 0.001)])),
        false,
    );
    close(clip(&editor, &id).speed, 0.0625, "held to 1/16x");
}

#[test]
fn a_number_that_is_not_finite_is_refused_and_changes_nothing() {
    let (mut editor, _, id) = one_clip();
    let before = editor.project().clone();
    for bad in [
        straight(&[(f64::NAN, 1.0), (1.0, 1.0)]),
        straight(&[(0.0, f64::INFINITY), (1.0, 1.0)]),
    ] {
        let refused = editor.apply(Command::SetClipSpeedCurve {
            clip_id: id.clone(),
            curve: Some(bad),
            ripple: false,
        });
        assert_eq!(refused, Err(CommandError::NotANumber));
        assert_eq!(
            refused.unwrap_err().to_string(),
            "A number in that edit is not finite."
        );
    }
    assert_eq!(editor.project(), &before);
}

#[test]
fn an_unknown_clip_is_a_quiet_no_op() {
    let (mut editor, _, _) = one_clip();
    assert!(!set_curve(
        &mut editor,
        "nope",
        Some(straight(&RAMP)),
        false
    ));
}

#[test]
fn setting_a_constant_rate_ends_the_curve() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    editor
        .apply(Command::SetClipSpeed {
            clip_id: id.clone(),
            speed: 0.5,
            ripple: false,
        })
        .unwrap();
    let slow = clip(&editor, &id);
    assert!(slow.speed_curve.is_none());
    close(slow.speed, 0.5, "rate");
    close(
        slow.duration,
        20.0,
        "the curve's ten seconds of footage at half speed",
    );
}

#[test]
fn one_undo_takes_a_curve_off_and_redo_puts_it_back() {
    let (mut editor, _, id) = one_clip();
    let before = editor.project().clone();
    set_curve(&mut editor, &id, Some(eased(&RAMP)), false);
    let after = editor.project().clone();
    assert!(editor.undo());
    assert_eq!(editor.project(), &before);
    assert!(editor.redo());
    assert_eq!(editor.project(), &after);
}

#[test]
fn a_clip_that_grows_over_the_next_pushes_the_lane_along() {
    let (mut editor, media, id) = one_clip();
    let next = add_at(&mut editor, &media, 10.0);
    let later = add_at(&mut editor, &media, 30.0);
    // Slow in the middle: mean 0.625, so 16 s.
    set_curve(
        &mut editor,
        &id,
        Some(straight(&[(0.0, 1.0), (0.5, 0.25), (1.0, 1.0)])),
        false,
    );
    close(clip(&editor, &id).duration, 16.0, "slowed");
    close(clip(&editor, &next).start, 16.0, "pushed by the growth");
    close(
        clip(&editor, &later).start,
        36.0,
        "the rest of the lane too",
    );
}

#[test]
fn a_clip_that_shrinks_leaves_its_gap_unless_the_timeline_is_magnetic() {
    for ripple in [false, true] {
        let (mut editor, media, id) = one_clip();
        let next = add_at(&mut editor, &media, 10.0);
        let later = add_at(&mut editor, &media, 30.0);
        // Mean 2.5: four seconds.
        set_curve(
            &mut editor,
            &id,
            Some(straight(&[(0.0, 4.0), (1.0, 1.0)])),
            ripple,
        );
        close(clip(&editor, &id).duration, 4.0, "sped up");
        let (next, later) = (clip(&editor, &next).start, clip(&editor, &later).start);
        if ripple {
            close(next, 4.0, "the next clip closes up");
            close(later, 24.0, "and the rest by as much");
        } else {
            close(next, 10.0, "the gap stays");
            close(later, 30.0, "the gap stays");
        }
    }
}

#[test]
fn a_split_gives_each_piece_its_share_of_a_straight_curve() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    let whole = clip(&editor, &id);
    let tail_id = editor
        .apply(Command::SplitClips {
            clip_ids: vec![id.clone()],
            time: 3.0,
        })
        .unwrap()
        .created_id
        .unwrap();
    let (head, tail) = (clip(&editor, &id), clip(&editor, &tail_id));
    assert_eq!(
        head.speed_curve,
        Some(straight(&[(0.0, 0.5), (1.0, 1.0625)])),
        "the head ends at the speed the cut was at"
    );
    assert_eq!(
        tail.speed_curve,
        Some(straight(&[(0.0, 1.0625), (1.0, 2.0)]))
    );
    close(head.duration, 3.0, "head length");
    close(tail.duration, 5.0, "tail length");
    close(head.speed, 0.78125, "head mean");
    close(tail.speed, 1.53125, "tail mean");
    close(
        tail.source_start,
        2.34375,
        "the tail starts where the curve had got to",
    );
    for t in [0.0, 0.7, 1.5, 3.0] {
        close(
            source_at(&head, t),
            source_at(&whole, t),
            "head plays the same",
        );
    }
    for t in [0.0, 1.1, 2.5, 5.0] {
        close(
            source_at(&tail, t),
            source_at(&whole, 3.0 + t),
            "tail plays the same",
        );
    }
}

#[test]
fn a_split_of_an_eased_curve_carries_the_bend_across_the_cut() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(eased(&RAMP)), false);
    let whole = clip(&editor, &id);
    let tail_id = editor
        .apply(Command::SplitClips {
            clip_ids: vec![id.clone()],
            time: 3.0,
        })
        .unwrap()
        .created_id
        .unwrap();
    let (head, tail) = (clip(&editor, &id), clip(&editor, &tail_id));
    let head_curve = head.speed_curve.clone().unwrap();
    let tail_curve = tail.speed_curve.clone().unwrap();
    assert!(is_smooth(&head_curve) && is_smooth(&tail_curve));
    // The cut's speed, and its slope re-expressed over each piece.
    close(head_curve[1].speed, 0.974_609_375, "speed at the cut");
    close(
        head_curve[1].slope.unwrap(),
        0.791_015_625,
        "head's slope at the cut",
    );
    close(
        tail_curve[0].slope.unwrap(),
        1.318_359_375,
        "tail's slope at the cut",
    );
    close(head_curve[0].slope.unwrap(), 0.0, "the eased start");
    close(tail_curve[1].slope.unwrap(), 0.0, "the eased end");
    close(head.speed, 0.671_386_718_75, "head mean");
    close(tail.speed, 1.597_167_968_75, "tail mean");
    close(tail.source_start, 2.014_160_156_25, "tail in-point");
    for t in [0.0, 0.4, 1.9, 3.0] {
        close(source_at(&head, t), source_at(&whole, t), "head");
    }
    for t in [0.0, 0.3, 2.2, 5.0] {
        close(source_at(&tail, t), source_at(&whole, 3.0 + t), "tail");
    }
}

#[test]
fn a_split_keeps_the_points_inside_each_piece_where_they_were() {
    let (mut editor, _, id) = one_clip();
    let points = straight(&[(0.0, 1.0), (0.2, 3.0), (0.6, 0.5), (1.0, 1.0)]);
    set_curve(&mut editor, &id, Some(points), false);
    let length = clip(&editor, &id).duration;
    let tail_id = editor
        .apply(Command::SplitClips {
            clip_ids: vec![id.clone()],
            time: length * 0.4,
        })
        .unwrap()
        .created_id
        .unwrap();
    let head = clip(&editor, &id).speed_curve.unwrap();
    let tail = clip(&editor, &tail_id).speed_curve.unwrap();
    // Head: 0..0.4 of the whole, so the point at 0.2 lands half way.
    assert_eq!(head.len(), 3);
    close(head[1].at, 0.5, "head's middle point");
    close(head[1].speed, 3.0, "keeps its speed");
    close(head[2].speed, 1.75, "the cut, on the way down");
    close(tail[0].speed, 1.75, "the tail starts there");
    // Tail: 0.4..1, so the point at 0.6 lands a third of the way.
    assert_eq!(tail.len(), 3);
    close(tail[1].at, 1.0 / 3.0, "tail's middle point");
    close(tail[1].speed, 0.5, "keeps its speed");
}

#[test]
fn trimming_cuts_the_curve_so_what_is_left_plays_as_before() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    let whole = clip(&editor, &id);
    editor
        .apply(Command::TrimClip {
            clip_id: id.clone(),
            edge: TrimEdge::Start,
            delta: 2.0,
            ripple: false,
        })
        .unwrap();
    let headless = clip(&editor, &id);
    close(headless.start, 2.0, "start");
    close(headless.duration, 6.0, "length");
    close(
        headless.source_start,
        1.375,
        "the in-point moves by what the curve consumed",
    );
    assert_eq!(
        headless.speed_curve,
        Some(straight(&[(0.0, 0.875), (1.0, 2.0)]))
    );
    close(headless.speed, 1.4375, "the rest's mean");
    editor
        .apply(Command::TrimClip {
            clip_id: id.clone(),
            edge: TrimEdge::End,
            delta: -1.0,
            ripple: false,
        })
        .unwrap();
    let trimmed = clip(&editor, &id);
    close(trimmed.duration, 5.0, "length");
    assert_eq!(
        trimmed.speed_curve,
        Some(straight(&[(0.0, 0.875), (1.0, 1.8125)]))
    );
    close(trimmed.speed, 1.34375, "mean");
    for t in [0.0, 1.0, 2.5, 5.0] {
        close(
            source_at(&trimmed, t),
            source_at(&whole, 2.0 + t),
            "same footage",
        );
    }
}

#[test]
fn a_tail_pulled_past_the_curve_holds_the_end_speed() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    editor
        .apply(Command::TrimClip {
            clip_id: id.clone(),
            edge: TrimEdge::End,
            delta: 2.0,
            ripple: false,
        })
        .unwrap();
    let longer = clip(&editor, &id);
    close(longer.duration, 10.0, "length");
    let curve = longer.speed_curve.clone().unwrap();
    close(curve.last().unwrap().speed, 2.0, "the end speed held");
    close(curve[0].speed, 0.5, "the start kept");
    // Eight seconds covered ten of footage, the two more cover four.
    close(source_at(&longer, 10.0), 14.0, "footage covered");
}

#[test]
fn a_head_pulled_out_reaches_back_at_the_start_speed_and_stops_at_the_file() {
    let (mut editor, media, _) = one_clip();
    // A clip from 3 s into its file, placed at 20 s, under the ramp.
    let id = add_at(&mut editor, &media, 20.0);
    editor
        .apply(Command::TrimClip {
            clip_id: id.clone(),
            edge: TrimEdge::Start,
            delta: 3.0,
            ripple: false,
        })
        .unwrap();
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    let before = clip(&editor, &id);
    close(before.source_start, 3.0, "in-point");
    // Back by 10 s asked; at the start's half speed only 6 s reach the file's start.
    editor
        .apply(Command::TrimClip {
            clip_id: id.clone(),
            edge: TrimEdge::Start,
            delta: -10.0,
            ripple: false,
        })
        .unwrap();
    let after = clip(&editor, &id);
    close(after.source_start, 0.0, "at the file's start");
    close(after.start, before.start - 6.0, "six seconds earlier");
    close(after.duration, before.duration + 6.0, "six seconds longer");
    close(
        after.speed_curve.as_ref().unwrap()[0].speed,
        0.5,
        "the start speed held",
    );
    for t in [0.0, 2.0, before.duration] {
        close(
            source_at(&after, 6.0 + t),
            source_at(&before, t),
            "the rest unchanged",
        );
    }
}

#[test]
fn a_freeze_on_a_curved_clip_holds_a_still_and_cuts_the_curve_like_a_split() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    let whole = clip(&editor, &id);
    let mut still = footage("/still.jpg", 0.0);
    still.duration = None;
    still.kind = MediaKind::Image;
    still.has_audio = false;
    still.audio_codec = None;
    let freeze = editor
        .apply(Command::FreezeFrame {
            clip_id: id.clone(),
            time: 4.0,
            duration: Some(1.0),
            still: Some(still),
        })
        .unwrap()
        .created_id
        .unwrap();
    let timeline = editor.project().active();
    let frozen = timeline.clip(&freeze).unwrap();
    assert!(frozen.speed_curve.is_none(), "a still has no curve");
    close(frozen.speed, 1.0, "nor a rate");
    let head = timeline.clip(&id).unwrap();
    let tail = timeline
        .clips
        .iter()
        .find(|c| c.id != id && c.id != freeze)
        .unwrap();
    assert!(head.speed_curve.is_some() && tail.speed_curve.is_some());
    close(head.duration, 4.0, "head");
    close(tail.start, 5.0, "tail after the hold");
    close(
        tail.source_start,
        source_at(&whole, 4.0),
        "tail picks up at the cut",
    );
}

#[test]
fn clips_under_a_curve_are_not_merged() {
    let (mut editor, _, id) = one_clip();
    let tail = editor
        .apply(Command::SplitClips {
            clip_ids: vec![id.clone()],
            time: 5.0,
        })
        .unwrap()
        .created_id
        .unwrap();
    set_curve(
        &mut editor,
        &tail,
        Some(straight(&[(0.0, 1.0), (1.0, 1.0)])),
        false,
    );
    let refused = editor.apply(Command::MergeClips {
        clip_ids: vec![id, tail],
    });
    assert_eq!(
        refused.unwrap_err().to_string(),
        "A clip with a speed curve cannot be merged."
    );
}

#[test]
fn matching_a_clip_to_another_keeps_its_own_speed_and_curve() {
    let (mut editor, media, id) = one_clip();
    let other = add_at(&mut editor, &media, 20.0);
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    let like = Box::new(clip(&editor, &id));
    editor
        .apply(Command::MatchClip {
            clip_id: other.clone(),
            like,
        })
        .unwrap();
    let matched = clip(&editor, &other);
    assert!(matched.speed_curve.is_none());
    close(matched.speed, 1.0, "own rate");
}

#[test]
fn a_clip_given_other_media_keeps_its_curve() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(eased(&RAMP)), false);
    editor
        .apply(Command::ReplaceClipMedia {
            clip_id: id.clone(),
            item: footage("/ramp-reversed.mp4", 10.0),
            source_start: Some(0.0),
            enhanced: false,
            offset: 0.0,
        })
        .unwrap();
    let replaced = clip(&editor, &id);
    assert_eq!(replaced.speed_curve, Some(eased(&RAMP)));
    close(replaced.duration, 8.0, "length kept");
}

#[test]
fn the_presets_are_these_shapes_in_this_order() {
    /// A preset's id, its points and the mean speed they make.
    type Shipped = (&'static str, &'static [(f64, f64)], f64);
    let expected: [Shipped; 6] = [
        (
            "montage",
            &[
                (0.0, 0.6),
                (0.15, 0.6),
                (0.3, 3.0),
                (0.45, 0.6),
                (0.6, 0.6),
                (0.75, 3.0),
                (0.9, 0.6),
                (1.0, 0.6),
            ],
            1.32,
        ),
        (
            "hero",
            &[
                (0.0, 2.5),
                (0.25, 2.5),
                (0.38, 0.5),
                (0.62, 0.5),
                (0.75, 2.5),
                (1.0, 2.5),
            ],
            1.76,
        ),
        (
            "bullet",
            &[
                (0.0, 3.0),
                (0.3, 3.0),
                (0.42, 0.2),
                (0.58, 0.2),
                (0.7, 3.0),
                (1.0, 3.0),
            ],
            2.216,
        ),
        (
            "jump-cut",
            &[
                (0.0, 1.0),
                (0.38, 1.0),
                (0.45, 8.0),
                (0.55, 8.0),
                (0.62, 1.0),
                (1.0, 1.0),
            ],
            2.19,
        ),
        (
            "flash-in",
            &[(0.0, 8.0), (0.12, 4.0), (0.3, 1.0), (1.0, 1.0)],
            1.87,
        ),
        (
            "flash-out",
            &[(0.0, 1.0), (0.7, 1.0), (0.88, 4.0), (1.0, 8.0)],
            1.87,
        ),
    ];
    assert_eq!(PRESETS.len(), expected.len());
    for (shipped, (id, points, mean)) in PRESETS.iter().zip(expected) {
        assert_eq!(shipped.id, id);
        assert_eq!(shipped.points, points, "{id}");
        // Chosen, a preset is eased through every point.
        let chosen = preset(id).unwrap();
        assert_eq!(chosen, eased(points), "{id}");
        close(mean_of(&chosen), mean, id);
        close(mean_of(&straight(points)), mean, id);
    }
    assert!(preset("slow-mo").is_none());
}

#[test]
fn a_preset_is_known_by_its_points_whether_smooth_or_straight() {
    let hero = preset("hero").unwrap();
    assert_eq!(preset_of(&hero), Some("hero"));
    let mut lines = hero.clone();
    for point in &mut lines {
        point.slope = None;
    }
    assert_eq!(preset_of(&lines), Some("hero"));
    let mut moved = hero.clone();
    moved[3].at += 0.01;
    assert_eq!(preset_of(&moved), None);
    let mut fewer = hero;
    fewer.remove(2);
    assert_eq!(preset_of(&fewer), None);
}

#[test]
fn a_preset_on_a_ten_second_clip_sets_its_length() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, preset("bullet"), false);
    close(
        clip(&editor, &id).duration,
        10.0 / 2.216,
        "ten seconds at the mean",
    );
}

#[test]
fn smooth_means_every_point_has_a_slope() {
    assert!(is_smooth(&eased(&RAMP)));
    assert!(!is_smooth(&straight(&RAMP)));
    assert!(!is_smooth(&[]));
    let mut mixed = eased(&RAMP);
    mixed[1].slope = None;
    assert!(!is_smooth(&mixed));
    // A mix plays as straight lines.
    assert!(curve_of(&mixed).unwrap().slopes().is_none());
    assert!(curve_of(&[]).is_none());
    close(mean_of(&[]), 1.0, "no curve is normal speed");
}

#[test]
fn the_api_spells_a_curve_as_points_with_an_optional_slope() {
    let command: Command = serde_json::from_value(json!({
        "op": "setClipSpeedCurve",
        "clipId": "c1",
        "curve": [
            { "at": 0.0, "speed": 0.5, "slope": 0.0 },
            { "at": 1.0, "speed": 2.0, "slope": 0.0 }
        ],
        "ripple": true
    }))
    .unwrap();
    let Command::SetClipSpeedCurve {
        clip_id,
        curve,
        ripple,
    } = command
    else {
        panic!("not a curve command");
    };
    assert_eq!(clip_id, "c1");
    assert_eq!(curve, Some(eased(&RAMP)));
    assert!(ripple);
    // No curve and no ripple: take the curve off, plainly.
    let command: Command =
        serde_json::from_value(json!({ "op": "setClipSpeedCurve", "clipId": "c1" })).unwrap();
    assert!(matches!(
        command,
        Command::SetClipSpeedCurve {
            curve: None,
            ripple: false,
            ..
        }
    ));
    // A straight point is written without a slope.
    assert_eq!(
        serde_json::to_value(straight(&RAMP)[0]).unwrap(),
        json!({ "at": 0.0, "speed": 0.5 })
    );
}

fn settings() -> DocumentSettings {
    DocumentSettings {
        name: "ramp".to_owned(),
        width: 1280,
        height: 720,
        rate_num: 25,
        rate_den: 1,
        canvas: [0, 0, 0, 255],
    }
}

/// Every clip object in a written project, in the active timeline's flat
/// mirror and in each timeline.
fn clip_objects(document: &mut Value) -> Vec<&mut Value> {
    let mut found = Vec::new();
    let Value::Object(map) = document else {
        return found;
    };
    for (key, value) in map.iter_mut() {
        match (key.as_str(), value) {
            ("clips", Value::Array(clips)) => found.extend(clips.iter_mut()),
            ("timelines", Value::Array(timelines)) => {
                for timeline in timelines {
                    if let Some(Value::Array(clips)) = timeline.get_mut("clips") {
                        found.extend(clips.iter_mut());
                    }
                }
            }
            _ => {}
        }
    }
    found
}

#[test]
fn a_saved_curve_is_written_as_speed_curve_points_and_reads_back_the_same() {
    let (mut editor, _, id) = one_clip();
    let points = vec![
        SpeedPoint {
            at: 0.0,
            speed: 0.5,
            slope: Some(0.0),
        },
        SpeedPoint {
            at: 0.4,
            speed: 3.0,
            slope: Some(1.5),
        },
        SpeedPoint {
            at: 1.0,
            speed: 1.0,
            slope: Some(0.0),
        },
    ];
    set_curve(&mut editor, &id, Some(points.clone()), false);
    let mut document = editor.to_document(&settings());
    let written = clip_objects(&mut document);
    assert!(!written.is_empty());
    for object in written {
        assert_eq!(
            object["speedCurve"],
            json!([
                { "at": 0.0, "speed": 0.5, "slope": 0.0 },
                { "at": 0.4, "speed": 3.0, "slope": 1.5 },
                { "at": 1.0, "speed": 1.0, "slope": 0.0 }
            ])
        );
    }
    let reopened = Editor::from_document(&document).unwrap();
    let back = reopened.project().active().clip(&id).unwrap();
    assert_eq!(back.speed_curve, Some(points));
    assert_eq!(back.speed, clip(&editor, &id).speed);
    assert_eq!(back.duration, clip(&editor, &id).duration);
}

#[test]
fn a_clip_without_a_curve_writes_no_speed_curve_key() {
    let (editor, _, _) = one_clip();
    let mut document = editor.to_document(&settings());
    for object in clip_objects(&mut document) {
        assert!(object.get("speedCurve").is_none());
    }
}

#[test]
fn a_hand_edited_curve_is_tidied_as_it_is_read() {
    let (mut editor, _, id) = one_clip();
    set_curve(&mut editor, &id, Some(straight(&RAMP)), false);
    let mut document = editor.to_document(&settings());
    for object in clip_objects(&mut document) {
        object["speedCurve"] = json!([
            { "at": 0.0, "speed": 99.0 },
            "not a point",
            { "at": 1.5, "speed": 1.0 },
            { "at": 0.5, "speed": 0.0, "slope": 2.0 },
            { "at": 1.0 }
        ]);
    }
    let reopened = Editor::from_document(&document).unwrap();
    let back = reopened.project().active().clip(&id).unwrap();
    // What does not parse is dropped, a point off the clip is dropped, and
    // speeds are held to the range.
    assert_eq!(
        back.speed_curve,
        Some(vec![
            SpeedPoint {
                at: 0.0,
                speed: 16.0,
                slope: None,
            },
            SpeedPoint {
                at: 0.5,
                speed: 0.0625,
                slope: Some(2.0),
            },
        ])
    );
    for object in clip_objects(&mut document) {
        object["speedCurve"] = json!([{ "at": 7.0, "speed": 1.0 }]);
    }
    let reopened = Editor::from_document(&document).unwrap();
    assert!(
        reopened
            .project()
            .active()
            .clip(&id)
            .unwrap()
            .speed_curve
            .is_none(),
        "nothing left is no curve"
    );
    for object in clip_objects(&mut document) {
        object["speedCurve"] = json!("ramp");
    }
    let reopened = Editor::from_document(&document).unwrap();
    assert!(
        reopened
            .project()
            .active()
            .clip(&id)
            .unwrap()
            .speed_curve
            .is_none()
    );
}
