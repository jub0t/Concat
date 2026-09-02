// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Deciding what is on screen at a given instant.
//!
//! A [`FramePlan`] is the contract between the editor model and whatever draws
//! pixels. It names the media, the exact source timestamp and the blend
//! strength for each visible layer, bottom-most first, and it does so without
//! opening a single file.
//!
//! Keeping this pure is what lets you unit-test "does a cut land on the right
//! frame" in microseconds instead of by exporting a file and squinting at it.

use std::path::PathBuf;

use wolfcut_core::time::Rational;
use wolfcut_core::timeline::{ClipId, Timeline, TrackKind, Transform};

/// One visible layer at one instant.
#[derive(Clone, PartialEq, Debug)]
pub struct PlannedLayer {
    /// Which clip produced this layer.
    pub clip: ClipId,
    /// The file to pull pixels from.
    pub media: PathBuf,
    /// The timestamp *within that file* to pull.
    pub source_time: Rational,
    /// Source seconds consumed per timeline second. A decoder that pulls one
    /// frame per output frame must decode at `output_rate / speed` to stay in
    /// step with `source_time`.
    pub speed: Rational,
    /// Blend strength over everything beneath, in `0.0..=1.0`.
    pub opacity: f32,
    /// The clip's placement in the frame, resolution-independent.
    pub transform: Transform,
}

/// Everything needed to draw one output frame.
#[derive(Clone, PartialEq, Debug)]
pub struct FramePlan {
    /// The timeline instant this describes.
    pub time: Rational,
    /// Output width.
    pub width: u32,
    /// Output height.
    pub height: u32,
    /// Visible layers, bottom-most first.
    pub layers: Vec<PlannedLayer>,
}

impl FramePlan {
    /// True if nothing is on screen. The result is a black frame, not an error:
    /// gaps in a timeline are ordinary.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

/// Works out what is on screen at `time`.
///
/// Audio tracks and disabled tracks are skipped. A track with no clip under the
/// playhead simply contributes nothing.
pub fn plan_frame(timeline: &Timeline, time: Rational) -> FramePlan {
    let mut layers = Vec::new();

    for (track_id, track) in timeline.tracks() {
        if !track.enabled || track.kind != TrackKind::Video {
            continue;
        }
        let Some(clip_id) = timeline.clip_on_track_at(track_id, time) else {
            continue;
        };
        let Some(clip) = timeline.clip(clip_id) else {
            continue;
        };
        let Some(source_time) = clip.source_time_at(time) else {
            continue;
        };

        layers.push(PlannedLayer {
            clip: clip_id,
            media: clip.media.path.clone(),
            source_time,
            speed: clip.speed,
            // The fade ramp multiplies in here, so the compositor only ever
            // sees a per-frame opacity - it has no idea fades exist.
            opacity: (clip.opacity * clip.video_fade_factor(time)).clamp(0.0, 1.0),
            transform: clip.transform,
        });
    }

    FramePlan { time, width: timeline.width, height: timeline.height, layers }
}

#[cfg(test)]
mod tests {
    use wolfcut_core::time::FrameRate;
    use wolfcut_core::timeline::{Clip, MediaRef, Track};

    use super::*;

    fn seconds(value: i64) -> Rational {
        Rational::from_int(value)
    }

    #[test]
    fn stacks_layers_bottom_most_first() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let lower = timeline.add_track(Track::new("V1", TrackKind::Video));
        let upper = timeline.add_track(Track::new("V2", TrackKind::Video));
        timeline
            .add_clip(lower, Clip::new(MediaRef::new("bottom.mp4"), seconds(0), seconds(10)))
            .expect("track exists");
        timeline
            .add_clip(upper, Clip::new(MediaRef::new("top.mp4"), seconds(0), seconds(10)))
            .expect("track exists");

        let plan = plan_frame(&timeline, seconds(4));
        let media: Vec<_> = plan.layers.iter().map(|layer| layer.media.clone()).collect();
        assert_eq!(media, vec![PathBuf::from("bottom.mp4"), PathBuf::from("top.mp4")]);
        assert_eq!((plan.width, plan.height), (640, 360));
    }

    #[test]
    fn maps_each_layer_to_its_own_source_time() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        let mut clip = Clip::new(MediaRef::new("a.mp4"), seconds(5), seconds(10));
        clip.source_start = seconds(30);
        timeline.add_clip(track, clip).expect("track exists");

        let plan = plan_frame(&timeline, seconds(7));
        assert_eq!(plan.layers[0].source_time, seconds(32), "2s into a clip that starts at 30s");
    }

    #[test]
    fn a_retimed_clip_plans_scaled_source_times() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        let mut clip = Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(4));
        clip.speed = Rational::from_int(2);
        timeline.add_clip(track, clip).expect("track exists");

        let plan = plan_frame(&timeline, seconds(3));
        assert_eq!(plan.layers[0].source_time, seconds(6), "3s in at 2x is 6s of source");
        assert_eq!(plan.layers[0].speed, Rational::from_int(2));
    }

    #[test]
    fn a_gap_plans_to_nothing() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        timeline
            .add_clip(track, Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(2)))
            .expect("track exists");

        assert!(plan_frame(&timeline, seconds(5)).is_empty());
    }

    #[test]
    fn skips_disabled_and_audio_tracks() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let muted = timeline.add_track(Track::new("V1", TrackKind::Video));
        let audio = timeline.add_track(Track::new("A1", TrackKind::Audio));
        timeline.track_mut(muted).expect("track exists").enabled = false;

        for track in [muted, audio] {
            timeline
                .add_clip(track, Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(10)))
                .expect("track exists");
        }

        assert!(plan_frame(&timeline, seconds(1)).is_empty());
    }

    #[test]
    fn a_video_fade_ramps_the_planned_opacity() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        let mut clip = Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(4));
        clip.video_fade_in = Rational::from_int(2);
        timeline.add_clip(track, clip).expect("track exists");

        assert_eq!(plan_frame(&timeline, seconds(0)).layers[0].opacity, 0.0);
        assert_eq!(plan_frame(&timeline, seconds(1)).layers[0].opacity, 0.5);
        assert_eq!(plan_frame(&timeline, seconds(3)).layers[0].opacity, 1.0);
    }

    #[test]
    fn clamps_a_nonsense_opacity() {
        let mut timeline = Timeline::new(640, 360, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        let mut clip = Clip::new(MediaRef::new("a.mp4"), seconds(0), seconds(10));
        clip.opacity = 4.0;
        timeline.add_clip(track, clip).expect("track exists");

        assert_eq!(plan_frame(&timeline, seconds(1)).layers[0].opacity, 1.0);
    }
}
