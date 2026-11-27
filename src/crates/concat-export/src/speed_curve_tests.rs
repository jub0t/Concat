// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How a clip under a speed curve reaches the renderer and the mixer: the
//! points and slopes a project's clip is flattened to, and the sound it is
//! played as - one stream whose tempo steps when the pitch is held, or
//! pieces at a constant rate each when the pitch follows the speed.

use concat_doc::commands::Command;
use concat_doc::model::SpeedPoint;

use crate::test_doc::{Doc, at, video};
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

/// A ramp from half speed to double reaches the renderer as a curve by
/// share of the clip from 0.5 to 2, its mean the clip's pace, eased or
/// straight.
#[test]
fn a_ramp_reaches_the_renderer_from_its_first_speed_to_its_last() {
    let mut doc = Doc::new();
    let media = doc.media(video("/ramp.mp4", 10.0, (1280, 720), true));
    let item = doc.item(Command::AddClip {
        media,
        lane: doc.main_lane(),
        start: at(0.0),
        ripple: false,
    });
    let point = |at, speed, slope| SpeedPoint { at, speed, slope };
    for curve in [
        vec![point(0.0, 0.5, Some(0.0)), point(1.0, 2.0, Some(0.5))],
        vec![point(0.0, 0.5, None), point(1.0, 2.0, None)],
    ] {
        doc.apply(Command::SetClipSpeedCurve {
            item,
            curve: Some(curve),
            ripple: false,
        });
        let flat = doc.flat();
        let clip = flat
            .iter()
            .find(|clip| clip.kind == ClipKind::Video)
            .expect("the clip");
        let first = clip.speed_curve.first().copied().expect("a curve");
        let last = clip.speed_curve.last().copied().expect("a curve");
        assert!(
            first.0.abs() < 1e-9 && (first.1 - 0.5).abs() < 0.02,
            "{first:?}"
        );
        assert!(
            (last.0 - 1.0).abs() < 1e-9 && (last.1 - 2.0).abs() < 0.05,
            "{last:?}"
        );
        // Ten seconds of source over the clip's length is its mean.
        assert!((clip.speed - 10.0 / clip.duration).abs() < 1e-3);
        assert!(clip.engine_curve().is_some());
    }
}
