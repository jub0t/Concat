// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How a clip under a speed curve reaches the renderer and the mixer: the
//! points and slopes a project's clip is flattened to, and the sound it is
//! played as - one stream whose tempo steps when the pitch is held, or
//! pieces at a constant rate each when the pitch follows the speed.

use concat_project::commands::NewMedia;
use concat_project::model::{MediaKind, SpeedPoint};
use concat_project::{Command, Editor};

use crate::flatten::flatten_timeline;
use crate::{ClipKind, ExportClip, audio_pieces};

const TIGHT: f64 = 1e-9;

fn close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < TIGHT,
        "{what}: got {actual}, expected {expected}"
    );
}

/// Two seconds at 2 s on the timeline, from 1 s into its file, under a
/// straight ramp from half speed to double.
fn ramped(preserve_pitch: bool) -> ExportClip {
    let mut clip = ExportClip::blank(ClipKind::Video, 2.0, 2.0, 0);
    clip.path = "/ramp.mp4".to_owned();
    clip.source_start = 1.0;
    clip.speed = 1.25;
    clip.speed_curve = vec![(0.0, 0.5), (1.0, 2.0)];
    clip.preserve_pitch = preserve_pitch;
    clip.has_audio = Some(true);
    clip
}

#[test]
fn held_pitch_is_one_stream_whose_tempo_steps_every_tenth_of_a_second() {
    let pieces = audio_pieces(&ramped(true));
    assert_eq!(pieces.len(), 1, "one stream, so no seam is heard");
    let sound = &pieces[0];
    close(sound.start, 2.0, "start");
    close(sound.duration, 2.0, "length");
    close(sound.source_start, 1.0, "in-point");
    close(sound.speed, 1.25, "the curve's mean");
    assert!(sound.preserve_pitch);
    assert_eq!(
        sound.tempo.len(),
        20,
        "a step each tenth of the clip's two seconds"
    );
    for (index, at, speed) in [
        (0, 0.0, 0.5375),
        (5, 0.5, 0.9125),
        (10, 1.0, 1.2875),
        (15, 1.5, 1.6625),
    ] {
        close(sound.tempo[index].0, at, "step time");
        close(sound.tempo[index].1, speed, "step rate: its stretch's mean");
    }
}

#[test]
fn pitch_let_go_is_pieces_each_at_its_stretch_mean_from_where_the_curve_had_got() {
    let pieces = audio_pieces(&ramped(false));
    assert_eq!(pieces.len(), 20);
    for (index, start, source, speed) in [
        (0, 2.0, 1.0, 0.5375),
        (5, 2.5, 1.34375, 0.9125),
        (10, 3.0, 1.875, 1.2875),
        (15, 3.5, 2.59375, 1.6625),
    ] {
        let piece = &pieces[index];
        close(piece.start, start, "piece start");
        close(piece.duration, 0.1, "piece length");
        close(piece.source_start, source, "piece in-point");
        close(piece.speed, speed, "piece rate");
        assert!(piece.tempo.is_empty());
        assert!(!piece.preserve_pitch);
    }
    // Together they cover the footage the curve does: two seconds at 1.25.
    let covered: f64 = pieces
        .iter()
        .map(|piece| piece.duration * piece.speed)
        .sum();
    close(covered, 2.5, "footage covered");
}

#[test]
fn a_flat_stretch_is_one_step_and_a_flat_curve_needs_no_steps() {
    let mut clip = ramped(true);
    clip.duration = 1.0;
    clip.speed_curve = vec![(0.0, 1.0), (0.5, 1.0), (1.0, 3.0)];
    let sound = &audio_pieces(&clip)[0];
    assert_eq!(
        sound.tempo.len(),
        6,
        "the first half one step, then one a tenth"
    );
    close(sound.tempo[0].1, 1.0, "the flat half");
    close(sound.tempo[1].0, 0.5, "the climb starts");
    close(sound.tempo[1].1, 1.2, "first step up");

    clip.speed_curve = vec![(0.0, 3.0), (1.0, 3.0)];
    let sound = &audio_pieces(&clip)[0];
    assert!(sound.tempo.is_empty(), "one rate the whole way");
    close(sound.speed, 3.0, "at that rate");
}

#[test]
fn short_clips_get_eight_steps_and_long_ones_four_hundred() {
    let mut clip = ramped(false);
    clip.duration = 0.3;
    assert_eq!(audio_pieces(&clip).len(), 8);
    clip.duration = 120.0;
    assert_eq!(audio_pieces(&clip).len(), 400);
}

#[test]
fn a_constant_rate_is_one_piece_at_that_rate() {
    let mut clip = ramped(true);
    clip.speed_curve.clear();
    clip.speed = 2.0;
    let pieces = audio_pieces(&clip);
    assert_eq!(pieces.len(), 1);
    close(pieces[0].speed, 2.0, "rate");
    assert!(pieces[0].tempo.is_empty());
}

#[test]
fn curves_with_eased_points_carry_their_slopes_and_straight_ones_none() {
    let mut editor = Editor::new();
    let media = editor
        .apply(Command::AddMedia {
            item: NewMedia {
                path: "/ramp.mp4".to_owned(),
                name: "ramp.mp4".to_owned(),
                duration: Some(10.0),
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
            },
        })
        .unwrap()
        .created_id
        .unwrap();
    let track_id = editor.project().active().tracks[0].id.clone();
    let id = editor
        .apply(Command::AddClip {
            media_id: media,
            track_id,
            start: 0.0,
            ripple: false,
        })
        .unwrap()
        .created_id
        .unwrap();
    let point = |at, speed, slope| SpeedPoint { at, speed, slope };
    for (curve, slopes) in [
        (
            vec![point(0.0, 0.5, Some(0.0)), point(1.0, 2.0, Some(0.5))],
            vec![0.0, 0.5],
        ),
        (vec![point(0.0, 0.5, None), point(1.0, 2.0, None)], vec![]),
        // A curve with slopes on some points only plays straight.
        (
            vec![point(0.0, 0.5, Some(1.0)), point(1.0, 2.0, None)],
            vec![],
        ),
    ] {
        editor
            .apply(Command::SetClipSpeedCurve {
                clip_id: id.clone(),
                curve: Some(curve),
                ripple: false,
            })
            .unwrap();
        let flat = flatten_timeline(editor.project(), None);
        let clip = flat
            .iter()
            .find(|clip| clip.kind == ClipKind::Video)
            .unwrap();
        assert_eq!(clip.speed_curve, vec![(0.0, 0.5), (1.0, 2.0)]);
        assert_eq!(clip.speed_slopes, slopes);
        close(
            clip.speed,
            editor.project().active().clip(&id).unwrap().speed,
            "mean",
        );
        assert_eq!(
            clip.engine_curve().unwrap().tangents().is_some(),
            !slopes.is_empty()
        );
    }
}
