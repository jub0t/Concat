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

fn unity() -> f64 {
    1.0
}

fn is_unity(value: &f64) -> bool {
    *value == 1.0
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

/// How a picture's background is taken away when there is no key colour
/// to remove: a person mask found by the cutout model, alone or corrected
/// by hand.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cutout {
    /// Automatic keeps what the model finds; custom adds the strokes.
    pub mode: CutoutMode,
    /// How far the edge is softened, as a fraction of the picture's width.
    #[serde(default = "default_feather")]
    pub feather: f64,
    /// Corrections painted on the monitor, in the order they were made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<Stroke>,
}

/// The two ways a cutout decides what stays.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CutoutMode {
    /// The model's mask as it is.
    Auto,
    /// The model's mask, then the strokes over it.
    Custom,
}

/// One brush stroke over a cutout: which tool, how wide, and where it went.
/// Points are fractions of the source picture, `(0, 0)` its top-left, so a
/// stroke survives a change of output size, a crop or a flip.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    /// What the stroke does to the mask beneath it.
    pub tool: BrushTool,
    /// The brush's diameter as a fraction of the picture's width.
    pub size: f64,
    /// The path, as `[x, y]` fractions.
    pub points: Vec<[f64; 2]>,
}

/// The four brushes of a custom cutout.
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
    /// The size held to its range and the points to the picture; `None`
    /// for a stroke with no points left.
    pub fn tidy(mut self) -> Option<Stroke> {
        self.size = if self.size.is_finite() {
            self.size.clamp(MIN_BRUSH, MAX_BRUSH)
        } else {
            MIN_BRUSH
        };
        self.points.retain(|[x, y]| x.is_finite() && y.is_finite());
        for point in &mut self.points {
            point[0] = point[0].clamp(-0.5, 1.5);
            point[1] = point[1].clamp(-0.5, 1.5);
        }
        (!self.points.is_empty()).then_some(self)
    }
}

/// A crop: what is taken off each edge, as fractions of the source.
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

/// Which end of a clip an animation belongs to, or the whole of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnimationSlot {
    /// The first seconds.
    In,
    /// The last seconds.
    Out,
    /// The whole clip.
    Combo,
}

/// A named animation on one slot. The keys are made from the name for the
/// clip's current length whenever they are needed; see the `animation`
/// module.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipAnimation {
    /// The shape's name, e.g. "Fade".
    pub preset: String,
    /// Seconds the shape takes, for In and Out; ignored by a Combo.
    pub duration: f64,
}

/// Which property a user-set key belongs to.
///
/// Five of the six are the picture's; the sixth is the mix's. They are one
/// enum because a key is a key - the panel that sets them, the commands that
/// carry them and the document that stores them do not care which side of
/// the clip a value ends up on, and only the thing that finally reads them
/// does.
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
            _ => return None,
        })
    }

    /// Every property that can be keyed, in the order a panel lists them.
    pub const ALL: [KeyProperty; 6] = [
        KeyProperty::Scale,
        KeyProperty::OffsetX,
        KeyProperty::OffsetY,
        KeyProperty::Rotation,
        KeyProperty::Opacity,
        KeyProperty::Volume,
    ];
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
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyEase(pub [f64; 4]);

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
    /// so a hand-edited document cannot hand the solver an x outside `0..=1`
    /// - where a cubic bezier stops being a function of x and the Newton
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
    pub speed_curve: Option<Vec<SpeedPoint>>,
    /// Played backwards.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
    /// How the clip comes in: a named shape over its first seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation_in: Option<ClipAnimation>,
    /// How it goes out: a named shape over its last seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation_out: Option<ClipAnimation>,
    /// A shape over its whole length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation_combo: Option<ClipAnimation>,
    /// The user's own keys, sorted by property and then by `at`. Empty is a
    /// clip whose properties are the constants above.
    ///
    /// These sit *under* the animation presets rather than beside them: a
    /// keyed property's value replaces the constant the preset is relative
    /// to, so a clip can carry both a hand-keyed scale and a Fade preset
    /// without either having to know about the other.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
    pub crop: Option<Crop>,
    /// The background taken away by a mask rather than a key colour; see
    /// [`Cutout`]. Keying by colour is a package on `video_effects`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutout: Option<Cutout>,
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

impl Clip {
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
            Some(index) => self.keys[index] = ClipKey { property, at, value, ease },
            None => self.keys.push(ClipKey { property, at, value, ease }),
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

    /// Drops keys that are not finite or not in `0..=1`, then orders them.
    /// Called by everything that can put a key in, including the document
    /// reader, so a hand-edited file cannot produce an unsorted track.
    pub fn sort_keys(&mut self) {
        self.keys
            .retain(|key| key.at.is_finite() && key.value.is_finite() && (0.0..=1.0).contains(&key.at));
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
