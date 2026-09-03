// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The edit itself: a project, its tracks, and the clips on them.
//!
//! Tracks and clips live in arenas and are addressed by [`TrackId`] and
//! [`ClipId`]. Nothing here owns another editable object directly, which is
//! what makes undo, cross-references and multi-threaded rendering tractable.
//!
//! Track order is explicit and bottom-first: `track_ids()[0]` is composited
//! first and everything after it draws on top.

use std::path::{Path, PathBuf};

use crate::animate::Animation;
use crate::arena::{Arena, Id};
use crate::retime::SpeedCurve;
use crate::time::{FrameRate, Rational, TimeRange};

/// Handle to a [`Track`].
pub type TrackId = Id<Track>;
/// Handle to a [`Clip`].
pub type ClipId = Id<Clip>;

/// What kind of material a track carries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TrackKind {
    /// Picture.
    Video,
    /// Sound.
    Audio,
}

/// Where a clip's pixels come from.
///
/// A path today. When a media library lands this becomes a handle into it, and
/// this is the one type that has to change.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MediaRef {
    /// Path to the media file on disk.
    pub path: PathBuf,
}

impl MediaRef {
    /// References a file on disk.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The referenced path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// A span of one piece of media, placed on a track.
#[derive(Clone, Debug)]
pub struct Clip {
    /// Where the pixels come from.
    pub media: MediaRef,
    /// The in-point: how far into the media the clip starts.
    pub source_start: Rational,
    /// Where the clip sits on the timeline.
    pub start: Rational,
    /// How long the clip runs on the timeline.
    pub duration: Rational,
    /// Playback rate: source seconds consumed per timeline second. One is
    /// normal speed. Always positive - enforce at the boundary that sets it.
    ///
    /// This is the *only* definition of retiming. Everything downstream - the
    /// frame plan, the export decoder rate, the audio graph - derives from it,
    /// so picture and sound cannot disagree about what a sped-up clip means.
    pub speed: Rational,
    /// Blend factor in `0.0..=1.0`, applied over whatever is beneath.
    pub opacity: f32,
    /// Ramp the picture's opacity from zero over this many seconds from the
    /// clip's start. Zero means no ramp.
    ///
    /// This is what a cross-dissolve is made of: the incoming clip overlaps
    /// the outgoing one and ramps in on top. It multiplies `opacity`, so a
    /// half-transparent clip fades up to half, not to solid.
    pub video_fade_in: Rational,
    /// Ramp the opacity back to zero over this many seconds into the clip's
    /// end. Zero means no ramp.
    pub video_fade_out: Rational,
    /// How the picture sits in the frame. Identity is fitted and centred.
    pub transform: Transform,
    /// Speed as it changes over the clip, when it does. Overrides `speed`
    /// for the time map; `speed` is then the curve's mean, kept so a sound
    /// path that needs one number has one. See [`SpeedCurve`].
    pub retime: Option<SpeedCurve>,
    /// Played backwards: the source is consumed from its far end towards
    /// `source_start`.
    pub reverse: bool,
    /// Keys over the clip's placement and opacity, when it has any. See
    /// [`Animation`].
    pub animation: Option<Animation>,
}

/// A clip's placement in the output frame.
///
/// Deliberately resolution-independent: `scale` is relative to the fitted
/// size and the offsets are fractions of the frame, so the same transform
/// means the same picture on a 720p export and a 4K one.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transform {
    /// Multiplier over the fitted size. 1 fills the frame, preserving aspect.
    pub scale: f64,
    /// Offset of the picture's centre from frame centre, as a fraction of
    /// frame width.
    pub offset_x: f64,
    /// Offset as a fraction of frame height.
    pub offset_y: f64,
    /// Clockwise rotation about the picture's centre, in degrees.
    pub rotation: f64,
}

impl Transform {
    /// Fitted, centred, unrotated.
    pub const IDENTITY: Transform = Transform {
        scale: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
        rotation: 0.0,
    };

    /// True when applying this transform would change nothing.
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Clip {
    /// A clip that starts at the beginning of its media and is fully opaque.
    pub fn new(media: MediaRef, start: Rational, duration: Rational) -> Self {
        Self {
            media,
            source_start: Rational::ZERO,
            start,
            duration,
            speed: Rational::ONE,
            opacity: 1.0,
            video_fade_in: Rational::ZERO,
            video_fade_out: Rational::ZERO,
            transform: Transform::IDENTITY,
            retime: None,
            reverse: false,
            animation: None,
        }
    }

    /// Where in the clip `time` falls, `0..=1`; zero outside it.
    pub fn fraction_at(&self, time: Rational) -> f64 {
        if self.duration.is_zero() {
            return 0.0;
        }
        ((time - self.start) / self.duration)
            .as_f64()
            .clamp(0.0, 1.0)
    }

    /// The placement at `time`: the clip's own, moved by its animation.
    pub fn transform_at(&self, time: Rational) -> Transform {
        match &self.animation {
            Some(animation) => animation.transform_at(self.transform, self.fraction_at(time)),
            None => self.transform,
        }
    }

    /// The opacity at `time`, before the fade ramps: the clip's own, scaled
    /// by its animation.
    pub fn opacity_at(&self, time: Rational) -> f32 {
        match &self.animation {
            Some(animation) => animation.opacity_at(self.opacity, self.fraction_at(time)),
            None => self.opacity,
        }
    }

    /// The opacity ramp factor at `time`, in `0.0..=1.0`.
    ///
    /// One at any instant outside the fade windows, and always one for a clip
    /// with no fades - the common case costs two comparisons. This lives on
    /// the clip, next to `source_time_at`, because it is the same kind of
    /// fact: the one definition every renderer has to agree with.
    pub fn video_fade_factor(&self, time: Rational) -> f32 {
        let mut factor = 1.0f32;
        let local = time - self.start;

        if !self.video_fade_in.is_zero() && local < self.video_fade_in {
            factor *= (local / self.video_fade_in).as_f64().clamp(0.0, 1.0) as f32;
        }
        let remaining = self.duration - local;
        if !self.video_fade_out.is_zero() && remaining < self.video_fade_out {
            factor *= (remaining / self.video_fade_out).as_f64().clamp(0.0, 1.0) as f32;
        }
        factor
    }

    /// The span of timeline this clip occupies.
    pub fn range(&self) -> TimeRange {
        TimeRange::new(self.start, self.duration)
    }

    /// True if the clip is on screen at `time`.
    pub fn contains(&self, time: Rational) -> bool {
        self.range().contains(time)
    }

    /// Converts a timeline timestamp into a timestamp within the source media,
    /// or `None` if the clip is not on screen then.
    ///
    /// An affine map: `source_start + (time - start) * speed`. This is the one
    /// piece of arithmetic that every trim, ripple, slip and retime operation
    /// ultimately has to agree with, so it lives in exactly one place.
    pub fn source_time_at(&self, time: Rational) -> Option<Rational> {
        if !self.contains(time) {
            return None;
        }
        let forward = match &self.retime {
            // The area under the curve, in fractions of the clip's length,
            // scaled back to seconds. Approximated to a rational at the end
            // so the rest of the arithmetic stays exact.
            Some(curve) => {
                let x = ((time - self.start) / self.duration).as_f64();
                let consumed = curve.consumed(x) * self.duration.as_f64();
                Rational::approximate(consumed).unwrap_or(Rational::ZERO)
            }
            None => (time - self.start) * self.speed,
        };
        Some(if self.reverse {
            // From the far end back: the last instant lands on the first
            // source frame, and the first on the frame just short of the
            // end, which is the one a forward play would have shown last.
            let span = self.source_duration();
            let back = span - forward;
            self.source_start + back.clamp_to(Rational::ZERO, span)
        } else {
            self.source_start + forward
        })
    }

    /// How much of the source this clip consumes: `duration * speed`, or the
    /// area under the curve when there is one.
    pub fn source_duration(&self) -> Rational {
        match &self.retime {
            Some(curve) => Rational::approximate(curve.mean() * self.duration.as_f64())
                .unwrap_or(self.duration),
            None => self.duration * self.speed,
        }
    }

    /// Whether a decoder can be paced at one rate for this clip: false when
    /// the speed changes over it or it runs backwards, in which case every
    /// frame has to be sought by its own source time.
    pub fn is_paced(&self) -> bool {
        self.retime.is_none() && !self.reverse
    }
}

/// A horizontal lane of clips.
#[derive(Clone, Debug)]
pub struct Track {
    /// Display name.
    pub name: String,
    /// Picture or sound.
    pub kind: TrackKind,
    /// When false the track is skipped entirely while rendering.
    pub enabled: bool,
    clips: Vec<ClipId>,
}

impl Track {
    /// An empty, enabled track.
    pub fn new(name: impl Into<String>, kind: TrackKind) -> Self {
        Self {
            name: name.into(),
            kind,
            enabled: true,
            clips: Vec::new(),
        }
    }

    /// The clips on this track, in insertion order.
    pub fn clips(&self) -> &[ClipId] {
        &self.clips
    }
}

/// The edit: output format, tracks, and the clips on them.
#[derive(Debug)]
pub struct Timeline {
    /// Output frame rate.
    pub frame_rate: FrameRate,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    tracks: Arena<Track>,
    clips: Arena<Clip>,
    order: Vec<TrackId>,
}

impl Timeline {
    /// An empty timeline with the given output format.
    pub fn new(width: u32, height: u32, frame_rate: FrameRate) -> Self {
        Self {
            frame_rate,
            width,
            height,
            tracks: Arena::new(),
            clips: Arena::new(),
            order: Vec::new(),
        }
    }

    /// A 1920x1080, 30 fps timeline - the default for a new project.
    pub fn hd() -> Self {
        Self::new(1920, 1080, FrameRate::THIRTY)
    }

    /// Adds a track on top of the existing ones.
    pub fn add_track(&mut self, track: Track) -> TrackId {
        let id = self.tracks.insert(track);
        self.order.push(id);
        id
    }

    /// Track handles, bottom-most first. Composite in this order.
    pub fn track_ids(&self) -> &[TrackId] {
        &self.order
    }

    /// Borrows a track.
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.get(id)
    }

    /// Mutably borrows a track.
    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.get_mut(id)
    }

    /// Iterates over tracks bottom-most first.
    pub fn tracks(&self) -> impl Iterator<Item = (TrackId, &Track)> {
        self.order
            .iter()
            .filter_map(|&id| self.tracks.get(id).map(|track| (id, track)))
    }

    /// Removes a track and every clip on it.
    pub fn remove_track(&mut self, id: TrackId) -> Option<Track> {
        let track = self.tracks.remove(id)?;
        for &clip in &track.clips {
            self.clips.remove(clip);
        }
        self.order.retain(|&other| other != id);
        Some(track)
    }

    /// Places a clip on a track. Returns `None` if the track handle is stale.
    pub fn add_clip(&mut self, track: TrackId, clip: Clip) -> Option<ClipId> {
        // Insert into the clip arena only once we know the track is real,
        // otherwise a stale handle leaks an orphaned clip.
        self.tracks.get(track)?;
        let id = self.clips.insert(clip);
        self.tracks
            .get_mut(track)
            .expect("checked above")
            .clips
            .push(id);
        Some(id)
    }

    /// Borrows a clip.
    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips.get(id)
    }

    /// Mutably borrows a clip.
    pub fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        self.clips.get_mut(id)
    }

    /// Removes a clip from wherever it sits.
    pub fn remove_clip(&mut self, id: ClipId) -> Option<Clip> {
        let clip = self.clips.remove(id)?;
        for (_, track) in self.tracks.iter_mut() {
            track.clips.retain(|&other| other != id);
        }
        Some(clip)
    }

    /// How many clips the timeline holds, across all tracks.
    pub const fn clip_count(&self) -> usize {
        self.clips.len()
    }

    /// The clip visible on `track` at `time`, if any.
    ///
    /// Clips on one track are not supposed to overlap; if they do, the
    /// last-added one wins, which matches what you see after a paste.
    pub fn clip_on_track_at(&self, track: TrackId, time: Rational) -> Option<ClipId> {
        let track = self.tracks.get(track)?;
        track
            .clips
            .iter()
            .rev()
            .copied()
            .find(|&id| self.clips.get(id).is_some_and(|clip| clip.contains(time)))
    }

    /// Where the last clip ends. Zero for an empty timeline.
    pub fn duration(&self) -> Rational {
        self.clips
            .iter()
            .map(|(_, clip)| clip.range().end())
            .max()
            .unwrap_or(Rational::ZERO)
    }

    /// How many whole output frames the timeline runs for.
    pub fn frame_count(&self) -> i64 {
        self.frame_rate.frames_in(self.duration())
    }
}

/// A timeline plus the things that surround it on disk.
#[derive(Debug)]
pub struct Project {
    /// Display name.
    pub name: String,
    /// The edit.
    pub timeline: Timeline,
}

impl Project {
    /// A new, empty project at the given output format.
    pub fn new(name: impl Into<String>, timeline: Timeline) -> Self {
        Self {
            name: name.into(),
            timeline,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retime::SpeedCurve;

    fn seconds(value: i64) -> Rational {
        Rational::from_int(value)
    }

    fn fixture() -> (Timeline, TrackId, ClipId) {
        let mut timeline = Timeline::hd();
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        let clip = timeline
            .add_clip(
                track,
                Clip::new(MediaRef::new("a.mp4"), seconds(2), seconds(3)),
            )
            .expect("track exists");
        (timeline, track, clip)
    }

    #[test]
    fn a_clip_maps_timeline_time_to_source_time() {
        let (timeline, _, clip) = fixture();
        let clip = timeline.clip(clip).expect("clip exists");

        assert_eq!(clip.source_time_at(seconds(2)), Some(Rational::ZERO));
        assert_eq!(clip.source_time_at(seconds(4)), Some(seconds(2)));
        assert_eq!(clip.source_time_at(seconds(1)), None, "before the clip");
        assert_eq!(
            clip.source_time_at(seconds(5)),
            None,
            "the end is exclusive"
        );
    }

    /// A curve makes the map the area under the speed line, and a reverse
    /// walks it from the far end; both keep the source covered the same.
    #[test]
    fn a_curve_bends_the_map_and_a_reverse_walks_it_backwards() {
        let mut clip = Clip::new(
            MediaRef::new("a.mp4"),
            Rational::ZERO,
            Rational::from_int(10),
        );
        // Half speed for the first half, double for the second: 2.5 s of
        // source in the first 5 s of timeline, 10 s in the second.
        clip.retime = SpeedCurve::new(&[(0.0, 0.5), (0.5, 0.5), (0.5, 2.0), (1.0, 2.0)]);
        fn at(clip: &Clip, seconds: i64) -> f64 {
            clip.source_time_at(Rational::from_int(seconds))
                .unwrap()
                .as_f64()
        }
        assert!((at(&clip, 5) - 2.5).abs() < 1e-6);
        assert!((at(&clip, 9) - 10.5).abs() < 1e-3);
        assert!((clip.source_duration().as_f64() - 12.5).abs() < 1e-6);
        assert!(!clip.is_paced());

        clip.retime = None;
        clip.speed = Rational::from_int(2);
        clip.reverse = true;
        // Backwards at 2x over 10 s covers 20 s of source: the start shows
        // the far end, the middle the middle.
        assert!((at(&clip, 0) - 20.0).abs() < 1e-9);
        assert!((at(&clip, 5) - 10.0).abs() < 1e-9);
        assert!(!clip.is_paced());
    }

    #[test]
    fn speed_scales_the_source_time_map() {
        let (mut timeline, _, id) = fixture();
        {
            let clip = timeline.clip_mut(id).expect("clip exists");
            clip.speed = Rational::from_int(2);
            clip.source_start = seconds(10);
        }
        let clip = timeline.clip(id).expect("clip exists");

        // The clip covers timeline [2, 5) at 2x, so it consumes source [10, 16).
        assert_eq!(clip.source_time_at(seconds(2)), Some(seconds(10)));
        assert_eq!(clip.source_time_at(seconds(4)), Some(seconds(14)));
        assert_eq!(clip.source_duration(), seconds(6));

        // Half speed consumes half the source.
        timeline.clip_mut(id).expect("clip exists").speed = Rational::new(1, 2);
        let clip = timeline.clip(id).expect("clip exists");
        assert_eq!(clip.source_time_at(seconds(4)), Some(seconds(11)));
        assert_eq!(clip.source_duration(), Rational::new(3, 2));
    }

    #[test]
    fn an_in_point_offsets_the_source_time() {
        let (mut timeline, _, id) = fixture();
        timeline.clip_mut(id).expect("clip exists").source_start = seconds(10);
        let clip = timeline.clip(id).expect("clip exists");
        assert_eq!(clip.source_time_at(seconds(2)), Some(seconds(10)));
        assert_eq!(clip.source_time_at(seconds(3)), Some(seconds(11)));
    }

    #[test]
    fn video_fades_ramp_and_hold() {
        let (mut timeline, _, id) = fixture(); // covers timeline [2, 5)
        {
            let clip = timeline.clip_mut(id).expect("clip exists");
            clip.video_fade_in = Rational::ONE;
            clip.video_fade_out = Rational::ONE;
        }
        let clip = timeline.clip(id).expect("clip exists");

        assert_eq!(clip.video_fade_factor(seconds(2)), 0.0, "starts at zero");
        assert_eq!(
            clip.video_fade_factor(Rational::new(5, 2)),
            0.5,
            "halfway up"
        );
        assert_eq!(
            clip.video_fade_factor(seconds(3)),
            1.0,
            "holds at one between fades"
        );
        assert_eq!(
            clip.video_fade_factor(Rational::new(9, 2)),
            0.5,
            "halfway down"
        );
    }

    #[test]
    fn no_fade_means_no_attenuation() {
        let (timeline, _, id) = fixture();
        let clip = timeline.clip(id).expect("clip exists");
        assert_eq!(clip.video_fade_factor(seconds(2)), 1.0);
        assert_eq!(clip.video_fade_factor(seconds(4)), 1.0);
    }

    #[test]
    fn finds_the_clip_under_the_playhead() {
        let (timeline, track, clip) = fixture();
        assert_eq!(timeline.clip_on_track_at(track, seconds(3)), Some(clip));
        assert_eq!(timeline.clip_on_track_at(track, seconds(9)), None);
    }

    #[test]
    fn duration_is_the_end_of_the_last_clip() {
        let (mut timeline, track, _) = fixture();
        assert_eq!(timeline.duration(), seconds(5));

        timeline
            .add_clip(
                track,
                Clip::new(MediaRef::new("b.mp4"), seconds(5), seconds(4)),
            )
            .expect("track exists");
        assert_eq!(timeline.duration(), seconds(9));
        assert_eq!(timeline.frame_count(), 270); // 9s at 30fps
    }

    #[test]
    fn tracks_come_back_bottom_most_first() {
        let mut timeline = Timeline::hd();
        let lower = timeline.add_track(Track::new("V1", TrackKind::Video));
        let upper = timeline.add_track(Track::new("V2", TrackKind::Video));
        assert_eq!(timeline.track_ids(), &[lower, upper]);
    }

    #[test]
    fn removing_a_track_takes_its_clips_with_it() {
        let (mut timeline, track, clip) = fixture();
        timeline.remove_track(track);

        assert!(timeline.clip(clip).is_none());
        assert_eq!(timeline.clip_count(), 0);
        assert!(timeline.track_ids().is_empty());
        assert_eq!(timeline.duration(), Rational::ZERO);
    }

    #[test]
    fn removing_a_clip_unlinks_it_from_its_track() {
        let (mut timeline, track, clip) = fixture();
        assert!(timeline.remove_clip(clip).is_some());

        assert!(
            timeline
                .track(track)
                .expect("track exists")
                .clips()
                .is_empty()
        );
        assert_eq!(timeline.clip_on_track_at(track, seconds(3)), None);
        assert!(
            timeline.remove_clip(clip).is_none(),
            "removing twice is harmless"
        );
    }

    #[test]
    fn a_stale_track_handle_does_not_leak_a_clip() {
        let (mut timeline, track, _) = fixture();
        timeline.remove_track(track);

        let orphan = timeline.add_clip(
            track,
            Clip::new(MediaRef::new("c.mp4"), seconds(0), seconds(1)),
        );
        assert_eq!(orphan, None);
        assert_eq!(timeline.clip_count(), 0);
    }
}
