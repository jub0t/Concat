// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The document model: what a Concat project *is*.
//!
//! Times are `f64` seconds here because that is what the on-disk format
//! stores, and the documents that exist freeze the format. The conversion
//! to exact rationals stays at the render boundary - `concat-export`'s
//! timeline builder. Moving the *document* to rational time is a format
//! decision for a deliberate version 2, made once, here.
//!
//! Serde names are camelCase: that is the document's spelling.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What a piece of media is. A still is not a video with one frame: it has no
/// intrinsic duration, so its length on a timeline is editorial.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    /// Moving pictures, with or without embedded sound.
    Video,
    /// Sound only; nothing to composite.
    Audio,
    /// A still, whose timeline length is editorial rather than intrinsic.
    Image,
}

/// What a clip can be - wider than [`MediaKind`] because a text clip has no
/// file behind it; it *is* its own content.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipKind {
    /// A cut of a video media item.
    Video,
    /// A cut of audio - either audio media or sound detached from a video.
    Audio,
    /// A placed still.
    Image,
    /// A title. No media behind it; the content lives in [`Clip::text`].
    Text,
    /// A treatment over everything beneath it for as long as it runs - a
    /// look or an effect placed as a layer. No media behind it; the chain
    /// lives in [`Clip::video_effects`], its strength in [`Clip::opacity`]
    /// and its ramps in the fades.
    Layer,
}

impl ClipKind {
    /// True for the kinds that put pixels on screen.
    pub fn is_visual(self) -> bool {
        matches!(self, ClipKind::Video | ClipKind::Image)
    }
}

/// One entry in the media bin: a file the user imported, plus what the host's
/// probe learned about it. The probe metadata is stored, not re-derived, so a
/// document opens meaningfully even when the file itself is missing.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    /// Minted by the editor ("m1", "m2", ...) and never re-issued, even
    /// across a save/load - clips reference media by this id.
    pub id: String,
    /// Absolute path on the user's disk. Doubles as the duplicate check:
    /// adding the same path twice is a no-op.
    pub path: String,
    /// Display name in the bin, normally the file's basename.
    pub name: String,
    /// Seconds. None when the container did not say.
    pub duration: Option<f64>,
    /// What the probe decided the file is; fixes which [`ClipKind`] its
    /// clips get.
    pub kind: MediaKind,
    /// Pixel width, when the file has pictures and the probe found one.
    pub width: Option<u32>,
    /// Pixel height, same terms as `width`.
    pub height: Option<u32>,
    /// Frames per second as a decimal - convenient for display, not exact.
    pub frame_rate: Option<f64>,
    /// The exact fraction the engine works in, e.g. "30000/1001".
    pub frame_rate_fraction: Option<String>,
    /// Codec name as the probe reported it, e.g. "h264". Informational.
    pub video_codec: Option<String>,
    /// Codec of the embedded audio, when there is any.
    pub audio_codec: Option<String>,
    /// Whether the file carries an audio stream; gates `DetachAudio`.
    pub has_audio: bool,
    /// True when this item is a template slot: a stand-in whose metadata says
    /// what kind of media belongs here, waiting to be replaced by the user's
    /// own file (`Command::FillSlot`). In a creator's own project the path is
    /// still real; only a packed template bundle blanks it. Skipped when
    /// false, so documents without templates stay byte-identical.
    #[serde(default, skip_serializing_if = "is_false")]
    pub placeholder: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// A lane. Deliberately untyped: any media goes on any track.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// Minted per timeline ("t5", or "T1".."T4" for the first timeline's
    /// starter lanes); never shared between timelines.
    pub id: String,
    /// User-editable label. Renames to whitespace are ignored, so it is
    /// never blank.
    pub name: String,
    /// Video clips on this track are left out of the composite when false.
    pub visible: bool,
    /// Audio on this track is silent when true.
    pub muted: bool,
}

/// One applied audio filter or video effect: a catalogue id plus whatever
/// parameters were set. The catalogues themselves (and the FFmpeg strings
/// they build) live in the UI today; the model only stores the numbers.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedFilter {
    /// Which catalogue entry this is, e.g. "sepia". Not minted; an id the
    /// catalogue no longer knows is simply skipped at render time.
    pub id: String,
    /// The user's knob settings, keyed by parameter name. Missing keys mean
    /// the catalogue's defaults; ordered so serialisation is stable.
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
    /// False bypasses without losing settings. Absent means enabled.
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// One point of a speed curve.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedPoint {
    /// Where in the clip, as a fraction of its timeline length, 0..=1.
    pub at: f64,
    /// Source seconds per timeline second there.
    pub speed: f64,
}

/// A transition on the cut into a clip.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    /// Which transition from the catalogue, e.g. "cross-fade".
    pub id: String,
    /// Seconds the transition covers.
    pub duration: f64,
}

/// How a title's lines sit within their block. The block itself is placed by
/// the clip's transform, so this only matters for multi-line text.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    /// Lines share a left edge.
    Left,
    /// Lines are centred on each other - the default, and what the reader
    /// falls back to for an unrecognised value.
    Center,
    /// Lines share a right edge.
    Right,
}

/// A title's styling. Sizes are fractions of the frame, so a title composed
/// against 1080p lands correctly exported at 4K.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    /// The words themselves, newlines included. The clip's display name is
    /// snapshotted from the first non-empty line.
    pub content: String,
    /// CSS-style family name, quotes included where the name needs them,
    /// e.g. `"Cabinet Grotesk"`. May name a [`CustomFont`] the user added.
    pub font_family: String,
    /// Cap height as a fraction of frame height.
    pub font_size: f64,
    /// CSS-scale weight, 100..=900; the reader clamps hand-edited values
    /// into that range.
    pub font_weight: f64,
    /// Italic when true. A style flag, not a separate face.
    pub italic: bool,
    /// Fill colour as a CSS hex string, e.g. "#ffffff".
    pub color: String,
    /// Line alignment within the block; see [`TextAlign`].
    pub align: TextAlign,
    /// Opacity of the whole title in 0..=1, multiplied with the clip's own
    /// opacity.
    pub opacity: f64,
    /// Outline thickness as a fraction of frame height. Zero - the default -
    /// means no stroke.
    pub stroke_width: f64,
    /// Outline colour, only visible when `stroke_width` is non-zero.
    pub stroke_color: String,
    /// A drop shadow for legibility over footage. On by default.
    pub shadow: bool,
    /// A solid plate behind the text. Empty string for none.
    pub background: String,
    /// Baseline spacing as a multiple of the font size; the reader floors it
    /// at 0.5 so lines cannot collapse onto each other.
    pub line_height: f64,
    /// Extra letter spacing, in the same frame-height fractions as
    /// `font_size`. Zero is the font's natural fit.
    pub tracking: f64,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            content: "Your text".to_owned(),
            font_family: "\"Cabinet Grotesk\"".to_owned(),
            font_size: 0.09,
            font_weight: 700.0,
            italic: false,
            color: "#ffffff".to_owned(),
            align: TextAlign::Center,
            opacity: 1.0,
            stroke_width: 0.0,
            stroke_color: "#000000".to_owned(),
            shadow: true,
            background: String::new(),
            line_height: 1.2,
            tracking: 0.0,
        }
    }
}

/// One placed piece of a timeline: a stretch of media (or a title) with its
/// timing, mix, transform, and effects. Everything an edit decision touches
/// lives here, which is why most commands are clip commands.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    /// Minted by the editor ("c1", "c2", ...); how every command names its
    /// target. Survives splits - the head keeps the id - and merges.
    pub id: String,
    /// The lane this clip sits on. Always a track of the same timeline; the
    /// reader drops clips whose track vanished.
    pub track_id: String,
    /// Empty for a text clip, which has no file behind it.
    pub media_id: String,
    /// Display label. Snapshotted from the media's name (or a title's first
    /// line) at creation, then the user's to change.
    pub name: String,
    /// What this clip renders as. Follows the media's kind, and is rewritten
    /// when a template slot is filled with a different kind of file.
    pub kind: ClipKind,
    /// Seconds from the start of the timeline.
    pub start: f64,
    /// Seconds of timeline the clip occupies. Source seconds divided by
    /// `speed`; never below a sixtieth of a second.
    pub duration: f64,
    /// In-point: how far into the media the clip starts.
    pub source_start: f64,
    /// Linear gain, 1 being unity.
    pub volume: f64,
    /// Seconds of audio ramp-in from silence at the head. Zero for none.
    pub fade_in: f64,
    /// Seconds of audio ramp-out to silence at the tail. Zero for none.
    pub fade_out: f64,
    /// Multiplier over the fitted size. 1 fills the frame, preserving aspect.
    pub scale: f64,
    /// Offset from centred, as a fraction of frame width / height.
    pub offset_x: f64,
    /// The vertical half of `offset_x`'s pair; positive moves down.
    pub offset_y: f64,
    /// Clockwise rotation in degrees, about the picture's centre.
    pub rotation: f64,
    /// Blend strength over whatever is beneath, in 0..1.
    pub opacity: f64,
    /// Playback rate. 1 is normal. With a curve set this is the curve's
    /// mean, kept in step by the commands that set either.
    pub speed: f64,
    /// Speed as it changes over the clip: points of `(at, speed)`, `at` a
    /// fraction of the clip's timeline length. None is the constant rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_curve: Option<Vec<SpeedPoint>>,
    /// Played backwards.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
    /// Keep voices at their natural pitch when `speed` is not 1. On by
    /// default; off gives the tape-machine chipmunk/slow-motion sound.
    pub preserve_pitch: bool,
    /// Audio filters, in order - order is audible.
    #[serde(default)]
    pub filters: Vec<AppliedFilter>,
    /// Video effects, in order - the visual sibling of `filters`.
    #[serde(default)]
    pub video_effects: Vec<AppliedFilter>,
    /// True when a video clip's embedded audio is detached out of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    /// On a detached audio clip: the video clip the sound came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached_from: Option<String>,
    /// The transition on the cut into this clip, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<Transition>,
    /// The overlay, when this is a text clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextStyle>,
}

/// One timeline: a name and its lanes and clips. Every operation takes the
/// project and works on whichever timeline is active.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    /// Minted by the editor ("tl2", ...), except the founding "TL1".
    pub id: String,
    /// The tab label; "Timeline N" by default, renameable but never blank.
    pub name: String,
    /// The lanes, top to bottom. Never empty - `RemoveTrack` keeps a floor
    /// of one.
    pub tracks: Vec<Track>,
    /// Every clip on this timeline, in insertion order, not time order -
    /// readers must sort by `start` where order matters.
    pub clips: Vec<Clip>,
}

/// A font the user added from disk.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomFont {
    /// The family name titles refer to; removal leaves referring clips
    /// untouched so the face can come back when the file does.
    pub family: String,
    /// Where the font file lives on disk. Doubles as the duplicate check
    /// when adding.
    pub path: String,
}

/// The edit: everything the document stores except the app-level settings
/// (name, output format) that the host manages around it.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// The bin: every imported file, shared by all timelines.
    pub media: Vec<MediaItem>,
    /// Fonts the user added from disk, available to every title.
    pub fonts: Vec<CustomFont>,
    /// Every timeline, in tab order. Always at least one.
    pub timelines: Vec<Timeline>,
    /// Which timeline commands act on. Maintained by the command layer, so
    /// it always names a member of `timelines`; [`Project::active`] degrades
    /// to the first timeline if it somehow does not.
    pub active_timeline_id: String,
}

impl Project {
    /// A new project: one timeline, four lanes.
    pub fn new() -> Self {
        Self {
            media: Vec::new(),
            fonts: Vec::new(),
            timelines: vec![Timeline {
                id: "TL1".to_owned(),
                name: "Timeline 1".to_owned(),
                tracks: (1..=4)
                    .map(|number| Track {
                        id: format!("T{number}"),
                        name: format!("Track {number}"),
                        visible: true,
                        muted: false,
                    })
                    .collect(),
                clips: Vec::new(),
            }],
            active_timeline_id: "TL1".to_owned(),
        }
    }

    /// The timeline being edited. The active id is maintained by the command
    /// layer, so a broken invariant here is a bug, not user input - but it
    /// degrades to the first timeline rather than panicking.
    pub fn active(&self) -> &Timeline {
        self.timelines
            .iter()
            .find(|timeline| timeline.id == self.active_timeline_id)
            .unwrap_or(&self.timelines[0])
    }

    /// Mutable twin of [`Project::active`], with the same degrade-to-first
    /// behaviour.
    pub fn active_mut(&mut self) -> &mut Timeline {
        let index = self
            .timelines
            .iter()
            .position(|timeline| timeline.id == self.active_timeline_id)
            .unwrap_or(0);
        &mut self.timelines[index]
    }

    /// The bin entry with this id, or None if it was removed.
    pub fn media_by_id(&self, media_id: &str) -> Option<&MediaItem> {
        self.media.iter().find(|item| item.id == media_id)
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Timeline {
    /// The clip with this id, or None if it is not on this timeline - which
    /// most commands treat as a tolerated no-op, not an error.
    pub fn clip(&self, clip_id: &str) -> Option<&Clip> {
        self.clips.iter().find(|clip| clip.id == clip_id)
    }

    /// Mutable twin of [`Timeline::clip`].
    pub fn clip_mut(&mut self, clip_id: &str) -> Option<&mut Clip> {
        self.clips.iter_mut().find(|clip| clip.id == clip_id)
    }

    /// The track with this id, or None if it was removed.
    pub fn track(&self, track_id: &str) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == track_id)
    }

    /// Where the last clip ends. Zero for an empty timeline.
    pub fn duration(&self) -> f64 {
        self.clips
            .iter()
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max)
    }
}
