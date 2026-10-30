// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

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
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// What a piece of media is. A still is not a video with one frame: it has no
/// intrinsic duration, so its length on a timeline is editorial.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    /// Moving pictures, with or without embedded sound.
    #[default]
    Video,
    /// Sound only; nothing to composite.
    Audio,
    /// A still, whose timeline length is editorial rather than intrinsic.
    Image,
}

/// What made a media file, when it was the editor and not the user's
/// import. The bin shelves such a file under Generated, by origin, and
/// keeps it out of the import shelves.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaOrigin {
    /// Read aloud by the speech sheet.
    Speech,
    /// A clip's sound rendered as it played - trimmed, at its speed, with
    /// its level, fades and effects baked in - as a file of its own.
    Processed,
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
    /// A treatment of the sound for as long as it runs: its chain, in
    /// [`Clip::filters`], works on the mix of everything audible over its
    /// span - a voice changer, a room or a telephone over the whole
    /// soundtrack, not one clip of it. No media behind it; how much of the
    /// treated sound replaces the untreated is [`Clip::opacity`], the
    /// strength a picture layer has, and its ramps are the fades. A kind of
    /// its own rather than a flag on [`ClipKind::Layer`], so it stays what
    /// it is with its chain emptied, and a build that predates it leaves it
    /// out as a clip it cannot play rather than reading it as a picture
    /// layer that treats nothing.
    #[serde(rename = "soundLayer")]
    SoundLayer,
    /// A shape: a square, a circle, an arrow, drawn over the picture. No
    /// media behind it; what it is and how it is painted lives in
    /// [`Clip::shape`], and it is placed and scaled like a still.
    Shape,
}

impl ClipKind {
    /// True for the kinds that put pixels on screen.
    pub fn is_visual(self) -> bool {
        matches!(self, ClipKind::Video | ClipKind::Image)
    }
}

/// One audio stream of a media file, as the probe reported it. A screen
/// recording keeps the desktop's sound and the microphone as two of these;
/// a clip names the one it plays by `index`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    /// The stream's index within the file - what [`Clip::audio_stream`] holds.
    pub index: u32,
    /// Codec short name, e.g. "aac". Informational.
    #[serde(default)]
    pub codec: String,
    /// Channel count: 1 mono, 2 stereo.
    #[serde(default)]
    pub channels: u32,
    /// Samples per second.
    #[serde(default)]
    pub sample_rate: u32,
    /// The name the file gives the track, when it gives one ("Desktop
    /// Audio", "Mic/Aux"); empty otherwise, and the UI numbers it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// The track's language tag, e.g. "eng"; empty when unstated.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
}

/// The levels a media file's picture spans, when the person says so over
/// whatever the file claims: video range, 16-235, or full range, 0-255.
/// A screen recording written full and tagged nothing plays grey where it
/// should be black until it is read as `full`; a video-range file tagged
/// full crushes its shadows until it is read as `limited`. Set with
/// `Command::SetMediaColorRange`; absent means the file's own tag is read.
/// https://github.com/jub0t/Concat/issues/103
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorRange {
    /// 16-235: what broadcast, cameras and every player expect.
    Limited,
    /// 0-255: what screen recorders and some phones write.
    Full,
}

/// One entry in the media bin: a file the user imported, plus what the host's
/// probe learned about it. The probe metadata is stored, not re-derived, so a
/// document opens meaningfully even when the file itself is missing.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
    #[serde(deserialize_with = "wire::media_kind")]
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
    /// Every audio stream the file carries, in file order, when the probe
    /// listed them. Empty for a file without sound and for a document from
    /// before the list was kept - `has_audio` still says whether there is
    /// sound at all. More than one is a recording with its tracks apart, and
    /// what lets a clip choose between them. Skipped when empty, so
    /// documents without such media stay byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "wire::list")]
    pub audio_tracks: Vec<AudioTrack>,
    /// True when this item is a template slot: a stand-in whose metadata says
    /// what kind of media belongs here, waiting to be replaced by the user's
    /// own file (`Command::FillSlot`). In a creator's own project the path is
    /// still real; only a packed template bundle blanks it. Skipped when
    /// false, so documents without templates stay byte-identical.
    #[serde(default, skip_serializing_if = "is_false")]
    pub placeholder: bool,
    /// The levels the picture is read as, over the file's own tag; see
    /// [`ColorRange`]. Absent, and left out of the document, for the tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub color_range: Option<ColorRange>,
    /// What the probe found the picture recorded in: HLG or PQ for HDR.
    /// Left out of the document for SDR - and absent from one written
    /// before it was kept, which reads as SDR.
    #[serde(default, skip_serializing_if = "ColorSpace::is_sdr")]
    #[serde(deserialize_with = "wire::color_space")]
    pub color_space: ColorSpace,
    /// Where the file came from when the editor made it; see
    /// [`MediaOrigin`]. Absent, and left out of the document, for an
    /// import, and read as absent when it names an origin this build does
    /// not know - the file is still a file, just one shelved with the
    /// imports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub origin: Option<MediaOrigin>,
    /// Fields this build does not know, kept so a document written by a
    /// newer or a different build round-trips through this one intact.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

fn unity() -> f64 {
    1.0
}

fn is_unity(value: &f64) -> bool {
    *value == 1.0
}

fn is_false(value: &bool) -> bool {
    !value
}

fn is_true(value: &bool) -> bool {
    *value
}

/// A lane. Deliberately untyped: any media goes on any track.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Track {
    /// Minted per timeline ("t5", or "T1".."T4" for the first timeline's
    /// starter lanes); never shared between timelines.
    pub id: String,
    /// Video clips on this track are left out of the composite when false.
    pub visible: bool,
    /// Audio on this track is silent when true.
    pub muted: bool,
    /// Fields this build does not know, kept so they round-trip.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Default for Track {
    fn default() -> Self {
        Track {
            id: String::new(),
            visible: true,
            muted: false,
            extra: Map::new(),
        }
    }
}

/// One applied audio filter or video effect: a catalogue id plus whatever
/// parameters were set. The catalogues themselves (and the FFmpeg strings
/// they build) live in the UI today; the model only stores the numbers.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// The knobs that ride rather than hold: a sorted run of keys per
    /// parameter name, each `at` a fraction of the clip like a `ClipKey`'s.
    /// A parameter with keys is played from them and its `params` entry is
    /// only what it falls back to with the keys taken off.
    #[serde(
        default,
        deserialize_with = "wire::runs",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub keys: BTreeMap<String, Vec<ParamKey>>,
}

fn yes() -> bool {
    true
}

/// One key on one parameter of an applied effect; see `AppliedFilter::keys`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamKey {
    /// Where in the clip, as a fraction of its timeline length, `0..=1`.
    pub at: f64,
    /// The value there, in the parameter's own units.
    pub value: f64,
    /// How this key is approached from the previous one.
    #[serde(default)]
    pub ease: KeyEase,
}

impl AppliedFilter {
    /// A link with nothing set: the package's defaults, enabled.
    pub fn new(id: impl Into<String>) -> AppliedFilter {
        AppliedFilter {
            id: id.into(),
            params: BTreeMap::new(),
            enabled: true,
            keys: BTreeMap::new(),
        }
    }

    /// This parameter's keys, in order; empty for one that holds still.
    pub fn keys_on(&self, key: &str) -> &[ParamKey] {
        self.keys.get(key).map_or(&[], Vec::as_slice)
    }

    /// Whether this parameter rides at all.
    pub fn is_keyed(&self, key: &str) -> bool {
        !self.keys_on(key).is_empty()
    }

    /// The index into this parameter's keys of the one at `at`, within
    /// `KEY_EPSILON`, choosing the nearest when two are in reach.
    pub fn key_at(&self, key: &str, at: f64) -> Option<usize> {
        self.keys_on(key)
            .iter()
            .enumerate()
            .filter(|(_, held)| (held.at - at).abs() <= KEY_EPSILON)
            .min_by(|(_, a), (_, b)| (a.at - at).abs().total_cmp(&(b.at - at).abs()))
            .map(|(index, _)| index)
    }

    /// This parameter's keys as the engine plays them.
    pub fn track_on(&self, key: &str) -> concat_core::animate::Track {
        concat_core::animate::Track::new(
            self.keys_on(key)
                .iter()
                .map(|held| concat_core::animate::Key {
                    at: held.at,
                    value: held.value,
                    ease: held.ease.into(),
                })
                .collect(),
        )
    }

    /// What a parameter is worth at `at`: its ride where it is keyed, and
    /// otherwise what `params` holds, or `fallback` - the package's default,
    /// which the model does not know - when that holds nothing either.
    pub fn value_at(&self, key: &str, at: f64, fallback: f64) -> f64 {
        let constant = self.params.get(key).copied().unwrap_or(fallback);
        if !self.is_keyed(key) {
            return constant;
        }
        self.track_on(key).value_at(at, constant)
    }

    /// Every set parameter at `at`: `params` with each keyed one replaced
    /// by its ride's value there. What a renderer resolves a frame from.
    pub fn params_at(&self, at: f64) -> BTreeMap<String, f64> {
        let mut set = self.params.clone();
        for (key, held) in &self.keys {
            if held.is_empty() {
                continue;
            }
            let rest = self.params.get(key).copied().unwrap_or(0.0);
            set.insert(key.clone(), self.track_on(key).value_at(at, rest));
        }
        set
    }

    /// The nearest key strictly before `at`, and the nearest strictly after;
    /// see `Clip::keys_around`.
    pub fn keys_around(&self, key: &str, at: f64) -> (Option<f64>, Option<f64>) {
        let mut before: Option<f64> = None;
        let mut after: Option<f64> = None;
        for held in self.keys_on(key) {
            if held.at < at - KEY_EPSILON {
                before = Some(before.map_or(held.at, |seen: f64| seen.max(held.at)));
            } else if held.at > at + KEY_EPSILON {
                after = Some(after.map_or(held.at, |seen: f64| seen.min(held.at)));
            }
        }
        (before, after)
    }

    /// Sets a key, replacing whichever key on that parameter was already
    /// within `KEY_EPSILON` of `at`. Keeps the run sorted by `at`.
    pub fn set_key(&mut self, key: &str, at: f64, value: f64, ease: KeyEase) {
        if !at.is_finite() || !value.is_finite() {
            return;
        }
        let at = at.clamp(0.0, 1.0);
        let slot = self.key_at(key, at);
        let run = self.keys.entry(key.to_owned()).or_default();
        match slot {
            Some(index) => run[index] = ParamKey { at, value, ease },
            None => run.push(ParamKey { at, value, ease }),
        }
        run.sort_by(|a, b| a.at.total_cmp(&b.at));
    }

    /// Removes this parameter's key at `at`, if there is one. True when a
    /// key actually went; the run goes with its last key.
    pub fn clear_key(&mut self, key: &str, at: f64) -> bool {
        let Some(index) = self.key_at(key, at) else {
            return false;
        };
        if let Some(run) = self.keys.get_mut(key) {
            run.remove(index);
            if run.is_empty() {
                self.keys.remove(key);
            }
        }
        true
    }

    /// Takes every key off one parameter. True when there were any.
    pub fn clear_keys(&mut self, key: &str) -> bool {
        self.keys.remove(key).is_some_and(|run| !run.is_empty())
    }

    /// Drops keys that are not finite or not in `0..=1`, empty runs with
    /// them, and orders what is left. For the document reader.
    pub fn sort_keys(&mut self) {
        for run in self.keys.values_mut() {
            run.retain(|key| {
                key.at.is_finite() && key.value.is_finite() && (0.0..=1.0).contains(&key.at)
            });
            run.sort_by(|a, b| a.at.total_cmp(&b.at));
        }
        self.keys.retain(|_, run| !run.is_empty());
    }
}

/// How a picture's background is taken away when there is no key colour
/// to remove: a person mask found by the cutout model, alone or corrected
/// by hand.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cutout {
    /// Automatic keeps what the model finds; custom adds the strokes.
    pub mode: CutoutMode,
    /// What the model looks for.
    #[serde(default, skip_serializing_if = "Subject::is_auto")]
    pub subject: Subject,
    /// How far the edge is softened, as a fraction of the picture's width.
    #[serde(default = "default_feather")]
    pub feather: f64,
    /// Corrections painted on the monitor, in the order they were made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<Stroke>,
}

/// The two ways a cutout decides what stays.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CutoutMode {
    /// The model's mask as it is.
    Auto,
    /// The model's mask, then the strokes over it.
    Custom,
}

/// What a cutout keeps: the person the matting model finds, the one
/// thing the picture is of whatever it is, or whichever of those the
/// analysis decides fits the footage.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Subject {
    /// A person when the person model finds one, the object otherwise.
    #[default]
    Auto,
    /// The person model, always.
    Person,
    /// The object model, always.
    Object,
}

impl Subject {
    /// The default, left out of the document.
    pub fn is_auto(&self) -> bool {
        *self == Subject::Auto
    }

    /// The name the mask cache is keyed by.
    pub fn key(&self) -> &'static str {
        match self {
            Subject::Auto => "auto",
            Subject::Person => "person",
            Subject::Object => "object",
        }
    }
}

/// One brush stroke over a cutout: which tool, how wide, and where it went.
/// Points are fractions of the source picture, `(0, 0)` its top-left, so a
/// stroke survives a change of output size, a crop or a flip.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    /// What the stroke does to the mask beneath it.
    pub tool: BrushTool,
    /// The brush's diameter as a fraction of the picture's width.
    pub size: f64,
    /// The path, as `[x, y]` fractions.
    pub points: Vec<[f64; 2]>,
    /// The source instant, in seconds, the stroke was painted at: the
    /// frame a smart brush reads the thing under it from. Absent on
    /// strokes from before it was recorded, which paint as plain discs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
}

/// The four brushes of a custom cutout.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrushTool {
    /// Keeps what the model thought might be the subject under the stroke.
    SmartBrush,
    /// Keeps everything under the stroke.
    Brush,
    /// Removes what the model was unsure of under the stroke.
    SmartEraser,
    /// Removes everything under the stroke.
    Eraser,
}

/// The edge softness a cutout starts with.
pub const DEFAULT_FEATHER: f64 = 0.01;
/// The widest an edge may be softened.
pub const MAX_FEATHER: f64 = 0.1;
/// The brush's narrowest and widest, as fractions of the picture's width.
pub const MIN_BRUSH: f64 = 0.005;
/// See [`MIN_BRUSH`].
pub const MAX_BRUSH: f64 = 0.5;

fn default_feather() -> f64 {
    DEFAULT_FEATHER
}

impl Cutout {
    /// An automatic cutout with the default edge.
    pub fn auto() -> Cutout {
        Cutout {
            mode: CutoutMode::Auto,
            subject: Subject::Auto,
            feather: DEFAULT_FEATHER,
            strokes: Vec::new(),
        }
    }

    /// The feather held to its range, every stroke to its own, and strokes
    /// with nowhere to go dropped.
    pub fn tidy(mut self) -> Cutout {
        self.feather = if self.feather.is_finite() {
            self.feather.clamp(0.0, MAX_FEATHER)
        } else {
            DEFAULT_FEATHER
        };
        self.strokes = self.strokes.into_iter().filter_map(Stroke::tidy).collect();
        self
    }
}

impl Stroke {
    /// Whether the stroke's tool reads the picture rather than painting
    /// a disc: the smart brush and the smart eraser.
    pub fn is_smart(&self) -> bool {
        matches!(self.tool, BrushTool::SmartBrush | BrushTool::SmartEraser)
    }

    /// The size held to its range and the points to the picture; `None`
    /// for a stroke with no points left.
    pub fn tidy(mut self) -> Option<Stroke> {
        self.size = if self.size.is_finite() {
            self.size.clamp(MIN_BRUSH, MAX_BRUSH)
        } else {
            MIN_BRUSH
        };
        self.at = self.at.filter(|at| at.is_finite()).map(|at| at.max(0.0));
        self.points.retain(|[x, y]| x.is_finite() && y.is_finite());
        for point in &mut self.points {
            point[0] = point[0].clamp(-0.5, 1.5);
            point[1] = point[1].clamp(-0.5, 1.5);
        }
        (!self.points.is_empty()).then_some(self)
    }
}

/// A clip's stabilization: the camera's shake taken out of the picture.
///
/// The camera's movement is measured once, frame to frame, by the host's
/// analysis and cached in the project folder beside the masks; these are
/// the settings the smooth path and the correction are worked out from,
/// live, whenever they change. Off is no `Stabilize` at all.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stabilize {
    /// How much of the camera's movement is smoothed away, `0..=1`: low
    /// keeps a pan or a glide and takes out the jitter, high leaves only
    /// the slowest of moves. Measured on the timeline, so a clip sped up
    /// is smoothed as calmly as one at its own pace.
    pub smoothness: f64,
    /// The most the picture may be zoomed to hide the edges the correction
    /// moves into view, `1..=MAX_STABILIZE_ZOOM`. The zoom used is the
    /// least the whole clip needs, up to this; past it the correction is
    /// eased where it would show an edge.
    pub max_zoom: f64,
    /// Lock the frame entirely, as on a tripod, rather than follow the
    /// camera's intended move.
    #[serde(skip_serializing_if = "is_false")]
    pub tripod: bool,
    /// Leave the camera's roll as shot: only its drift and its zoom are
    /// corrected.
    #[serde(skip_serializing_if = "is_false")]
    pub keep_rotation: bool,
}

/// The smoothness a stabilization starts with.
pub const DEFAULT_STABILIZE_SMOOTHNESS: f64 = 0.5;
/// The zoom a stabilization may use to begin with: a fifth larger.
pub const DEFAULT_STABILIZE_ZOOM: f64 = 1.2;
/// The most a stabilization may zoom: half again.
pub const MAX_STABILIZE_ZOOM: f64 = 1.5;

impl Default for Stabilize {
    fn default() -> Self {
        Stabilize {
            smoothness: DEFAULT_STABILIZE_SMOOTHNESS,
            max_zoom: DEFAULT_STABILIZE_ZOOM,
            tripod: false,
            keep_rotation: false,
        }
    }
}

impl Stabilize {
    /// Every number held to its range, a number that is not one to its
    /// default.
    pub fn tidy(self) -> Stabilize {
        Stabilize {
            smoothness: if self.smoothness.is_finite() {
                self.smoothness.clamp(0.0, 1.0)
            } else {
                DEFAULT_STABILIZE_SMOOTHNESS
            },
            max_zoom: if self.max_zoom.is_finite() {
                self.max_zoom.clamp(1.0, MAX_STABILIZE_ZOOM)
            } else {
                DEFAULT_STABILIZE_ZOOM
            },
            ..self
        }
    }
}

/// A crop: what is taken off each edge, as fractions of the source.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crop {
    /// Off the left, `0..1`.
    pub left: f64,
    /// Off the top.
    pub top: f64,
    /// Off the right.
    pub right: f64,
    /// Off the bottom.
    pub bottom: f64,
}

impl Crop {
    /// True when nothing is cut.
    pub fn is_none(&self) -> bool {
        self.left <= 0.0 && self.top <= 0.0 && self.right <= 0.0 && self.bottom <= 0.0
    }

    /// Each edge held to `0..=0.9`, and a pair that would meet pulled back
    /// so at least a tenth of the picture is left.
    pub fn tidy(self) -> Crop {
        let mut out = Crop {
            left: self.left.clamp(0.0, 0.9),
            top: self.top.clamp(0.0, 0.9),
            right: self.right.clamp(0.0, 0.9),
            bottom: self.bottom.clamp(0.0, 0.9),
        };
        if out.left + out.right > 0.9 {
            out.right = (0.9 - out.left).max(0.0);
        }
        if out.top + out.bottom > 0.9 {
            out.bottom = (0.9 - out.top).max(0.0);
        }
        out
    }
}

/// What fills the frame where the picture does not reach: the bars either
/// side of a wide shot in a tall frame, or the margin round a picture
/// scaled down. Nothing - black, as every project was before there was a
/// choice - a blurred, enlarged copy of the clip's own frame, a colour, or
/// a still from the bin.
///
/// A timeline carries one ([`Timeline::background`]) and a clip may carry
/// its own over it ([`Clip::background`]); which clips have one at all is
/// [`Timeline::background_of`]'s to say.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Background {
    /// Black: nothing drawn behind the picture.
    #[default]
    None,
    /// The clip's own frame, enlarged to cover the whole frame and blurred.
    Blur {
        /// How soft, `0..=1`: a blur of a twelfth of the frame's shorter
        /// side at one.
        amount: f64,
    },
    /// One flat colour, `#rrggbb`.
    Color {
        /// The colour, as a title's colours are written.
        color: String,
    },
    /// A still from the bin, enlarged to cover the frame.
    Image {
        /// The bin item. One that has gone, or none yet, draws nothing.
        media_id: String,
    },
}

impl Background {
    /// How soft a blur starts: soft enough that the copy behind reads as
    /// colour and light rather than as a second picture.
    pub const DEFAULT_BLUR: f64 = 0.5;

    /// True for black, the background that draws nothing.
    pub fn is_none(&self) -> bool {
        *self == Background::None
    }

    /// The amount held to `0..=1` and a colour that is not `#rrggbb` turned
    /// to black: what the document reader and every command make of a value
    /// before it is kept. A still naming no media stays one - the kind is
    /// chosen and the still not yet - and draws nothing.
    pub fn tidy(self) -> Background {
        match self {
            Background::Blur { amount } => Background::Blur {
                amount: if amount.is_finite() {
                    amount.clamp(0.0, 1.0)
                } else {
                    Background::DEFAULT_BLUR
                },
            },
            Background::Color { color } => Background::Color {
                color: match rgb_of(&color) {
                    Some([r, g, b]) => format!("#{r:02x}{g:02x}{b:02x}"),
                    None => "#000000".to_owned(),
                },
            },
            other => other,
        }
    }

    /// The colour's channels, for a colour background whose colour reads.
    pub fn rgb(&self) -> Option<[u8; 3]> {
        match self {
            Background::Color { color } => rgb_of(color),
            _ => None,
        }
    }
}

/// `#rrggbb` (or `#rrggbbaa`, its alpha ignored) as three channels.
/// `#rrggbb` (opaque) or `#rrggbbaa` as four straight channels.
fn rgba_of(hex: &str) -> Option<[u8; 4]> {
    let [r, g, b] = rgb_of(hex)?;
    let digits = hex.trim().strip_prefix('#')?;
    let a = match digits.len() {
        8 => u8::from_str_radix(&digits[6..8], 16).ok()?,
        _ => 255,
    };
    Some([r, g, b, a])
}

fn rgb_of(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.trim().strip_prefix('#')?;
    if !(digits.len() == 6 || digits.len() == 8) || !digits.is_ascii() {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// The two ways a picture can be sized to the frame in one go.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Framing {
    /// The whole picture in the frame, as large as it goes: bars where the
    /// shapes differ. Every picture's size before it is touched.
    Fit,
    /// The frame covered, the picture centred and cut at the edges that
    /// overhang it.
    Fill,
}

/// The scale over the fitted size at which a `picture` (its pixels, after
/// the crop) turned by `rotation` degrees fits in or covers `frame`,
/// centred. Scale one is contain-fitted unturned, which is what a picture
/// is drawn at before it is scaled ([`Clip::scale`]); a turn asks more of a
/// fill and less of a fit, since a turned rectangle's corners reach
/// further.
pub fn framing_scale(
    framing: Framing,
    picture: (f64, f64),
    frame: (f64, f64),
    rotation: f64,
) -> Option<f64> {
    let (cw, ch) = picture;
    let (width, height) = frame;
    if !(cw > 0.0 && ch > 0.0 && width > 0.0 && height > 0.0 && rotation.is_finite()) {
        return None;
    }
    let fit = (width / cw).min(height / ch);
    let (fw, fh) = (cw * fit, ch * fit);
    let turn = rotation.to_radians();
    let (cos, sin) = (turn.cos().abs(), turn.sin().abs());
    let scale = match framing {
        // The turned picture's bounding box inside the frame.
        Framing::Fit => (width / (fw * cos + fh * sin)).min(height / (fw * sin + fh * cos)),
        // The frame, turned into the picture's own axes, inside the picture.
        Framing::Fill => ((width * cos + height * sin) / fw).max((width * sin + height * cos) / fh),
    };
    // Rounded to a millionth, so a fit of an unturned picture is exactly
    // the one it has always been drawn at, not a hair under.
    let scale = (scale * 1e6).round() / 1e6;
    Some(scale.clamp(ranges::MIN_SCALE, ranges::MAX_SCALE))
}

/// Which shape a clip's mask keeps. The names are written into project
/// files, so they are stable for ever.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskShape {
    /// The part of the picture on one side of a line through the centre.
    Linear,
    /// A band along a line through the centre: the line mirrored.
    Mirror,
    /// An ellipse; a circle when its box is square. The default, and what
    /// the reader falls back to.
    #[default]
    Circle,
    /// A box, its corners rounded by the mask's `roundness`.
    Rectangle,
    /// A heart, point down.
    Heart,
    /// A five-pointed star, point up.
    Star,
}

impl MaskShape {
    /// Every shape, in the order the pickers list them.
    pub const ALL: [MaskShape; 6] = [
        MaskShape::Linear,
        MaskShape::Mirror,
        MaskShape::Circle,
        MaskShape::Rectangle,
        MaskShape::Heart,
        MaskShape::Star,
    ];

    /// The engine's shape.
    pub fn engine(self) -> concat_core::mask::MaskShape {
        use concat_core::mask::MaskShape as Engine;
        match self {
            MaskShape::Linear => Engine::Linear,
            MaskShape::Mirror => Engine::Mirror,
            MaskShape::Circle => Engine::Circle,
            MaskShape::Rectangle => Engine::Rectangle,
            MaskShape::Heart => Engine::Heart,
            MaskShape::Star => Engine::Star,
        }
    }
}

/// A shape that keeps part of a clip's picture: CapCut's Mask. Drawn over
/// the picture as it is seen - after the crop and the flips, before the
/// placement - so it moves, scales and turns with the clip, and multiplies
/// with the cutout's matte and everything else that weighs the picture.
///
/// Its numbers other than the shape and the invert can be keyed, as the
/// placement's can (`KeyProperty::MaskX` and on); a keyed number's keys
/// replace the constant here.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClipMask {
    /// What it keeps.
    #[serde(deserialize_with = "wire::mask_shape")]
    pub shape: MaskShape,
    /// The centre's offset from the picture's centre, as a fraction of the
    /// picture's width; positive is right.
    pub x: f64,
    /// The same down the picture's height; positive is down.
    pub y: f64,
    /// The box's width, as a fraction of the picture's shorter side - the
    /// unit that keeps a circle round on any aspect.
    pub width: f64,
    /// The box's height, on the same terms; a mirror's thickness.
    pub height: f64,
    /// Clockwise degrees about the mask's centre.
    pub rotation: f64,
    /// How round a rectangle's corners are, `0..=1`.
    pub roundness: f64,
    /// How soft the edge is, `0..=1`: one fades across half the picture's
    /// shorter side.
    pub feather: f64,
    /// Keeps the outside of the shape instead of the inside.
    #[serde(skip_serializing_if = "is_false")]
    pub invert: bool,
}

impl Default for ClipMask {
    fn default() -> Self {
        ClipMask::of(MaskShape::Circle)
    }
}

impl ClipMask {
    /// The smallest a mask's box may be, each way.
    pub const MIN_SIZE: f64 = 0.01;
    /// The largest, each way: four times the shorter side covers the
    /// longest side of any picture a person would mask.
    pub const MAX_SIZE: f64 = 4.0;
    /// How far off the picture's centre a mask may be moved, in its sides.
    pub const MAX_OFFSET: f64 = 2.0;

    /// A mask of `shape` at the size and place a new one starts from: in
    /// the middle, two thirds of the shorter side across - a band a third
    /// thick, a rectangle a little wider than tall.
    pub fn of(shape: MaskShape) -> ClipMask {
        let (width, height) = match shape {
            MaskShape::Linear => (1.0, 1.0),
            MaskShape::Mirror => (1.0, 0.33),
            MaskShape::Rectangle => (0.8, 0.5),
            MaskShape::Circle | MaskShape::Heart | MaskShape::Star => (0.66, 0.66),
        };
        ClipMask {
            shape,
            x: 0.0,
            y: 0.0,
            width,
            height,
            rotation: 0.0,
            roundness: 0.0,
            feather: 0.0,
            invert: false,
        }
    }

    /// A number held to the range its field may hold; see [`Self::tidy`].
    /// What a key on that number is held to as well.
    pub fn clamp(property: KeyProperty, value: f64) -> f64 {
        match property {
            KeyProperty::MaskX | KeyProperty::MaskY => {
                value.clamp(-Self::MAX_OFFSET, Self::MAX_OFFSET)
            }
            KeyProperty::MaskWidth | KeyProperty::MaskHeight => {
                value.clamp(Self::MIN_SIZE, Self::MAX_SIZE)
            }
            KeyProperty::MaskRoundness | KeyProperty::MaskFeather => value.clamp(0.0, 1.0),
            // A turn is a turn however many times round; a key past one is
            // a spin, so neither is wrapped.
            _ => value,
        }
    }

    /// The number a keyable property names, or None for a property that
    /// is not the mask's.
    pub fn get(&self, property: KeyProperty) -> Option<f64> {
        Some(match property {
            KeyProperty::MaskX => self.x,
            KeyProperty::MaskY => self.y,
            KeyProperty::MaskWidth => self.width,
            KeyProperty::MaskHeight => self.height,
            KeyProperty::MaskRotation => self.rotation,
            KeyProperty::MaskRoundness => self.roundness,
            KeyProperty::MaskFeather => self.feather,
            _ => return None,
        })
    }

    /// Sets the number a keyable property names, held to its range. A
    /// property that is not the mask's changes nothing.
    pub fn set(&mut self, property: KeyProperty, value: f64) {
        if !value.is_finite() {
            return;
        }
        let value = Self::clamp(property, value);
        match property {
            KeyProperty::MaskX => self.x = value,
            KeyProperty::MaskY => self.y = value,
            KeyProperty::MaskWidth => self.width = value,
            KeyProperty::MaskHeight => self.height = value,
            KeyProperty::MaskRotation => self.rotation = value,
            KeyProperty::MaskRoundness => self.roundness = value,
            KeyProperty::MaskFeather => self.feather = value,
            _ => {}
        }
    }

    /// Every number back in its range, a number that is not one replaced
    /// by a new mask's.
    pub fn tidy(self) -> ClipMask {
        let fresh = ClipMask::of(self.shape);
        let mut out = self;
        for property in KeyProperty::MASK {
            let value = self.get(property).filter(|value| value.is_finite());
            let value = value.or_else(|| fresh.get(property)).unwrap_or(0.0);
            out.set(property, value);
        }
        out
    }

    /// The engine's mask with these numbers.
    pub fn engine(&self) -> concat_core::mask::ShapeMask {
        concat_core::mask::ShapeMask {
            shape: self.shape.engine(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            rotation: self.rotation,
            roundness: self.roundness,
            feather: self.feather,
            invert: self.invert,
        }
    }
}

/// Which property a user-set key belongs to.
///
/// Five of the first six are the picture's placement and the sixth is the
/// mix's; the seven after them are the mask's numbers. They are one
/// enum because a key is a key - the panel that sets them, the commands that
/// carry them and the document that stores them do not care which side of
/// the clip a value ends up on, and only the thing that finally reads them
/// does.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyProperty {
    /// `Clip::scale`.
    Scale,
    /// `Clip::offset_x`.
    OffsetX,
    /// `Clip::offset_y`.
    OffsetY,
    /// `Clip::rotation`.
    Rotation,
    /// `Clip::opacity`.
    Opacity,
    /// `Clip::volume`.
    Volume,
    /// `ClipMask::x`.
    MaskX,
    /// `ClipMask::y`.
    MaskY,
    /// `ClipMask::width`.
    MaskWidth,
    /// `ClipMask::height`.
    MaskHeight,
    /// `ClipMask::rotation`.
    MaskRotation,
    /// `ClipMask::roundness`.
    MaskRoundness,
    /// `ClipMask::feather`.
    MaskFeather,
}

impl KeyProperty {
    /// The name this property carries in a document and across the export
    /// boundary. Matches the `ExportKey::property` vocabulary.
    pub fn name(self) -> &'static str {
        match self {
            KeyProperty::Scale => "scale",
            KeyProperty::OffsetX => "offsetX",
            KeyProperty::OffsetY => "offsetY",
            KeyProperty::Rotation => "rotation",
            KeyProperty::Opacity => "opacity",
            KeyProperty::Volume => "volume",
            KeyProperty::MaskX => "maskX",
            KeyProperty::MaskY => "maskY",
            KeyProperty::MaskWidth => "maskWidth",
            KeyProperty::MaskHeight => "maskHeight",
            KeyProperty::MaskRotation => "maskRotation",
            KeyProperty::MaskRoundness => "maskRoundness",
            KeyProperty::MaskFeather => "maskFeather",
        }
    }

    /// The property of that name, or None.
    pub fn from_name(name: &str) -> Option<KeyProperty> {
        Some(match name {
            "scale" => KeyProperty::Scale,
            "offsetX" => KeyProperty::OffsetX,
            "offsetY" => KeyProperty::OffsetY,
            "rotation" => KeyProperty::Rotation,
            "opacity" => KeyProperty::Opacity,
            "volume" => KeyProperty::Volume,
            "maskX" => KeyProperty::MaskX,
            "maskY" => KeyProperty::MaskY,
            "maskWidth" => KeyProperty::MaskWidth,
            "maskHeight" => KeyProperty::MaskHeight,
            "maskRotation" => KeyProperty::MaskRotation,
            "maskRoundness" => KeyProperty::MaskRoundness,
            "maskFeather" => KeyProperty::MaskFeather,
            _ => return None,
        })
    }

    /// Every property that can be keyed, in the order a panel lists them.
    pub const ALL: [KeyProperty; 13] = [
        KeyProperty::Scale,
        KeyProperty::OffsetX,
        KeyProperty::OffsetY,
        KeyProperty::Rotation,
        KeyProperty::Opacity,
        KeyProperty::Volume,
        KeyProperty::MaskX,
        KeyProperty::MaskY,
        KeyProperty::MaskWidth,
        KeyProperty::MaskHeight,
        KeyProperty::MaskRotation,
        KeyProperty::MaskRoundness,
        KeyProperty::MaskFeather,
    ];

    /// The mask's numbers: the properties a clip's mask carries.
    pub const MASK: [KeyProperty; 7] = [
        KeyProperty::MaskX,
        KeyProperty::MaskY,
        KeyProperty::MaskWidth,
        KeyProperty::MaskHeight,
        KeyProperty::MaskRotation,
        KeyProperty::MaskRoundness,
        KeyProperty::MaskFeather,
    ];

    /// Whether this is one of the mask's numbers.
    pub fn is_mask(self) -> bool {
        KeyProperty::MASK.contains(&self)
    }
}

/// How a key is approached from the one before it: a CSS timing function's
/// two control points, `[x1, y1, x2, y2]`.
///
/// The document's spelling of `concat_core::animate::Ease`, which is what it
/// becomes. Four numbers rather than a named shape because the curve editor
/// hands people a bezier to drag and the four named shapes are only its
/// preset chips - see `KeyEase::LINEAR` and friends.
///
/// Serialised as a bare array: `"ease": [0.42, 0, 0.58, 1]`. Documents
/// written before the curve editor spell it `"linear"` / `"in"` / `"out"` /
/// `"inOut"` instead, and `from_value` still reads those.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
#[serde(transparent)]
pub struct KeyEase(pub [f64; 4]);

impl<'de> Deserialize<'de> for KeyEase {
    /// Either spelling, and a straight line for anything else: the
    /// reader's standing rule is that a hand-edited file degrades to
    /// something openable, and a key that still moves is better than one
    /// that fails its clip.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Name(String),
            Points(Vec<f64>),
        }
        let value = Value::deserialize(deserializer)?;
        Ok(match serde_json::from_value::<Wire>(value) {
            Ok(Wire::Name(name)) => KeyEase::from_name(&name),
            Ok(Wire::Points(points)) if points.len() == 4 => {
                KeyEase([points[0], points[1], points[2], points[3]]).sane()
            }
            _ => KeyEase::LINEAR,
        })
    }
}

impl Default for KeyEase {
    fn default() -> Self {
        KeyEase::LINEAR
    }
}

impl KeyEase {
    /// A straight line.
    pub const LINEAR: KeyEase = KeyEase([0.0, 0.0, 1.0, 1.0]);
    /// Starts slow, arrives fast.
    pub const IN: KeyEase = KeyEase([0.42, 0.0, 1.0, 1.0]);
    /// Starts fast, arrives slow.
    pub const OUT: KeyEase = KeyEase([0.0, 0.0, 0.58, 1.0]);
    /// Slow at both ends.
    pub const IN_OUT: KeyEase = KeyEase([0.42, 0.0, 0.58, 1.0]);

    /// The four presets and what to call them, in the order the panel's
    /// chips sit in.
    pub const PRESETS: [(&'static str, KeyEase); 4] = [
        ("Linear", KeyEase::LINEAR),
        ("In", KeyEase::IN),
        ("Out", KeyEase::OUT),
        ("In · Out", KeyEase::IN_OUT),
    ];

    /// The ease named by a v1 document, or linear for a name this build has
    /// never heard of - a document naming something unknown should still
    /// open and still move.
    pub fn from_name(name: &str) -> KeyEase {
        match name {
            "in" => KeyEase::IN,
            "out" => KeyEase::OUT,
            "inOut" => KeyEase::IN_OUT,
            _ => KeyEase::LINEAR,
        }
    }

    /// The four numbers, with the two x values clamped where a legal timing
    /// function keeps them and anything unreadable falling back to linear.
    pub fn sane(self) -> KeyEase {
        let KeyEase([x1, y1, x2, y2]) = self;
        if ![x1, y1, x2, y2].iter().all(|n| n.is_finite()) {
            return KeyEase::LINEAR;
        }
        KeyEase([x1.clamp(0.0, 1.0), y1, x2.clamp(0.0, 1.0), y2])
    }

    /// Whether this is one of the presets, to within a hair. What lights a
    /// chip in the panel.
    pub fn is(self, other: KeyEase) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .all(|(a, b)| (a - b).abs() < 0.005)
    }
}

impl From<KeyEase> for concat_core::animate::Ease {
    /// The one conversion into what the engine plays. Sanitised on the way,
    /// so a hand-edited document cannot hand the solver an x outside `0..=1`,
    /// where a cubic bezier stops being a function of x and the Newton
    /// iteration stops converging.
    fn from(ease: KeyEase) -> Self {
        let KeyEase([x1, y1, x2, y2]) = ease.sane();
        concat_core::animate::Ease::new(x1, y1, x2, y2)
    }
}

/// One user-set key: a property, a point in the clip, and the value there.
///
/// The value is *absolute* - the number the inspector shows, in the
/// property's own units - and not the relative factor the engine's
/// `animate::Key` carries. Storing what the user typed is what keeps a key
/// meaning the same thing after the clip's own scale or gain is changed
/// underneath it; the conversion to relative happens on the way out, in
/// `concat_export::flatten::export_keys`.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipKey {
    /// Which property.
    pub property: KeyProperty,
    /// Where in the clip, as a fraction of its timeline length, `0..=1`.
    pub at: f64,
    /// The value there, in the property's own units.
    pub value: f64,
    /// How this key is approached from the previous one.
    #[serde(default)]
    pub ease: KeyEase,
}

/// How near two keys on one property have to be to count as the same key.
///
/// A fraction and not a duration, because that is what `at` is. Two
/// thousandths of a clip is under a frame for anything up to about twenty
/// seconds, which is the length where "the playhead is on that key" stops
/// being a question a user can answer by looking.
pub const KEY_EPSILON: f64 = 0.002;

/// One point of a speed curve.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedPoint {
    /// Where in the clip, as a fraction of its timeline length, 0..=1.
    pub at: f64,
    /// Source seconds per timeline second there.
    pub speed: f64,
    /// How fast the speed is changing there, speed per unit of `at`, for a
    /// curve with smooth bends: present on every point of such a curve and
    /// absent on every point of one joined by straight lines. See
    /// `concat_core::SpeedCurve`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slope: Option<f64>,
}

/// A clip's Enhance, once it has a copy: see [`Clip::enhanced`].
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Enhanced {
    /// The bin item the clip was cut from.
    pub original: String,
    /// The bin item holding the enhanced copy.
    pub copy: String,
    /// Whether the clip shows the copy.
    pub on: bool,
    /// Seconds into the original the copy starts at: a copy of the span the
    /// clip covers, rather than of the whole file, starts partway in. The
    /// clip's in-point is in the copy's seconds while it shows the copy, so
    /// switching between the two moves it by this much and the same moment
    /// stays on screen. Zero, and left out of the document, for a copy of
    /// the whole file.
    #[serde(default, skip_serializing_if = "is_zero_seconds")]
    pub offset: f64,
}

fn is_zero_seconds(seconds: &f64) -> bool {
    *seconds == 0.0
}

/// A transition on the cut into a clip.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    /// Which transition from the catalogue, e.g. "cross-fade".
    pub id: String,
    /// Seconds the transition covers.
    #[serde(default = "unity")]
    pub duration: f64,
    /// The package's knobs as set on this cut, by key; a knob left out is
    /// at its default. Empty for every transition saved before knobs could
    /// be set, which play exactly as they did.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, f64>,
}

/// How a title's lines sit within their block, and which point of the block
/// the clip's position pins: a centred title is placed by its middle, a
/// left-aligned one by its block's left edge and a right-aligned one by its
/// right edge. Typing more into a left-aligned title grows it to the right
/// and leaves its left edge where it was, as alignment means everywhere.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    /// Lines share a left edge, which is where the clip's position is.
    Left,
    /// Lines are centred on each other and on the clip's position - the
    /// default, and what the reader falls back to for an unrecognised value.
    Center,
    /// Lines share a right edge, which is where the clip's position is.
    Right,
}

/// Which figure a shape clip draws. The names are written into project
/// files, so they are stable for ever.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeKind {
    /// Four equal sides; the default, and what the reader falls back to.
    #[default]
    Square,
    /// A disc.
    Circle,
    /// Equilateral, point up.
    Triangle,
    /// A slanted rectangle, leaning right.
    Parallelogram,
    /// A rectangle narrower at the top.
    Trapezoid,
    /// A horizontal rule, `stroke_width` thick.
    Line,
    /// A horizontal rule with a head on its right end.
    Arrow,
}

impl ShapeKind {
    /// The name a document writes, and what a library payload carries.
    pub fn id(self) -> &'static str {
        match self {
            ShapeKind::Square => "square",
            ShapeKind::Circle => "circle",
            ShapeKind::Triangle => "triangle",
            ShapeKind::Parallelogram => "parallelogram",
            ShapeKind::Trapezoid => "trapezoid",
            ShapeKind::Line => "line",
            ShapeKind::Arrow => "arrow",
        }
    }

    /// The figure `id` names, if this build knows it.
    pub fn parse(id: &str) -> Option<ShapeKind> {
        [
            ShapeKind::Square,
            ShapeKind::Circle,
            ShapeKind::Triangle,
            ShapeKind::Parallelogram,
            ShapeKind::Trapezoid,
            ShapeKind::Line,
            ShapeKind::Arrow,
        ]
        .into_iter()
        .find(|kind| kind.id() == id)
    }
}

/// How a shape clip is painted. Sizes are fractions of the frame's height,
/// as a title's are, so a shape composed against 1080p is the same shape at
/// 4K. A style arriving in a command needs only the fields it changes.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShapeStyle {
    /// The figure.
    #[serde(deserialize_with = "wire::shape_kind")]
    pub kind: ShapeKind,
    /// The figure's longer side, as a fraction of frame height.
    pub size: f64,
    /// The inside's colour as `#rrggbb[aa]`; empty for an outline alone.
    pub fill: String,
    /// The outline's colour, drawn when `stroke_width` is above zero. A
    /// line and an arrow are all stroke, drawn in this colour, and fall
    /// back to `fill` when it is empty.
    pub stroke: String,
    /// The outline's thickness as a fraction of frame height; zero for a
    /// figure with no outline. A line's and an arrow's own thickness.
    pub stroke_width: f64,
}

impl ShapeStyle {
    /// The default look of one figure: solid white, no outline.
    pub fn of(kind: ShapeKind) -> ShapeStyle {
        ShapeStyle {
            kind,
            ..ShapeStyle::default()
        }
    }

    /// Every number back in its range; see [`TextStyle::tidy`].
    pub fn tidy(mut self) -> ShapeStyle {
        fn finite(value: f64, fallback: f64) -> f64 {
            if value.is_finite() { value } else { fallback }
        }
        let base = ShapeStyle::default();
        self.size = finite(self.size, base.size).clamp(0.01, 2.0);
        self.stroke_width = finite(self.stroke_width, base.stroke_width).clamp(0.0, 0.5);
        self
    }
}

impl Default for ShapeStyle {
    fn default() -> ShapeStyle {
        ShapeStyle {
            kind: ShapeKind::Square,
            // A third of the frame's height: big enough to be the point of
            // the frame, small enough to leave the picture around it.
            size: 0.35,
            fill: "#ffffff".to_owned(),
            stroke: String::new(),
            // A line or an arrow is nothing but its stroke, so the default
            // is a visible rule and not zero; a filled figure ignores it
            // until it is given an outline colour.
            stroke_width: 0.012,
        }
    }
}

/// Display casing, independent of the editable source text.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextCase {
    /// Display all cased letters in uppercase.
    Upper,
    /// Display all cased letters in lowercase.
    Lower,
    /// Lowercase the text and capitalize sentence initials.
    Sentence,
    /// Preserve the editable source's capitalization.
    #[default]
    #[serde(other)]
    Original,
}

/// What a stretch of a title's words wears over the title's own look: each
/// field is an override, and one left out is the title's. Every word of a
/// title is the title's style until a run says otherwise, so a title with
/// no runs is exactly the title it always was.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunStyle {
    /// Fill colour as `#rrggbb`, over the title's `color`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// CSS-scale weight, 100..=900, over the title's `fontWeight`; bold is
    /// 700, regular 400.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<f64>,
    /// Italic or upright, over the title's `italic`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    /// Underlined or not, over the title's `underline`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    /// Struck through or not, over the title's `strikethrough`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strikethrough: Option<bool>,
    /// Size as a multiple of the title's `fontSize`, 0.1..=10: 1.5 is half
    /// as big again. Relative, not a size of its own, so a word made bigger
    /// stays bigger by as much when the whole title is resized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_scale: Option<f64>,
    /// Outline colour, over the title's `strokeColor`; shows where the
    /// title has an outline at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<String>,
}

impl RunStyle {
    /// No override at all: the title's own look.
    pub fn is_empty(&self) -> bool {
        *self == RunStyle::default()
    }

    /// `over` laid on top of this one: what it says wins, what it leaves
    /// out stays as this one has it.
    pub fn and(&self, over: &RunStyle) -> RunStyle {
        RunStyle {
            color: over.color.clone().or_else(|| self.color.clone()),
            font_weight: over.font_weight.or(self.font_weight),
            italic: over.italic.or(self.italic),
            underline: over.underline.or(self.underline),
            strikethrough: over.strikethrough.or(self.strikethrough),
            font_scale: over.font_scale.or(self.font_scale),
            stroke_color: over
                .stroke_color
                .clone()
                .or_else(|| self.stroke_color.clone()),
        }
    }
}

/// A stretch of a title's words with a look of its own: some words in
/// another colour, one in bold. `start` and `end` count characters - Unicode
/// scalar values, what `str::chars` yields - into the title's `content`,
/// not bytes, so a caller in any language counts them the way it counts a
/// string; `end` is one past the last. Kept sorted, apart and non-empty;
/// see `TextStyle::style_range` and `TextStyle::edit_content`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    /// The first character the look covers.
    pub start: usize,
    /// One past the last.
    pub end: usize,
    /// The look.
    #[serde(flatten)]
    pub style: RunStyle,
}

/// A title's styling. Sizes are fractions of the frame, so a title composed
/// against 1080p lands correctly exported at 4K. A style arriving in a
/// command needs only the fields it changes; the rest are the default's, so
/// a caller can say `{ "content": "Hello" }` and get a title that looks like
/// one the window would place.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextStyle {
    /// The words themselves, newlines included. The clip's display name is
    /// snapshotted from the first non-empty line.
    pub content: String,
    /// CSS-style family name, quotes included where the name needs them,
    /// e.g. `"Hanken Grotesk"`. May name a [`CustomFont`] the user added.
    pub font_family: String,
    /// Cap height as a fraction of frame height.
    pub font_size: f64,
    /// CSS-scale weight, 100..=900; the reader clamps hand-edited values
    /// into that range.
    pub font_weight: f64,
    /// Italic when true. A style flag, not a separate face.
    pub italic: bool,
    /// Case is applied for painting; the editable source stays intact.
    pub text_case: TextCase,
    /// Draw an underline using the selected face's metrics.
    pub underline: bool,
    /// Draw a strike through the text using the selected face's metrics.
    pub strikethrough: bool,
    /// Fill colour as a CSS hex string, e.g. "#ffffff".
    pub color: String,
    /// Line alignment within the block; see [`TextAlign`].
    #[serde(deserialize_with = "wire::text_align")]
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
    /// A solid background behind the text, `#rrggbb[aa]`; its alpha is the
    /// background's opacity. Empty string for none. The painter and older
    /// comments call it the plate.
    pub background: String,
    /// The background's corner radius as a fraction of frame height, so a
    /// title composed against 1080p keeps its corners at 4K. The default is
    /// the rounding every background wore before it had a dial - 15 % of
    /// the default size's em - and zero is square.
    pub background_radius: f64,
    /// The air between the words and the background's left and right
    /// edges, as a fraction of frame height, on an axis the words size
    /// themselves: a box with a width is exactly that wide. The default
    /// is the air every background had before it was a dial, 35 % of the
    /// default size's em.
    pub background_padding_x: f64,
    /// The same above and below; 20 % of the default em by default.
    pub background_padding_y: f64,
    /// Baseline spacing as a multiple of the font size; the reader floors it
    /// at 0.5 so lines cannot collapse onto each other.
    pub line_height: f64,
    /// Extra letter spacing, in the same frame-height fractions as
    /// `font_size`. Zero is the font's natural fit.
    pub tracking: f64,
    /// The widest a line may run, as a fraction of frame width, before its
    /// words wrap onto the next; the stage's side grips set it. Zero - the
    /// default - is no limit: a line is as long as its words. With a limit
    /// the block is a box exactly that wide, plate included, and the lines
    /// align inside it.
    pub max_width: f64,
    /// The box's height as a fraction of frame height; the stage's top and
    /// bottom grips set it, and the inspector's Height field. Zero - the
    /// default - is the words' own height. With a height the block is a
    /// box exactly that tall, the words centred in it, so a lower third
    /// can be a band of a fixed size whatever is written on it.
    /// https://github.com/jub0t/Concat/issues/119
    pub max_height: f64,
    /// Stretches of the words with a look of their own over the rest of
    /// this style - a word in another colour, in bold, bigger. Empty for a
    /// title in one look, and then not written at all, so a document
    /// without runs is the document it always was and an older build reads
    /// a document with them as the title in its own look.
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "wire::list")]
    pub runs: Vec<TextRun>,
}

impl TextStyle {
    /// The words as they are painted: the content in the style's case,
    /// leaving the content itself - what is typed, saved and exported as
    /// subtitles - as it was written. See [`TextStyle::display`] for the
    /// runs that go with it.
    pub fn display_content(&self) -> String {
        self.display().0
    }

    /// Every number pulled into the range a title can be drawn at: a
    /// hand-edited zero size would render an invisible title, and lines
    /// cannot collapse onto each other.
    pub fn tidy(mut self) -> TextStyle {
        fn finite(value: f64, fallback: f64) -> f64 {
            if value.is_finite() { value } else { fallback }
        }
        let base = TextStyle::default();
        self.font_size = finite(self.font_size, base.font_size).clamp(0.01, 1.0);
        self.font_weight = finite(self.font_weight, base.font_weight).clamp(100.0, 900.0);
        self.opacity = finite(self.opacity, base.opacity).clamp(0.0, 1.0);
        self.stroke_width = finite(self.stroke_width, base.stroke_width).max(0.0);
        self.line_height = finite(self.line_height, base.line_height).max(0.5);
        self.tracking = finite(self.tracking, base.tracking);
        self.max_width = finite(self.max_width, base.max_width).max(0.0);
        self.max_height = finite(self.max_height, base.max_height).max(0.0);
        self.background_radius = finite(self.background_radius, base.background_radius).max(0.0);
        self.background_padding_x =
            finite(self.background_padding_x, base.background_padding_x).max(0.0);
        self.background_padding_y =
            finite(self.background_padding_y, base.background_padding_y).max(0.0);
        self.tidy_runs();
        self
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            content: "Your text".to_owned(),
            // The face Concat ships (`concat_text::BUNDLED_FAMILY`), so a
            // title made without one - by the API, a template, the agent -
            // is drawn in it everywhere rather than in a fallback.
            font_family: "Hanken Grotesk".to_owned(),
            font_size: 0.09,
            font_weight: 700.0,
            italic: false,
            text_case: TextCase::Original,
            underline: false,
            strikethrough: false,
            color: "#ffffff".to_owned(),
            align: TextAlign::Center,
            opacity: 1.0,
            stroke_width: 0.0,
            stroke_color: "#000000".to_owned(),
            shadow: true,
            background: String::new(),
            background_radius: 0.0135,
            background_padding_x: 0.0315,
            background_padding_y: 0.018,
            line_height: 1.2,
            tracking: 0.0,
            max_width: 0.0,
            max_height: 0.0,
            runs: Vec::new(),
        }
    }
}

/// Reading with tolerance. The document reader's standing rule is that a
/// hand-edited or older file degrades to something openable: a list keeps
/// the entries that parse and drops the rest, an optional keeps a value
/// that parses and is otherwise nothing, and a kind this build has never
/// heard of falls back to the plainest one. Every helper reads a
/// [`serde_json::Value`] first, so nothing here can fail the whole load.
pub(crate) mod wire {
    use std::collections::BTreeMap;

    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    use super::{
        Background, ClipKind, ColorSpace, MaskShape, MediaKind, ParamKey, ShapeKind, TextAlign,
    };

    /// The entries of a list that parse, in order; not a list at all is
    /// an empty one.
    pub fn list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(match value {
            Value::Array(items) => items
                .into_iter()
                .filter_map(|item| serde_json::from_value(item).ok())
                .collect(),
            _ => Vec::new(),
        })
    }

    /// [`list`], as an optional that is none when nothing parsed.
    pub fn maybe_list<'de, D, T>(deserializer: D) -> Result<Option<Vec<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let items: Vec<T> = list(deserializer)?;
        Ok((!items.is_empty()).then_some(items))
    }

    /// A value that parses, or nothing.
    pub fn maybe<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: DeserializeOwned,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(serde_json::from_value(value).ok())
    }

    /// An effect's parameter keys: a run per parameter name, each run the
    /// keys of it that parse, runs with none dropped.
    pub fn runs<'de, D>(deserializer: D) -> Result<BTreeMap<String, Vec<ParamKey>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(runs) = value else {
            return Ok(BTreeMap::new());
        };
        Ok(runs
            .into_iter()
            .filter_map(|(name, run)| {
                let keys: Vec<ParamKey> = match run {
                    Value::Array(items) => items
                        .into_iter()
                        .filter_map(|item| serde_json::from_value(item).ok())
                        .collect(),
                    _ => Vec::new(),
                };
                (!keys.is_empty()).then_some((name, keys))
            })
            .collect())
    }

    /// A clip kind by name, video for a name this build does not know.
    pub fn clip_kind<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ClipKind, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(serde_json::from_value(value).unwrap_or(ClipKind::Video))
    }

    /// A media kind by name, video for a name this build does not know.
    pub fn media_kind<'de, D: Deserializer<'de>>(deserializer: D) -> Result<MediaKind, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(serde_json::from_value(value).unwrap_or(MediaKind::Video))
    }

    /// A colour space by name, SDR for one this build does not know.
    pub fn color_space<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ColorSpace, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(serde_json::from_value(value).unwrap_or_default())
    }

    /// A text alignment by name, centred for one this build does not know.
    pub fn text_align<'de, D: Deserializer<'de>>(deserializer: D) -> Result<TextAlign, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(serde_json::from_value(value).unwrap_or(TextAlign::Center))
    }

    /// A background that parses, tidied, or black for one this build does
    /// not know - a kind from a newer build opens as no background rather
    /// than not at all.
    pub fn background<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Background, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(serde_json::from_value::<Background>(value)
            .map(Background::tidy)
            .unwrap_or_default())
    }

    /// A shape by name, a square for one this build does not know.
    pub fn shape_kind<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ShapeKind, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(serde_json::from_value(value).unwrap_or_default())
    }

    /// A mask's shape, or a circle for one this build does not know.
    pub fn mask_shape<'de, D: Deserializer<'de>>(deserializer: D) -> Result<MaskShape, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(serde_json::from_value(value).unwrap_or_default())
    }
}

/// The ranges a clip's numbers are held in. One set, used by every
/// command that sets a value and by the document reader through
/// [`Clip::tidy`], so a file and an edit can never disagree about what is
/// in range.
pub mod ranges {
    /// The shortest a clip can be: one frame at sixty.
    pub const MIN_CLIP_DURATION: f64 = 1.0 / 60.0;
    /// The furthest a clip may start, the longest it may run, and the
    /// deepest an in-point or a fade may reach, in seconds: a hundred
    /// hours. No cut is that long; a hand-edited `1e300` was, and its
    /// frame number overflowed the export's arithmetic.
    pub const MAX_TIME: f64 = 360_000.0;
    /// The engine's speed range (concat-media `SPEED_RANGE`), verbatim.
    pub const MIN_SPEED: f64 = 0.0625;
    /// The top of the engine's speed range.
    pub const MAX_SPEED: f64 = 16.0;
    /// The smallest a picture can be scaled to.
    pub const MIN_SCALE: f64 = 0.05;
    /// The largest a picture can be scaled to.
    pub const MAX_SCALE: f64 = 8.0;
    /// How far a picture may be pulled along one axis: a tenth to ten
    /// times its fitted extent, which covers every squash and every banner.
    pub const MIN_STRETCH: f64 = 0.1;
    /// The most a picture may be stretched along one axis.
    pub const MAX_STRETCH: f64 = 10.0;
    /// How far off centre a picture may be moved, in frame widths.
    pub const MAX_OFFSET: f64 = 3.0;
    /// A transition can be no shorter than this, in seconds.
    pub const MIN_TRANSITION: f64 = 0.1;

    /// A rotation kept in (-180, 180] so a full drag never accumulates
    /// turns.
    pub fn wrap_rotation(degrees: f64) -> f64 {
        let wrapped = ((degrees % 360.0) + 540.0) % 360.0 - 180.0;
        if wrapped == -180.0 { 180.0 } else { wrapped }
    }
}

/// One placed piece of a timeline: a stretch of media (or a title) with its
/// timing, mix, transform, and effects. Everything an edit decision touches
/// lives here, which is why most commands are clip commands.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
    #[serde(deserialize_with = "wire::clip_kind")]
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
    /// A multiplier on the fitted width beyond `scale`, for a picture
    /// pulled wider or narrower than its aspect; 1 keeps the aspect.
    #[serde(default = "unity", skip_serializing_if = "is_unity")]
    pub stretch_x: f64,
    /// The same for the height.
    #[serde(default = "unity", skip_serializing_if = "is_unity")]
    pub stretch_y: f64,
    /// Blend strength over whatever is beneath, in 0..1.
    pub opacity: f64,
    /// Playback rate. 1 is normal. With a curve set this is the curve's
    /// mean, kept in step by the commands that set either.
    pub speed: f64,
    /// Speed as it changes over the clip: points of `(at, speed)`, `at` a
    /// fraction of the clip's timeline length. None is the constant rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe_list")]
    pub speed_curve: Option<Vec<SpeedPoint>>,
    /// The user's own keys, sorted by property and then by `at`. Empty is a
    /// clip whose properties are the constants above. A keyed property's
    /// keys travel absolutely: they replace the constant rather than ride
    /// on it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[serde(deserialize_with = "wire::list")]
    pub keys: Vec<ClipKey>,
    /// Mirrored left to right.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_h: bool,
    /// Mirrored top to bottom.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_v: bool,
    /// How the picture's colour meets what is beneath it: "normal",
    /// "multiply", "screen", "add", "lighten", "darken".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub blend: String,
    /// What is cut off each edge of the source before it is fitted, as
    /// fractions of the source's width and height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub crop: Option<Crop>,
    /// The background taken away by a mask rather than a key colour; see
    /// [`Cutout`]. Keying by colour is a package on `video_effects`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub cutout: Option<Cutout>,
    /// The shape that keeps part of the picture; see [`ClipMask`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub mask: Option<ClipMask>,
    /// The camera's shake taken out; see [`Stabilize`]. None is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub stabilize: Option<Stabilize>,
    /// Keep voices at their natural pitch when `speed` is not 1. On by
    /// default; off gives the tape-machine chipmunk/slow-motion sound.
    pub preserve_pitch: bool,
    /// Audio filters, in order - order is audible. On a sound layer, the
    /// chain it runs over the mix.
    #[serde(default)]
    #[serde(deserialize_with = "wire::list")]
    pub filters: Vec<AppliedFilter>,
    /// Video effects, in order - the visual sibling of `filters`.
    #[serde(default)]
    #[serde(deserialize_with = "wire::list")]
    pub video_effects: Vec<AppliedFilter>,
    /// Which of the media's audio streams this clip plays, by the stream's
    /// index in the file - see [`MediaItem::audio_tracks`]. None is the first
    /// in file order, which is every clip from before files with several
    /// were told apart and every clip of a file with one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_stream: Option<u32>,
    /// True when a video clip's embedded audio is detached out of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    /// On a detached audio clip: the video clip the sound came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached_from: Option<String>,
    /// The transition on the cut into this clip, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub transition_in: Option<Transition>,
    /// The clip's Enhance: the media it was cut from and the enhanced copy
    /// of it, and which of the two it shows. Absent until Enhance has made
    /// a copy; switching it off shows the original again without running
    /// the model a second time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub enhanced: Option<Enhanced>,
    /// The overlay, when this is a text clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub text: Option<TextStyle>,
    /// The figure, when this is a shape clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub shape: Option<ShapeStyle>,
    /// What fills the frame behind this clip's picture, over the
    /// timeline's [`Timeline::background`]: None follows the timeline's,
    /// and `Some(Background::None)` is black under a timeline that has one.
    /// Only a picture on the lowest picture lane has a background at all;
    /// see [`Timeline::background_of`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub background: Option<Background>,
    /// Fields this build does not know, kept so a document written by a
    /// newer or a different build round-trips through this one intact.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// One property's keys, as `(at, value, ease)` over `0..=1` of an old
/// length, re-anchored to the window `(a, b)` of that length; see
/// [`Clip::rewindow_keys`]. `value_at` is the ride over the old length.
fn rewindow(
    keys: impl Iterator<Item = (f64, f64, KeyEase)>,
    (a, b): (f64, f64),
    value_at: impl Fn(f64) -> f64,
) -> Vec<(f64, f64, KeyEase)> {
    let keys: Vec<(f64, f64, KeyEase)> = keys.collect();
    let span = b - a;
    let mut out = Vec::with_capacity(keys.len() + 2);
    let before = keys.iter().any(|(at, ..)| *at < a - KEY_EPSILON);
    if before {
        out.push((0.0, value_at(a), KeyEase::LINEAR));
    }
    let mut first_after: Option<KeyEase> = None;
    for (at, value, ease) in &keys {
        if *at < a - KEY_EPSILON {
            continue;
        }
        if *at > b + KEY_EPSILON {
            first_after.get_or_insert(*ease);
            continue;
        }
        out.push((((at - a) / span).clamp(0.0, 1.0), *value, *ease));
    }
    if let Some(ease) = first_after {
        out.push((1.0, value_at(b), ease));
    }
    out
}

impl Default for Clip {
    /// What a document entry starts from before its own fields land on
    /// it: [`Clip::blank`] with no id, on no track, one second of video.
    fn default() -> Self {
        Clip::blank("", "", ClipKind::Video, "clip", 0.0, 1.0)
    }
}

impl Clip {
    /// A clip with nothing set but what names and places it: unity gain,
    /// scale and speed, no fades, no effects, no keys. Every constructor
    /// starts here and sets the few fields its kind needs, so a field added
    /// to the model is added in one place.
    pub fn blank(
        id: impl Into<String>,
        track_id: impl Into<String>,
        kind: ClipKind,
        name: impl Into<String>,
        start: f64,
        duration: f64,
    ) -> Clip {
        Clip {
            id: id.into(),
            track_id: track_id.into(),
            media_id: String::new(),
            name: name.into(),
            kind,
            start: start.max(0.0),
            duration: duration.max(ranges::MIN_CLIP_DURATION),
            source_start: 0.0,
            volume: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            rotation: 0.0,
            stretch_x: 1.0,
            stretch_y: 1.0,
            opacity: 1.0,
            speed: 1.0,
            preserve_pitch: true,
            speed_curve: None,
            keys: Vec::new(),
            flip_h: false,
            flip_v: false,
            blend: String::new(),
            crop: None,
            cutout: None,
            mask: None,
            stabilize: None,
            filters: Vec::new(),
            video_effects: Vec::new(),
            audio_stream: None,
            muted: None,
            detached_from: None,
            transition_in: None,
            enhanced: None,
            text: None,
            shape: None,
            background: None,
            extra: Map::new(),
        }
    }

    /// Pulls every number back into the range a command would have held
    /// it to: the floors and ceilings in [`ranges`], the rotation wrapped,
    /// non-finite values replaced by their neutral, the crop and the cutout
    /// tidied, the keys sorted. What the document reader runs over a clip
    /// it has just read, so a hand-edited or older file holds nothing an
    /// edit could not have produced.
    pub fn tidy(mut self) -> Clip {
        use ranges::*;
        fn finite(value: f64, fallback: f64) -> f64 {
            if value.is_finite() { value } else { fallback }
        }
        self.start = finite(self.start, 0.0).clamp(0.0, MAX_TIME);
        self.duration = finite(self.duration, 1.0).clamp(MIN_CLIP_DURATION, MAX_TIME);
        self.source_start = finite(self.source_start, 0.0).clamp(0.0, MAX_TIME);
        self.volume = finite(self.volume, 1.0).max(0.0);
        self.fade_in = finite(self.fade_in, 0.0).clamp(0.0, MAX_TIME);
        self.fade_out = finite(self.fade_out, 0.0).clamp(0.0, MAX_TIME);
        self.scale = finite(self.scale, 1.0).clamp(MIN_SCALE, MAX_SCALE);
        self.offset_x = finite(self.offset_x, 0.0).clamp(-MAX_OFFSET, MAX_OFFSET);
        self.offset_y = finite(self.offset_y, 0.0).clamp(-MAX_OFFSET, MAX_OFFSET);
        self.rotation = wrap_rotation(finite(self.rotation, 0.0));
        self.stretch_x = finite(self.stretch_x, 1.0).clamp(MIN_STRETCH, MAX_STRETCH);
        self.stretch_y = finite(self.stretch_y, 1.0).clamp(MIN_STRETCH, MAX_STRETCH);
        self.opacity = finite(self.opacity, 1.0).clamp(0.0, 1.0);
        self.speed = finite(self.speed, 1.0).clamp(MIN_SPEED, MAX_SPEED);
        self.speed_curve = self.speed_curve.take().and_then(|points| {
            let points: Vec<SpeedPoint> = points
                .into_iter()
                .filter(|point| point.at.is_finite() && (0.0..=1.0).contains(&point.at))
                .map(|point| SpeedPoint {
                    at: point.at,
                    speed: finite(point.speed, 1.0).clamp(MIN_SPEED, MAX_SPEED),
                    slope: point.slope.filter(|slope| slope.is_finite()),
                })
                .collect();
            (!points.is_empty()).then_some(points)
        });
        self.crop = self
            .crop
            .take()
            .map(Crop::tidy)
            .filter(|crop| !crop.is_none());
        self.cutout = self.cutout.take().map(Cutout::tidy);
        self.mask = self.mask.take().map(ClipMask::tidy);
        self.stabilize = self.stabilize.take().map(Stabilize::tidy);
        if let Some(transition) = self.transition_in.as_mut() {
            transition.duration = finite(transition.duration, 1.0).max(MIN_TRANSITION);
            // A knob that is not a number would save as null and make the
            // project unreadable: it goes, and the knob is at its default.
            transition.params.retain(|_, value| value.is_finite());
        }
        self.keys
            .retain(|key| (0.0..=1.0).contains(&key.at) && key.value.is_finite());
        // A key holds what the field itself may hold. Rotation is the one
        // exception: a key past a full turn is a spin, and wrapping it
        // would take the spin away.
        for key in &mut self.keys {
            key.value = match key.property {
                KeyProperty::Scale => key.value.clamp(MIN_SCALE, MAX_SCALE),
                KeyProperty::Opacity => key.value.clamp(0.0, 1.0),
                KeyProperty::Volume => key.value.max(0.0),
                KeyProperty::OffsetX | KeyProperty::OffsetY => {
                    key.value.clamp(-MAX_OFFSET, MAX_OFFSET)
                }
                KeyProperty::Rotation => key.value,
                mask => ClipMask::clamp(mask, key.value),
            };
        }
        self.sort_keys();
        for chain in [&mut self.filters, &mut self.video_effects] {
            for entry in chain.iter_mut() {
                entry.sort_keys();
            }
        }
        self.text = self.text.take().map(TextStyle::tidy);
        self.shape = self.shape.take().map(ShapeStyle::tidy);
        self.background = self.background.take().map(Background::tidy);
        if self.muted == Some(false) {
            self.muted = None;
        }
        self
    }

    /// The clip's own constant for a keyable property - what the property is
    /// worth everywhere its track is silent.
    pub fn constant(&self, property: KeyProperty) -> f64 {
        match property {
            KeyProperty::Scale => self.scale,
            KeyProperty::OffsetX => self.offset_x,
            KeyProperty::OffsetY => self.offset_y,
            KeyProperty::Rotation => self.rotation,
            KeyProperty::Opacity => self.opacity,
            KeyProperty::Volume => self.volume,
            // A clip with no mask answers with a new one's numbers, so a
            // panel opening on it shows what the first shape will start at.
            mask => self.mask.unwrap_or_default().get(mask).unwrap_or_default(),
        }
    }

    /// The scale at which this clip's picture, from media `width` by
    /// `height` and through its crop, fits in or covers a `frame`, turned as
    /// it is at `at` (`0..=1` of the clip); see [`framing_scale`]. None for a
    /// clip that is not a picture, or a size nobody knows.
    pub fn framing_scale(
        &self,
        framing: Framing,
        media: (Option<u32>, Option<u32>),
        frame: (u32, u32),
        at: f64,
    ) -> Option<f64> {
        if !self.kind.is_visual() {
            return None;
        }
        let (width, height) = (f64::from(media.0?), f64::from(media.1?));
        let crop = self.crop.unwrap_or_default().tidy();
        let picture = (
            width * (1.0 - crop.left - crop.right),
            height * (1.0 - crop.top - crop.bottom),
        );
        let rotation = if self.is_keyed(KeyProperty::Rotation) {
            self.value_at(KeyProperty::Rotation, at)
        } else {
            self.rotation
        };
        framing_scale(
            framing,
            picture,
            (f64::from(frame.0), f64::from(frame.1)),
            rotation,
        )
    }

    /// Writes a keyable property the way an edit means it: the constant
    /// while the property has no keys, and once it rides, a key at `at`
    /// (`0..=1` of the clip), with the ease of the key behind it. A keyed
    /// property no longer reads its constant, so writing that would change
    /// nothing anyone could see.
    pub fn write_at(&mut self, property: KeyProperty, value: f64, at: f64) {
        if self.is_keyed(property) {
            let ease = self
                .keys_on(property)
                .rfind(|key| key.at < at)
                .map_or(KeyEase::LINEAR, |key| key.ease);
            self.set_key(property, at, value, ease);
            return;
        }
        match property {
            KeyProperty::Scale => self.scale = value,
            KeyProperty::OffsetX => self.offset_x = value,
            KeyProperty::OffsetY => self.offset_y = value,
            KeyProperty::Rotation => self.rotation = value,
            KeyProperty::Opacity => self.opacity = value,
            KeyProperty::Volume => self.volume = value,
            // The mask's numbers are the mask's; a number written on a
            // clip with none puts one on, the shape a new one starts as.
            mask => self
                .mask
                .get_or_insert_with(ClipMask::default)
                .set(mask, value),
        }
    }

    /// This property's keys, in order. Borrowed rather than collected: the
    /// caller is usually asking a question about them, not keeping them.
    pub fn keys_on(
        &self,
        property: KeyProperty,
    ) -> impl DoubleEndedIterator<Item = &ClipKey> + Clone {
        self.keys.iter().filter(move |key| key.property == property)
    }

    /// Whether this property is keyed at all. A property with keys is one
    /// the panel shows as animated, however few there are.
    pub fn is_keyed(&self, property: KeyProperty) -> bool {
        self.keys_on(property).next().is_some()
    }

    /// The index into `keys` of this property's key at `at`, within
    /// `KEY_EPSILON`, choosing the nearest when two are in reach.
    pub fn key_at(&self, property: KeyProperty, at: f64) -> Option<usize> {
        self.keys
            .iter()
            .enumerate()
            .filter(|(_, key)| key.property == property && (key.at - at).abs() <= KEY_EPSILON)
            .min_by(|(_, a), (_, b)| (a.at - at).abs().total_cmp(&(b.at - at).abs()))
            .map(|(index, _)| index)
    }

    /// This property's keys as the engine plays them.
    pub fn track_on(&self, property: KeyProperty) -> concat_core::animate::Track {
        concat_core::animate::Track::new(
            self.keys_on(property)
                .map(|key| concat_core::animate::Key {
                    at: key.at,
                    value: key.value,
                    ease: key.ease.into(),
                })
                .collect(),
        )
    }

    /// What a property is worth at `at`: its ride where it is keyed, and its
    /// constant where it is not.
    ///
    /// The same arithmetic the engine does, which is what makes a key put on
    /// where the ride already is change nothing on screen - and what lets a
    /// panel show the live value under the playhead without asking the
    /// renderer.
    pub fn value_at(&self, property: KeyProperty, at: f64) -> f64 {
        let constant = self.constant(property);
        if !self.is_keyed(property) {
            return constant;
        }
        self.track_on(property).value_at(at, constant)
    }

    /// The nearest key strictly before `at`, and the nearest strictly after.
    /// What the panel's two chevrons move the playhead to; `None` on a side
    /// is that chevron greyed out.
    pub fn keys_around(&self, property: KeyProperty, at: f64) -> (Option<f64>, Option<f64>) {
        let mut before: Option<f64> = None;
        let mut after: Option<f64> = None;
        for key in self.keys_on(property) {
            if key.at < at - KEY_EPSILON {
                before = Some(before.map_or(key.at, |seen: f64| seen.max(key.at)));
            } else if key.at > at + KEY_EPSILON {
                after = Some(after.map_or(key.at, |seen: f64| seen.min(key.at)));
            }
        }
        (before, after)
    }

    /// Sets a key, replacing whichever key on that property was already
    /// within `KEY_EPSILON` of `at`. Keeps `keys` sorted by property and
    /// then by `at`, which is what lets everything else read them in order
    /// without sorting first.
    pub fn set_key(&mut self, property: KeyProperty, at: f64, value: f64, ease: KeyEase) {
        if !at.is_finite() || !value.is_finite() {
            return;
        }
        let at = at.clamp(0.0, 1.0);
        match self.key_at(property, at) {
            Some(index) => {
                self.keys[index] = ClipKey {
                    property,
                    at,
                    value,
                    ease,
                }
            }
            None => self.keys.push(ClipKey {
                property,
                at,
                value,
                ease,
            }),
        }
        self.sort_keys();
    }

    /// Removes this property's key at `at`, if there is one. True when a key
    /// actually went.
    pub fn clear_key(&mut self, property: KeyProperty, at: f64) -> bool {
        match self.key_at(property, at) {
            Some(index) => {
                self.keys.remove(index);
                true
            }
            None => false,
        }
    }

    /// Takes every key off one property. True when there were any.
    pub fn clear_keys(&mut self, property: KeyProperty) -> bool {
        let before = self.keys.len();
        self.keys.retain(|key| key.property != property);
        self.keys.len() != before
    }

    /// Re-anchors every key - the clip's own and its effects' - after the
    /// clip's span changed, so each key stays at the instant of the picture
    /// it was set on.
    ///
    /// Keys are stored as fractions of the clip's length, which is the
    /// right unit for a preset and the wrong one for an edit: a split, a
    /// trim or a merge changes the length under them, and without this a
    /// fade over the first half of a clip becomes a fade over the first
    /// half of each piece. `old` is the length the keys were set against;
    /// `[from, to]` is the window of that length the clip now covers, in
    /// seconds from its old start. A window past either end of the old
    /// clip (a trim that extends it, a merge) simply spreads the keys over
    /// the longer span, where the ride holds its end values anyway.
    ///
    /// A key outside the window is replaced by one on the window's edge
    /// carrying the ride's value there, so the piece plays exactly what
    /// the whole played over that stretch, and a merge of the pieces gets
    /// the ride back.
    pub fn rewindow_keys(&mut self, old: f64, from: f64, to: f64) {
        let sane = old.is_finite() && old > 0.0 && to.is_finite() && to > from;
        if !sane {
            return;
        }
        let (a, b) = (from / old, to / old);
        for property in KeyProperty::ALL {
            if !self.is_keyed(property) {
                continue;
            }
            let ride = self.track_on(property);
            let constant = self.constant(property);
            let keys: Vec<ClipKey> = self.keys_on(property).copied().collect();
            let windowed = rewindow(
                keys.iter().map(|key| (key.at, key.value, key.ease)),
                (a, b),
                |x| ride.value_at(x, constant),
            );
            self.keys.retain(|key| key.property != property);
            self.keys
                .extend(windowed.into_iter().map(|(at, value, ease)| ClipKey {
                    property,
                    at,
                    value,
                    ease,
                }));
        }
        for link in self.filters.iter_mut().chain(self.video_effects.iter_mut()) {
            let names: Vec<String> = link.keys.keys().cloned().collect();
            for name in names {
                let ride = link.track_on(&name);
                let constant = link.params.get(&name).copied().unwrap_or(0.0);
                let keys: Vec<ParamKey> = link.keys_on(&name).to_vec();
                let windowed = rewindow(
                    keys.iter().map(|key| (key.at, key.value, key.ease)),
                    (a, b),
                    |x| ride.value_at(x, constant),
                );
                link.keys.insert(
                    name,
                    windowed
                        .into_iter()
                        .map(|(at, value, ease)| ParamKey { at, value, ease })
                        .collect(),
                );
            }
            link.sort_keys();
        }
        self.sort_keys();
    }

    /// Takes on the keys of `piece`, a clip that sat `offset` seconds
    /// after this one's start and has been merged into it: each of its
    /// keys lands at the same instant of the picture it marked, now
    /// measured over this clip's `duration`. Effect keys come across where
    /// the effect at the same position of the chain is the same effect.
    pub fn absorb_keys(&mut self, piece: &Clip, offset: f64) {
        let sane = self.duration.is_finite() && self.duration > 0.0;
        if !sane {
            return;
        }
        for key in &piece.keys {
            let at = (offset + key.at * piece.duration) / self.duration;
            self.set_key(key.property, at, key.value, key.ease);
        }
        for (mine, theirs) in self.filters.iter_mut().zip(piece.filters.iter()).chain(
            self.video_effects
                .iter_mut()
                .zip(piece.video_effects.iter()),
        ) {
            if mine.id != theirs.id {
                continue;
            }
            for (name, run) in &theirs.keys {
                for key in run {
                    let at = (offset + key.at * piece.duration) / self.duration;
                    mine.set_key(name, at, key.value, key.ease);
                }
            }
        }
    }

    /// Drops keys that are not finite or not in `0..=1`, then orders them.
    /// Called by everything that can put a key in, including the document
    /// reader, so a hand-edited file cannot produce an unsorted track.
    pub fn sort_keys(&mut self) {
        self.keys.retain(|key| {
            key.at.is_finite() && key.value.is_finite() && (0.0..=1.0).contains(&key.at)
        });
        self.keys.sort_by(|a, b| {
            (a.property as u8)
                .cmp(&(b.property as u8))
                .then_with(|| a.at.total_cmp(&b.at))
        });
    }
}

/// One timeline: a name and its lanes and clips. Every operation takes the
/// project and works on whichever timeline is active.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Timeline {
    /// Minted by the editor ("tl2", ...), except the founding "TL1".
    pub id: String,
    /// The tab label; "Timeline N" by default, renameable but never blank.
    pub name: String,
    /// The frame this timeline renders to and the rate it runs at. Each
    /// timeline's own: a vertical cut for one platform and a wide one for
    /// another are different frames of the same media, and that is most of
    /// what a second timeline is for.
    #[serde(default)]
    pub video: VideoSettings,
    /// What fills the frame behind the pictures on the lowest picture lane,
    /// where they do not reach it; see [`Timeline::background_of`]. The
    /// timeline's rather than [`VideoSettings`]' because that is the four
    /// numbers a frame is, copied and compared everywhere, and this names
    /// media. Black, and left out of the document, until one is chosen.
    #[serde(default, skip_serializing_if = "Background::is_none")]
    #[serde(deserialize_with = "wire::background")]
    pub background: Background,
    /// The lanes, top to bottom. Never empty - `RemoveTrack` keeps a floor
    /// of one.
    #[serde(deserialize_with = "wire::list")]
    pub tracks: Vec<Track>,
    /// Every clip on this timeline, in insertion order, not time order -
    /// readers must sort by `start` where order matters.
    ///
    /// Behind `Arc` so that cloning the timeline - which undo does before
    /// every command - copies one pointer per clip, and only the clip a
    /// command then writes to is copied for real ([`Arc::make_mut`] in
    /// [`Timeline::clip_mut`]). Reading through the `Arc` is transparent.
    #[serde(deserialize_with = "wire::list")]
    pub clips: Vec<Arc<Clip>>,
    /// Linkage: whether the other lanes follow an edit that closes up or
    /// opens the main lane ([`Timeline::background_lane`]), so a caption,
    /// a title, an overlay or a sound placed against the footage stays on
    /// the frames it was placed against. CapCut's Linkage, Final Cut's
    /// connected clips. On unless turned off, and left out of the document
    /// while on.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub linkage: bool,
    /// Fields this build does not know, kept so they round-trip.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Default for Timeline {
    fn default() -> Self {
        Timeline {
            id: String::new(),
            name: "Timeline".to_owned(),
            video: VideoSettings::default(),
            background: Background::None,
            tracks: Vec::new(),
            clips: Vec::new(),
            linkage: true,
            extra: Map::new(),
        }
    }
}

/// The colour a picture is in: SDR on Rec. 709, or HDR on Rec. 2020 with
/// the hybrid log-gamma or the perceptual quantiser transfer. A timeline's
/// is what it is output in; a media file's, what it was recorded in.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorSpace {
    /// Rec. 709, SDR: every timeline until its first HDR clip.
    #[default]
    Sdr,
    /// Rec. 2020 with HLG: what an iPhone records, and what a timeline
    /// becomes on its first HDR clip.
    Hlg,
    /// Rec. 2020 with PQ: HDR10.
    Pq,
}

impl ColorSpace {
    /// Standard dynamic range: what a document that says nothing is in.
    pub fn is_sdr(&self) -> bool {
        *self == ColorSpace::Sdr
    }

    /// High dynamic range, HLG or PQ.
    pub fn is_hdr(&self) -> bool {
        !self.is_sdr()
    }
}

/// A timeline's output frame, rate and colour.
///
/// The same four numbers the document's top-level `video` block has always
/// carried, now one set per timeline, and the colour it is output in. The
/// top-level block is still written, as the active timeline's frame and
/// rate, so a build that predates this reads the document it always did,
/// and a document from such a build gives every timeline that block on the
/// way in.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSettings {
    /// Output frame width in pixels.
    pub width: u32,
    /// Output frame height in pixels.
    pub height: u32,
    /// Numerator of the frame rate, e.g. 30000 for 29.97 fps.
    pub rate_num: i64,
    /// Denominator of the frame rate, e.g. 1001 for 29.97 fps.
    pub rate_den: i64,
    /// The colour the timeline is output in: SDR for every timeline from
    /// before HDR, and until its first HDR clip (see
    /// `Editor::follow_first_hdr`). Left out of the document for SDR, so
    /// those documents stay as they were.
    #[serde(default, skip_serializing_if = "ColorSpace::is_sdr")]
    #[serde(deserialize_with = "wire::color_space")]
    pub color_space: ColorSpace,
    /// What shows where no picture covers the frame - a gap, the space
    /// around a title, a clip fading in - as straight `[r, g, b, a]`,
    /// written `#rrggbb` in the document, or `#rrggbbaa` when it lets
    /// something show through: opaque black unless the project was given
    /// another (#456). Alpha zero is a transparent frame, for an overlay
    /// exported in a format that keeps alpha; a format that does not
    /// gets black there. Left out of the document while it is opaque
    /// black, so those documents stay as they were.
    #[serde(default = "opaque_black", skip_serializing_if = "is_black", with = "hex_rgba")]
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub canvas: [u8; 4],
}

/// The canvas every project had before it could be given one.
pub const OPAQUE_BLACK: [u8; 4] = [0, 0, 0, 255];

/// A canvas written as a bare array of channels, as the recents list and
/// the export and preview requests carry it: `[r, g, b, a]`, or `[r, g, b]`
/// as it was written before the canvas had an alpha, which is opaque. Any
/// other shape is opaque black, so one odd entry does not lose the rest of
/// what it is read with.
pub fn rgba_array<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<[u8; 4], D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let channels: Option<Vec<u8>> = value.as_array().and_then(|items| {
        items
            .iter()
            .map(|item| item.as_u64().and_then(|n| u8::try_from(n).ok()))
            .collect()
    });
    Ok(match channels.as_deref() {
        Some(&[r, g, b]) => [r, g, b, 255],
        Some(&[r, g, b, a]) => [r, g, b, a],
        _ => OPAQUE_BLACK,
    })
}

fn opaque_black() -> [u8; 4] {
    OPAQUE_BLACK
}

fn is_black(rgba: &[u8; 4]) -> bool {
    *rgba == OPAQUE_BLACK
}

/// A colour kept as four straight channels and written the way the
/// document writes every colour: `#rrggbb` while it is opaque,
/// `#rrggbbaa` when it is not. One that does not read is opaque black.
pub mod hex_rgba {
    use serde::{Deserialize, Deserializer, Serializer};

    /// `#rrggbb`, or `#rrggbbaa` short of opaque; lower case.
    pub fn serialize<S: Serializer>(rgba: &[u8; 4], serializer: S) -> Result<S::Ok, S::Error> {
        let [r, g, b, a] = *rgba;
        serializer.serialize_str(&if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        })
    }

    /// `#rrggbb` (opaque) or `#rrggbbaa`; opaque black for anything else.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 4], D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value
            .as_str()
            .and_then(super::rgba_of)
            .unwrap_or(super::OPAQUE_BLACK))
    }
}

impl Default for VideoSettings {
    /// 1080p at 30 in SDR, the frame a fresh project has always been born
    /// with.
    fn default() -> Self {
        VideoSettings {
            width: 1920,
            height: 1080,
            rate_num: 30,
            rate_den: 1,
            color_space: ColorSpace::Sdr,
            canvas: OPAQUE_BLACK,
        }
    }
}

impl VideoSettings {
    /// The rate as a number, for anything that draws or counts frames.
    pub fn rate(self) -> f64 {
        self.rate_num as f64 / self.rate_den.max(1) as f64
    }

    /// This frame with every zero, negative or oversize field taken from
    /// `fallback` instead, and the rate as a pair when it is not one a video
    /// has (see [`FrameRate::checked`](concat_core::time::FrameRate::checked)):
    /// a document hand-edited into nonsense still opens at a size that is a
    /// size, and at a rate the frame arithmetic can count in.
    pub fn or(self, fallback: VideoSettings) -> VideoSettings {
        let settings = self.or_terms(fallback);
        if concat_core::time::FrameRate::checked(settings.rate_num, settings.rate_den).is_some() {
            settings
        } else {
            VideoSettings {
                rate_num: fallback.rate_num,
                rate_den: fallback.rate_den,
                ..settings
            }
        }
    }

    fn or_terms(self, fallback: VideoSettings) -> VideoSettings {
        VideoSettings {
            width: if Self::side_is_sane(self.width) {
                self.width
            } else {
                fallback.width
            },
            height: if Self::side_is_sane(self.height) {
                self.height
            } else {
                fallback.height
            },
            rate_num: if self.rate_num > 0 {
                self.rate_num
            } else {
                fallback.rate_num
            },
            rate_den: if self.rate_den > 0 {
                self.rate_den
            } else {
                fallback.rate_den
            },
            color_space: self.color_space,
            canvas: self.canvas,
        }
    }

    /// Whether every term is one a frame could actually have. A zero
    /// dimension or rate is never a real setting, only a caller bug, and
    /// writing one would poison the document until the next open. Neither
    /// is a rate whose terms overflow the frame arithmetic, nor a side past
    /// what a GPU can hold in one texture
    /// ([`MAX_SIDE`](concat_core::frame::MAX_SIDE)): a 10 000-pixel
    /// timeline panicked the export and the preview at the first texture.
    pub fn is_sane(self) -> bool {
        Self::side_is_sane(self.width)
            && Self::side_is_sane(self.height)
            && concat_core::time::FrameRate::checked(self.rate_num, self.rate_den).is_some()
    }

    fn side_is_sane(side: u32) -> bool {
        (1..=concat_core::frame::MAX_SIDE).contains(&side)
    }
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
    #[serde(default, deserialize_with = "wire::list")]
    pub media: Vec<MediaItem>,
    /// Fonts the user added from disk, available to every title.
    #[serde(default, deserialize_with = "wire::list")]
    pub fonts: Vec<CustomFont>,
    /// Every timeline, in tab order. Always at least one. Behind `Arc` for
    /// the reason [`Timeline::clips`] is: a command on one timeline leaves
    /// the others shared with the undo snapshot.
    #[serde(default, deserialize_with = "wire::list")]
    pub timelines: Vec<Arc<Timeline>>,
    /// Which timeline commands act on. Maintained by the command layer, so
    /// it always names a member of `timelines`; [`Project::active`] degrades
    /// to the first timeline if it somehow does not.
    #[serde(default)]
    pub active_timeline_id: String,
    /// Top-level fields this build does not know, kept so they round-trip.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

/// A media reference that points to a non-existent file.
#[derive(Clone, Debug)]
pub struct MissingMedia {
    /// The media item's stable id (e.g. "m1", "m2").
    pub id: String,
    /// Display name in the bin.
    pub name: String,
    /// The absolute path that no longer exists on disk.
    pub path: String,
}

impl Project {
    /// A new project: one timeline, four lanes, at the default frame.
    pub fn new() -> Self {
        Self::with_video(VideoSettings::default())
    }

    /// A new project whose one timeline renders to `video` - what a project
    /// created from the launch screen's size and rate pickers starts as.
    pub fn with_video(video: VideoSettings) -> Self {
        Self {
            media: Vec::new(),
            fonts: Vec::new(),
            extra: Map::new(),
            timelines: vec![Arc::new(Timeline {
                id: "TL1".to_owned(),
                name: "Timeline 1".to_owned(),
                video,
                background: Background::None,
                tracks: (1..=4)
                    .map(|number| Track {
                        id: format!("T{number}"),
                        ..Track::default()
                    })
                    .collect(),
                clips: Vec::new(),
                linkage: true,
                extra: Map::new(),
            })],
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
        Arc::make_mut(&mut self.timelines[index])
    }

    /// The bin entry with this id, or None if it was removed.
    pub fn media_by_id(&self, media_id: &str) -> Option<&MediaItem> {
        self.media.iter().find(|item| item.id == media_id)
    }

    /// Returns every media item whose file does not exist on disk.
    pub fn missing_media(&self) -> Vec<MissingMedia> {
        self.media
            .iter()
            .filter(|item| !std::path::Path::new(&item.path).exists())
            .map(|item| MissingMedia {
                id: item.id.clone(),
                name: item.name.clone(),
                path: item.path.clone(),
            })
            .collect()
    }
}

impl MediaItem {
    /// A nameless entry is called by its path.
    pub fn tidy(mut self) -> MediaItem {
        if self.name.is_empty() {
            self.name = self.path.clone();
        }
        self
    }

    /// Which row of `audio_tracks` a clip's `audio_stream` is: the named
    /// stream's position, or the first row for a clip that names none or
    /// names a stream this file does not have - the same fallback the
    /// engine's readers make.
    pub fn audio_track_position(&self, stream: Option<u32>) -> usize {
        stream
            .and_then(|index| {
                self.audio_tracks
                    .iter()
                    .position(|track| track.index == index)
            })
            .unwrap_or(0)
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
        self.clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .map(Arc::as_ref)
    }

    /// Mutable twin of [`Timeline::clip`]. The clip is copied out of any
    /// undo snapshot still sharing it before the reference is handed back.
    pub fn clip_mut(&mut self, clip_id: &str) -> Option<&mut Clip> {
        self.clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
            .map(Arc::make_mut)
    }

    /// Mutable access to the clip at `index`, copied out of any snapshot
    /// sharing it first. The `Vec` index twin of [`Timeline::clip_mut`].
    pub fn clip_at_mut(&mut self, index: usize) -> &mut Clip {
        Arc::make_mut(&mut self.clips[index])
    }

    /// The clips `keep` picks, mutable, each copied out of any snapshot
    /// sharing it only once picked: a ripple that moves ten clips of five
    /// thousand copies ten, and the undo step stays the size of the edit
    /// (audit 2026-09-23, #8).
    pub fn clips_where(
        &mut self,
        mut keep: impl FnMut(&Clip) -> bool,
    ) -> impl Iterator<Item = &mut Clip> {
        self.clips
            .iter_mut()
            .filter(move |clip| keep(clip))
            .map(Arc::make_mut)
    }

    /// The track with this id, or None if it was removed.
    pub fn track(&self, track_id: &str) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == track_id)
    }

    /// The lane whose pictures have a background: the lowest lane holding a
    /// video or a still - the main picture of the cut, where CapCut has its
    /// main track. Lowest rather than simply the first lane, so a cut whose
    /// bottom lane holds only music still has one. None for a timeline
    /// without a picture.
    pub fn background_lane(&self) -> Option<&str> {
        self.tracks
            .iter()
            .find(|track| {
                self.clips
                    .iter()
                    .any(|clip| clip.track_id == track.id && clip.kind.is_visual())
            })
            .map(|track| track.id.as_str())
    }

    /// What fills the frame behind `clip`: its own background, else the
    /// timeline's, for a video or a still on [`Timeline::background_lane`];
    /// None for any other clip, and for one whose background is black.
    /// Titles, shapes and pictures on the lanes above sit over the frame the
    /// main picture and its background have already filled, so they never
    /// have one of their own.
    pub fn background_of<'a>(&'a self, clip: &'a Clip) -> Option<&'a Background> {
        self.background_on(clip, self.background_lane())
    }

    /// [`Timeline::background_of`] with the lane found once, by
    /// [`Timeline::background_lane`], for a caller asking of every clip.
    pub fn background_on<'a>(
        &'a self,
        clip: &'a Clip,
        lane: Option<&str>,
    ) -> Option<&'a Background> {
        if !clip.kind.is_visual() || lane != Some(clip.track_id.as_str()) {
            return None;
        }
        let background = clip.background.as_ref().unwrap_or(&self.background);
        (!background.is_none()).then_some(background)
    }

    /// Where the last clip ends. Zero for an empty timeline.
    pub fn duration(&self) -> f64 {
        self.clips
            .iter()
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max)
    }
}
