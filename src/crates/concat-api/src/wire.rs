// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The edit commands as the API's JSON spells them, and what each becomes
//! in the document.
//!
//! The wire is the published contract: `op`-tagged objects, camelCase
//! fields, times in seconds, ids as strings, colours as `#rrggbb[aa]`, a
//! key's place as a share of its clip. The document says the same things
//! in its own terms - ticks, typed ids, colours of four channels, keys at
//! ticks of an item's content time - and [`to_doc`] is the one place the
//! first become the second. Nothing here decides what an edit means: that
//! is the document's commands, which every caller shares.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::str::FromStr;

use concat_doc::commands::{self as doc, placeholder};
use concat_doc::model::{self as m, Animated, Entry, ItemId, Model};
use concat_doc::{Tick, TickDur, TickLen, TypedId};
use serde::{Deserialize, Deserializer, Serialize};

// ----- small spellings -----------------------------------------------------

/// What a piece of media is.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    /// Moving pictures, with or without sound.
    #[default]
    Video,
    /// Sound only.
    Audio,
    /// A still.
    Image,
}

/// What made a file, when it was the editor and not an import.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaOrigin {
    /// Read aloud by the speech sheet.
    Speech,
    /// A clip's sound rendered as it played.
    Processed,
}

/// The colour a picture is in.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ColorSpace {
    /// Rec. 709, SDR.
    #[default]
    Sdr,
    /// Rec. 2020 with HLG.
    Hlg,
    /// Rec. 2020 with PQ.
    Pq,
}

impl ColorSpace {
    fn doc(self) -> m::ColorSpace {
        match self {
            ColorSpace::Sdr => m::ColorSpace::Sdr,
            ColorSpace::Hlg => m::ColorSpace::Hlg,
            ColorSpace::Pq => m::ColorSpace::Pq,
        }
    }

    /// The wire's word for a document's colour space.
    pub fn of(space: &m::ColorSpace) -> ColorSpace {
        match space {
            m::ColorSpace::Hlg => ColorSpace::Hlg,
            m::ColorSpace::Pq => ColorSpace::Pq,
            _ => ColorSpace::Sdr,
        }
    }
}

/// One audio stream of a file.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioTrack {
    /// The stream's index in the file.
    pub index: u32,
    /// Its codec.
    pub codec: String,
    /// Its channels.
    pub channels: u32,
    /// Its sample rate.
    pub sample_rate: u32,
    /// Its title.
    pub title: String,
    /// Its language.
    pub language: String,
}

/// A file, as a probe describes it.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMedia {
    /// The file.
    pub path: String,
    /// Its name in the bin.
    pub name: String,
    /// Its length in seconds, when it has one.
    #[serde(default)]
    pub duration: Option<f64>,
    /// What it is.
    pub kind: MediaKind,
    /// Its picture's width.
    #[serde(default)]
    pub width: Option<u32>,
    /// Its picture's height.
    #[serde(default)]
    pub height: Option<u32>,
    /// Its frame rate, as a decimal.
    #[serde(default)]
    pub frame_rate: Option<f64>,
    /// Its frame rate, exactly: "30000/1001".
    #[serde(default)]
    pub frame_rate_fraction: Option<String>,
    /// Its video codec.
    #[serde(default)]
    pub video_codec: Option<String>,
    /// Its audio codec.
    #[serde(default)]
    pub audio_codec: Option<String>,
    /// Whether it has sound.
    pub has_audio: bool,
    /// Its audio streams.
    #[serde(default)]
    pub audio_tracks: Vec<AudioTrack>,
    /// What made it, when it was the editor.
    #[serde(default)]
    pub origin: Option<MediaOrigin>,
    /// What its picture was recorded in.
    #[serde(default)]
    pub color_space: ColorSpace,
}

impl NewMedia {
    /// The document's file.
    pub fn doc(&self) -> Result<m::Media, String> {
        let kind = match self.kind {
            MediaKind::Video => m::MediaKind::Video,
            MediaKind::Audio => m::MediaKind::Audio,
            MediaKind::Image => m::MediaKind::Image,
        };
        let length = match self.kind {
            MediaKind::Image => None,
            _ => match self.duration {
                Some(seconds) => length(seconds, "duration").ok(),
                None => None,
            },
        };
        let rate = self
            .frame_rate_fraction
            .as_deref()
            .and_then(|fraction| {
                let (num, den) = fraction.split_once('/')?;
                concat_doc::Rate::new(num.trim().parse().ok()?, den.trim().parse().ok()?).ok()
            })
            .or_else(|| {
                self.frame_rate
                    .and_then(|fps| concat_doc::Rate::from_decimal(fps).ok())
            });
        let has_sound = self.has_audio && self.kind != MediaKind::Image;
        let mut streams: Vec<m::SoundStream> = Vec::new();
        if has_sound {
            for track in &self.audio_tracks {
                if !streams.iter().any(|kept| kept.index == track.index) {
                    streams.push(m::SoundStream {
                        index: track.index,
                        channels: track.channels,
                        sample_rate: track.sample_rate,
                        codec: track.codec.clone(),
                        language: track.language.clone(),
                        title: track.title.clone(),
                    });
                }
            }
        }
        Ok(m::Media {
            path: self.path.clone(),
            name: self.name.clone(),
            kind,
            length,
            rate,
            size: match (self.width, self.height) {
                (Some(width), Some(height)) if self.kind != MediaKind::Audio => {
                    m::Size::new(width, height)
                }
                _ => None,
            },
            has_sound,
            streams,
            video_codec: self.video_codec.clone(),
            audio_codec: self.audio_codec.clone(),
            color: self.color_space.doc(),
            origin: self.origin.map(|origin| match origin {
                MediaOrigin::Speech => m::MediaOrigin::Speech,
                MediaOrigin::Processed => m::MediaOrigin::Processed,
            }),
            ..m::Media::default()
        })
    }
}

/// A timeline's frame, rate and colour.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSettings {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub rate_num: i64,
    /// Frame rate denominator.
    pub rate_den: i64,
    /// What shows where no picture covers the frame, `#rrggbb[aa]`;
    /// opaque black when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<String>,
    /// What the timeline is output in.
    #[serde(default)]
    pub color_space: ColorSpace,
}

impl Default for VideoSettings {
    fn default() -> VideoSettings {
        VideoSettings {
            width: 1920,
            height: 1080,
            rate_num: 30,
            rate_den: 1,
            canvas: None,
            color_space: ColorSpace::Sdr,
        }
    }
}

impl VideoSettings {
    /// The canvas as four channels.
    pub fn canvas_rgba(&self) -> Result<[u8; 4], String> {
        match &self.canvas {
            None => Ok(concat_export::rgba::OPAQUE_BLACK),
            Some(hex) => {
                concat_export::rgba::parse_hex(hex).ok_or_else(|| format!("{hex:?} is no colour"))
            }
        }
    }

    /// The document's frame.
    pub fn doc(&self) -> Result<m::Frame, String> {
        let (Some(width), Some(height)) = (
            std::num::NonZeroU32::new(self.width),
            std::num::NonZeroU32::new(self.height),
        ) else {
            return Err("A frame needs a width and a height".to_owned());
        };
        let rate = concat_doc::Rate::new(
            u32::try_from(self.rate_num).map_err(|_| "the frame rate is out of range")?,
            u32::try_from(self.rate_den).map_err(|_| "the frame rate is out of range")?,
        )
        .map_err(|error| error.to_string())?;
        let [r, g, b, a] = self.canvas_rgba()?;
        Ok(m::Frame {
            width,
            height,
            rate,
            canvas: m::Color { r, g, b, a },
            color: self.color_space.doc(),
        })
    }

    /// The wire's spelling of a document's frame.
    pub fn of(frame: &m::Frame) -> VideoSettings {
        let c = frame.canvas;
        VideoSettings {
            width: frame.width.get(),
            height: frame.height.get(),
            rate_num: i64::from(frame.rate.num()),
            rate_den: i64::from(frame.rate.den()),
            canvas: Some(concat_export::rgba::to_hex([c.r, c.g, c.b, c.a])),
            color_space: ColorSpace::of(&frame.color),
        }
    }
}

/// A key's easing: a CSS timing function's two control points.
pub type KeyEase = [f64; 4];

const LINEAR: KeyEase = [0.0, 0.0, 1.0, 1.0];

fn linear() -> KeyEase {
    LINEAR
}

fn one() -> f64 {
    1.0
}

/// Which of a clip's numbers a key is on.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyProperty {
    /// Its scale.
    Scale,
    /// Across.
    OffsetX,
    /// Down.
    OffsetY,
    /// Its turn.
    Rotation,
    /// Its width beyond its scale.
    StretchX,
    /// Its height beyond its scale.
    StretchY,
    /// How much of it shows.
    Opacity,
    /// Its level.
    Volume,
}

impl KeyProperty {
    fn doc(self) -> doc::KeyProperty {
        match self {
            KeyProperty::Scale => doc::KeyProperty::Scale,
            KeyProperty::OffsetX => doc::KeyProperty::OffsetX,
            KeyProperty::OffsetY => doc::KeyProperty::OffsetY,
            KeyProperty::Rotation => doc::KeyProperty::Rotation,
            KeyProperty::StretchX => doc::KeyProperty::StretchX,
            KeyProperty::StretchY => doc::KeyProperty::StretchY,
            KeyProperty::Opacity => doc::KeyProperty::Opacity,
            KeyProperty::Volume => doc::KeyProperty::Volume,
        }
    }
}

/// Which end of a clip a trim drags.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrimEdge {
    /// The head.
    Start,
    /// The tail.
    End,
}

/// A lane's two toggles.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackFlag {
    /// Its pictures drawn.
    Visible,
    /// Its sound silent.
    Muted,
}

/// The two ways a picture is sized to the frame.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Framing {
    /// All of it inside the frame.
    Fit,
    /// The frame covered.
    Fill,
}

/// The levels a file's picture spans.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorRange {
    /// 16-235.
    Limited,
    /// 0-255.
    Full,
}

/// Which of a file's beats are marked.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BeatMarks {
    /// As many as the zoom has room for.
    Zoom,
    /// Every beat.
    Every,
    /// Every second beat.
    Second,
    /// The first of each bar.
    Bar,
}

/// One point of a speed curve.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedPoint {
    /// Where in the clip, `0..=1`.
    pub at: f64,
    /// The speed there.
    pub speed: f64,
    /// How fast it changes there; with one on every point the curve bends.
    #[serde(default)]
    pub slope: Option<f64>,
}

/// Where one clip of a drag goes.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipMove {
    /// The clip.
    pub clip_id: String,
    /// Its new start, in seconds.
    pub start: f64,
    /// Its new lane; left out keeps its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
}

/// One key on one knob of an effect, or one number of a mask.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamKey {
    /// Where in the clip, `0..=1`.
    pub at: f64,
    /// The value there.
    pub value: f64,
    /// How it is approached.
    #[serde(default = "linear")]
    pub ease: KeyEase,
}

/// A package applied to a clip: an effect or a filter.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedFilter {
    /// The package's id.
    pub id: String,
    /// The knobs set, by key.
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
    /// Whether it runs.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The knobs that ride, by key.
    #[serde(default)]
    pub keys: BTreeMap<String, Vec<ParamKey>>,
}

fn yes() -> bool {
    true
}

/// A transition on the cut into a clip.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    /// The package.
    pub id: String,
    /// Seconds it covers.
    #[serde(default = "one")]
    pub duration: f64,
    /// Its knobs.
    #[serde(default)]
    pub params: BTreeMap<String, f64>,
}

/// What a crop cuts off each edge, as shares of the picture.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Crop {
    /// Off the left.
    pub left: f64,
    /// Off the top.
    pub top: f64,
    /// Off the right.
    pub right: f64,
    /// Off the bottom.
    pub bottom: f64,
}

/// What fills the frame where the picture does not reach.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Background {
    /// Black.
    None,
    /// The clip's own frame, enlarged and blurred.
    Blur {
        /// How soft, `0..=1`.
        amount: f64,
    },
    /// One colour.
    Color {
        /// `#rrggbb`.
        color: String,
    },
    /// A still from the bin.
    Image {
        /// The bin item.
        media_id: String,
    },
}

/// What a project's cover is.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Cover {
    /// A frame of the edit, `time` seconds in.
    Frame {
        /// Seconds in.
        time: f64,
    },
    /// A picture from disk.
    Picture {
        /// The file.
        path: String,
    },
}

/// An entrance or an exit.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionMove {
    /// The move's name.
    pub kind: String,
    /// Seconds it takes.
    #[serde(default = "half")]
    pub seconds: f64,
}

fn half() -> f64 {
    0.5
}

/// A loop.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipLoop {
    /// The loop's name.
    pub kind: String,
    /// Seconds a lap takes.
    #[serde(default = "one")]
    pub seconds: f64,
    /// How much of it.
    #[serde(default = "one")]
    pub amount: f64,
}

/// A clip's entrance, exit and loop.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClipMotion {
    /// How it comes on.
    #[serde(default, rename = "in")]
    pub enter: Option<MotionMove>,
    /// How it goes off.
    #[serde(default, rename = "out")]
    pub exit: Option<MotionMove>,
    /// What it does all along.
    #[serde(default, rename = "loop")]
    pub repeat: Option<ClipLoop>,
}

/// Which shape a mask keeps.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MaskShape {
    /// One side of a line.
    Linear,
    /// A band along a line.
    Mirror,
    /// An ellipse.
    #[default]
    Circle,
    /// A box.
    Rectangle,
    /// A heart.
    Heart,
    /// A star.
    Star,
}

/// How a mask joins the masks above it.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    /// A union.
    #[default]
    Add,
    /// Takes away.
    Subtract,
    /// Keeps only what both have.
    Intersect,
    /// Cuts nothing.
    None,
}

/// What a mask is drawn from.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MaskSource {
    /// A shape in a box.
    Shape {
        /// Which shape.
        #[serde(default)]
        shape: MaskShape,
        /// The box's width, in the picture's shorter side.
        #[serde(default)]
        width: Option<f64>,
        /// The box's height.
        #[serde(default)]
        height: Option<f64>,
        /// How round a box's corners are.
        #[serde(default)]
        roundness: f64,
    },
}

impl Default for MaskSource {
    fn default() -> MaskSource {
        MaskSource::Shape {
            shape: MaskShape::Circle,
            width: Some(0.66),
            height: Some(0.66),
            roundness: 0.0,
        }
    }
}

/// One of a clip's masks.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mask {
    /// Its id; left out, one is minted.
    pub id: String,
    /// Its name.
    pub name: String,
    /// How it joins the masks above.
    pub mode: MaskMode,
    /// Keeps the outside.
    pub invert: bool,
    /// What it is drawn from.
    pub source: MaskSource,
    /// Across from centred.
    pub x: f64,
    /// Down from centred.
    pub y: f64,
    /// A factor on its size.
    pub scale: f64,
    /// Its turn.
    pub rotation: f64,
    /// Its edge's softness.
    pub feather: f64,
    /// How far its edge moves out.
    pub expand: f64,
    /// How much it counts.
    pub opacity: f64,
    /// The numbers that ride, by name.
    pub keys: BTreeMap<String, Vec<ParamKey>>,
}

impl Default for Mask {
    fn default() -> Mask {
        Mask {
            id: String::new(),
            name: String::new(),
            mode: MaskMode::Add,
            invert: false,
            source: MaskSource::default(),
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            rotation: 0.0,
            feather: 0.0,
            expand: 0.0,
            opacity: 1.0,
            keys: BTreeMap::new(),
        }
    }
}

/// How a title's lines sit.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    /// A left edge.
    Left,
    /// Centred.
    #[default]
    Center,
    /// A right edge.
    Right,
}

/// A title's display casing.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextCase {
    /// All upper.
    Upper,
    /// All lower.
    Lower,
    /// Sentence case.
    Sentence,
    /// As typed.
    #[default]
    Original,
}

/// A stretch of a title's words with a look of its own.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    /// The first character.
    pub start: u32,
    /// One past the last.
    pub end: u32,
    /// Fill colour.
    #[serde(default)]
    pub color: Option<String>,
    /// Size as a multiple of the title's.
    #[serde(default)]
    pub font_scale: Option<f64>,
    /// Weight.
    #[serde(default)]
    pub font_weight: Option<f64>,
    /// Italic.
    #[serde(default)]
    pub italic: Option<bool>,
    /// Underlined.
    #[serde(default)]
    pub underline: Option<bool>,
    /// Struck through.
    #[serde(default)]
    pub strikethrough: Option<bool>,
    /// Outline colour.
    #[serde(default)]
    pub stroke_color: Option<String>,
}

/// A title's words and look. A field left out is the default's.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextStyle {
    /// The words.
    pub content: String,
    /// The family.
    pub font_family: String,
    /// Cap height as a share of the frame's height.
    pub font_size: f64,
    /// Weight, 100 to 900.
    pub font_weight: f64,
    /// Italic.
    pub italic: bool,
    /// Underlined.
    pub underline: bool,
    /// Struck through.
    pub strikethrough: bool,
    /// Fill colour.
    pub color: String,
    /// The whole title's opacity.
    pub opacity: f64,
    /// How the lines sit.
    pub align: TextAlign,
    /// Display casing.
    pub text_case: TextCase,
    /// Line height.
    pub line_height: f64,
    /// Letter spacing.
    pub tracking: f64,
    /// Widest the block runs; zero for no limit.
    pub max_width: f64,
    /// Tallest; zero for no limit.
    pub max_height: f64,
    /// Outline colour.
    pub stroke_color: String,
    /// Outline thickness.
    pub stroke_width: f64,
    /// A drop shadow.
    pub shadow: bool,
    /// A plate behind the words, `#rrggbb[aa]`; empty for none.
    pub background: String,
    /// The plate's air either side.
    pub background_padding_x: f64,
    /// The plate's air above and below.
    pub background_padding_y: f64,
    /// The plate's corners.
    pub background_radius: f64,
    /// Words with a look of their own.
    pub runs: Vec<TextRun>,
}

impl Default for TextStyle {
    fn default() -> TextStyle {
        TextStyle::of(&m::Title::default())
    }
}

fn hex(color: m::Color) -> String {
    concat_export::rgba::to_hex([color.r, color.g, color.b, color.a])
}

fn colour(text: &str) -> Result<m::Color, String> {
    let [r, g, b, a] =
        concat_export::rgba::parse_hex(text).ok_or_else(|| format!("{text:?} is no colour"))?;
    Ok(m::Color { r, g, b, a })
}

fn colour_or_none(text: &str) -> Result<Option<m::Color>, String> {
    if text.trim().is_empty() {
        Ok(None)
    } else {
        colour(text).map(Some)
    }
}

impl TextStyle {
    /// The wire's spelling of a document's title.
    pub fn of(title: &m::Title) -> TextStyle {
        TextStyle {
            content: title.content.clone(),
            font_family: title.font_family.clone(),
            font_size: title.font_size,
            font_weight: title.font_weight,
            italic: title.italic,
            underline: title.underline,
            strikethrough: title.strikethrough,
            color: hex(title.color),
            opacity: title.opacity,
            align: match title.align {
                m::TextAlign::Left => TextAlign::Left,
                m::TextAlign::Right => TextAlign::Right,
                _ => TextAlign::Center,
            },
            text_case: match title.case {
                m::TextCase::Upper => TextCase::Upper,
                m::TextCase::Lower => TextCase::Lower,
                m::TextCase::Sentence => TextCase::Sentence,
                _ => TextCase::Original,
            },
            line_height: title.line_height,
            tracking: title.tracking,
            max_width: title.max_width,
            max_height: title.max_height,
            stroke_color: hex(title.stroke_color),
            stroke_width: title.stroke_width,
            shadow: title.shadow,
            background: title.plate.map(hex).unwrap_or_default(),
            background_padding_x: title.plate_padding_x,
            background_padding_y: title.plate_padding_y,
            background_radius: title.plate_radius,
            runs: title
                .runs
                .iter()
                .map(|run| TextRun {
                    start: run.start,
                    end: run.end,
                    color: run.color.map(hex),
                    font_scale: run.font_scale,
                    font_weight: run.font_weight,
                    italic: run.italic,
                    underline: run.underline,
                    strikethrough: run.strikethrough,
                    stroke_color: run.stroke_color.map(hex),
                })
                .collect(),
        }
    }

    /// The document's title.
    pub fn doc(&self) -> Result<m::Title, String> {
        let mut runs = Vec::with_capacity(self.runs.len());
        for run in &self.runs {
            runs.push(m::TextRun {
                start: run.start,
                end: run.end,
                color: run.color.as_deref().map(colour).transpose()?,
                font_scale: run.font_scale,
                font_weight: run.font_weight,
                italic: run.italic,
                underline: run.underline,
                strikethrough: run.strikethrough,
                stroke_color: run.stroke_color.as_deref().map(colour).transpose()?,
            });
        }
        Ok(m::Title {
            content: self.content.clone(),
            font_family: self.font_family.clone(),
            font_size: self.font_size,
            font_weight: self.font_weight,
            italic: self.italic,
            underline: self.underline,
            strikethrough: self.strikethrough,
            color: colour(&self.color)?,
            opacity: self.opacity,
            align: match self.align {
                TextAlign::Left => m::TextAlign::Left,
                TextAlign::Center => m::TextAlign::Center,
                TextAlign::Right => m::TextAlign::Right,
            },
            case: match self.text_case {
                TextCase::Upper => m::TextCase::Upper,
                TextCase::Lower => m::TextCase::Lower,
                TextCase::Sentence => m::TextCase::Sentence,
                TextCase::Original => m::TextCase::Original,
            },
            line_height: self.line_height,
            tracking: self.tracking,
            max_width: self.max_width,
            max_height: self.max_height,
            stroke_color: colour(&self.stroke_color)?,
            stroke_width: self.stroke_width,
            shadow: self.shadow,
            plate: colour_or_none(&self.background)?,
            plate_padding_x: self.background_padding_x,
            plate_padding_y: self.background_padding_y,
            plate_radius: self.background_radius,
            runs,
        })
    }
}

/// Which figure a shape draws.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ShapeKind {
    /// A rectangle.
    #[default]
    Square,
    /// An ellipse.
    Circle,
    /// Point up.
    Triangle,
    /// Leaning right.
    Parallelogram,
    /// Narrower at the top.
    Trapezoid,
    /// A stroke.
    Line,
    /// A line with a head.
    Arrow,
    /// A regular polygon.
    Polygon,
    /// A star.
    Star,
    /// A heart.
    Heart,
}

/// What a line wears on an end.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineHead {
    /// Its own round end.
    None,
    /// Two strokes, open.
    Arrow,
    /// A filled point.
    Triangle,
    /// A dot.
    Circle,
    /// A bar across.
    Bar,
}

/// Where an outline sits.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokeAlign {
    /// Straddling the edge.
    #[default]
    Center,
    /// Inside it.
    Inside,
    /// Outside it.
    Outside,
}

/// The outline's pattern.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokeDash {
    /// Unbroken.
    #[default]
    Solid,
    /// Dashes.
    Dashed,
    /// Dots.
    Dotted,
}

/// A blend from the fill into a second colour.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShapeGradient {
    /// The colour it ends in.
    pub color: String,
    /// Which way a straight blend runs.
    pub angle: f64,
    /// Out from the middle.
    pub radial: bool,
}

impl Default for ShapeGradient {
    fn default() -> ShapeGradient {
        ShapeGradient {
            color: "#000000".to_owned(),
            angle: 90.0,
            radial: false,
        }
    }
}

/// A shadow or a glow.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShapeShadow {
    /// Its colour.
    pub color: String,
    /// Which way it falls.
    pub angle: f64,
    /// How far.
    pub distance: f64,
    /// How soft.
    pub blur: f64,
}

impl Default for ShapeShadow {
    fn default() -> ShapeShadow {
        ShapeShadow {
            color: "#00000080".to_owned(),
            angle: 135.0,
            distance: 0.01,
            blur: 0.02,
        }
    }
}

/// How a shape is painted. A field left out is the default's.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShapeStyle {
    /// The figure.
    pub kind: ShapeKind,
    /// Its width, as a share of the frame's height.
    pub width: f64,
    /// Its height; zero for its own proportion.
    pub height: f64,
    /// How round its corners are.
    pub corner_radius: f64,
    /// A polygon's sides.
    pub sides: u32,
    /// A star's points.
    pub points: u32,
    /// How far out a star's inner corners sit.
    pub inner_radius: f64,
    /// The inside's colour; empty for an outline alone.
    pub fill: String,
    /// A blend into a second colour.
    pub gradient: Option<ShapeGradient>,
    /// The outline's colour; empty for none.
    pub stroke: String,
    /// The outline's thickness.
    pub stroke_width: f64,
    /// Where the outline sits.
    pub stroke_align: StrokeAlign,
    /// The outline's pattern.
    pub dash: StrokeDash,
    /// Where the outline starts, 0 to 1.
    pub trim_start: f64,
    /// Where it ends.
    pub trim_end: f64,
    /// What a line's start wears.
    pub start_head: Option<LineHead>,
    /// What a line's end wears.
    pub end_head: Option<LineHead>,
    /// A shadow or a glow.
    pub shadow: Option<ShapeShadow>,
}

impl Default for ShapeStyle {
    fn default() -> ShapeStyle {
        ShapeStyle::of(&m::ShapeStyle::default())
    }
}

fn head_of(head: &m::LineHead) -> LineHead {
    match head {
        m::LineHead::Arrow => LineHead::Arrow,
        m::LineHead::Triangle => LineHead::Triangle,
        m::LineHead::Circle => LineHead::Circle,
        m::LineHead::Bar => LineHead::Bar,
        _ => LineHead::None,
    }
}

fn head_doc(head: LineHead) -> m::LineHead {
    match head {
        LineHead::None => m::LineHead::None,
        LineHead::Arrow => m::LineHead::Arrow,
        LineHead::Triangle => m::LineHead::Triangle,
        LineHead::Circle => m::LineHead::Circle,
        LineHead::Bar => m::LineHead::Bar,
    }
}

impl ShapeStyle {
    /// The wire's spelling of a document's shape.
    pub fn of(style: &m::ShapeStyle) -> ShapeStyle {
        ShapeStyle {
            kind: match style.kind {
                m::ShapeKind::Circle => ShapeKind::Circle,
                m::ShapeKind::Triangle => ShapeKind::Triangle,
                m::ShapeKind::Parallelogram => ShapeKind::Parallelogram,
                m::ShapeKind::Trapezoid => ShapeKind::Trapezoid,
                m::ShapeKind::Line => ShapeKind::Line,
                m::ShapeKind::Arrow => ShapeKind::Arrow,
                m::ShapeKind::Polygon => ShapeKind::Polygon,
                m::ShapeKind::Star => ShapeKind::Star,
                m::ShapeKind::Heart => ShapeKind::Heart,
                _ => ShapeKind::Square,
            },
            width: style.width,
            height: style.height,
            corner_radius: style.corner_radius,
            sides: style.sides,
            points: style.points,
            inner_radius: style.inner_radius,
            fill: style.fill.map(hex).unwrap_or_default(),
            gradient: style.gradient.as_ref().map(|g| ShapeGradient {
                color: hex(g.color),
                angle: g.angle,
                radial: g.radial,
            }),
            stroke: style.stroke.map(hex).unwrap_or_default(),
            stroke_width: style.stroke_width,
            stroke_align: match style.stroke_align {
                m::StrokeAlign::Inside => StrokeAlign::Inside,
                m::StrokeAlign::Outside => StrokeAlign::Outside,
                _ => StrokeAlign::Center,
            },
            dash: match style.dash {
                m::StrokeDash::Dashed => StrokeDash::Dashed,
                m::StrokeDash::Dotted => StrokeDash::Dotted,
                _ => StrokeDash::Solid,
            },
            trim_start: style.trim_start,
            trim_end: style.trim_end,
            start_head: style.start_head.as_ref().map(head_of),
            end_head: style.end_head.as_ref().map(head_of),
            shadow: style.shadow.as_ref().map(|s| ShapeShadow {
                color: hex(s.color),
                angle: s.angle,
                distance: s.distance,
                blur: s.blur,
            }),
        }
    }

    /// The document's shape.
    pub fn doc(&self) -> Result<m::ShapeStyle, String> {
        Ok(m::ShapeStyle {
            kind: match self.kind {
                ShapeKind::Square => m::ShapeKind::Square,
                ShapeKind::Circle => m::ShapeKind::Circle,
                ShapeKind::Triangle => m::ShapeKind::Triangle,
                ShapeKind::Parallelogram => m::ShapeKind::Parallelogram,
                ShapeKind::Trapezoid => m::ShapeKind::Trapezoid,
                ShapeKind::Line => m::ShapeKind::Line,
                ShapeKind::Arrow => m::ShapeKind::Arrow,
                ShapeKind::Polygon => m::ShapeKind::Polygon,
                ShapeKind::Star => m::ShapeKind::Star,
                ShapeKind::Heart => m::ShapeKind::Heart,
            },
            width: self.width,
            height: self.height,
            corner_radius: self.corner_radius,
            sides: self.sides,
            points: self.points,
            inner_radius: self.inner_radius,
            fill: colour_or_none(&self.fill)?,
            gradient: self
                .gradient
                .as_ref()
                .map(|g| -> Result<m::Gradient, String> {
                    Ok(m::Gradient {
                        color: colour(&g.color)?,
                        angle: g.angle,
                        radial: g.radial,
                    })
                })
                .transpose()?,
            stroke: colour_or_none(&self.stroke)?,
            stroke_width: self.stroke_width,
            stroke_align: match self.stroke_align {
                StrokeAlign::Center => m::StrokeAlign::Center,
                StrokeAlign::Inside => m::StrokeAlign::Inside,
                StrokeAlign::Outside => m::StrokeAlign::Outside,
            },
            dash: match self.dash {
                StrokeDash::Solid => m::StrokeDash::Solid,
                StrokeDash::Dashed => m::StrokeDash::Dashed,
                StrokeDash::Dotted => m::StrokeDash::Dotted,
            },
            trim_start: self.trim_start,
            trim_end: self.trim_end,
            start_head: self.start_head.map(head_doc),
            end_head: self.end_head.map(head_doc),
            shadow: self
                .shadow
                .as_ref()
                .map(|s| -> Result<m::ShapeShadow, String> {
                    Ok(m::ShapeShadow {
                        color: colour(&s.color)?,
                        angle: s.angle,
                        distance: s.distance,
                        blur: s.blur,
                    })
                })
                .transpose()?,
        })
    }
}

/// The looks the Stickers page offers: every figure, and a few made of
/// them.
pub fn shape_cards() -> Vec<(String, ShapeStyle)> {
    let base = m::ShapeStyle::default;
    let white = m::Color::WHITE;
    let figures = [
        ("square", m::ShapeKind::Square),
        ("circle", m::ShapeKind::Circle),
        ("triangle", m::ShapeKind::Triangle),
        ("parallelogram", m::ShapeKind::Parallelogram),
        ("trapezoid", m::ShapeKind::Trapezoid),
        ("polygon", m::ShapeKind::Polygon),
        ("star", m::ShapeKind::Star),
        ("heart", m::ShapeKind::Heart),
        ("line", m::ShapeKind::Line),
        ("arrow", m::ShapeKind::Arrow),
    ];
    let mut cards: Vec<(String, m::ShapeStyle)> = figures
        .into_iter()
        .map(|(id, kind)| (id.to_owned(), m::ShapeStyle { kind, ..base() }))
        .collect();
    cards.push((
        "rounded".to_owned(),
        m::ShapeStyle {
            corner_radius: 0.35,
            ..base()
        },
    ));
    cards.push((
        "pill".to_owned(),
        m::ShapeStyle {
            height: 0.14,
            corner_radius: 1.0,
            ..base()
        },
    ));
    cards.push((
        "pentagon".to_owned(),
        m::ShapeStyle {
            kind: m::ShapeKind::Polygon,
            sides: 5,
            ..base()
        },
    ));
    cards.push((
        "ring".to_owned(),
        m::ShapeStyle {
            kind: m::ShapeKind::Circle,
            fill: None,
            stroke: Some(white),
            stroke_width: 0.03,
            ..base()
        },
    ));
    cards.push((
        "double-arrow".to_owned(),
        m::ShapeStyle {
            kind: m::ShapeKind::Arrow,
            start_head: Some(m::LineHead::Arrow),
            ..base()
        },
    ));
    cards
        .into_iter()
        .map(|(id, style)| (id, ShapeStyle::of(&style)))
        .collect()
}

/// A run style's fields, for `styleTextRange`.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RunStyle {
    /// Fill colour.
    pub color: Option<String>,
    /// Size as a multiple.
    pub font_scale: Option<f64>,
    /// Weight.
    pub font_weight: Option<f64>,
    /// Italic.
    pub italic: Option<bool>,
    /// Underlined.
    pub underline: Option<bool>,
    /// Struck through.
    pub strikethrough: Option<bool>,
    /// Outline colour.
    pub stroke_color: Option<String>,
}

/// How the captions on a caption lane look and sit.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CaptionLook {
    /// The style every caption is drawn in; its words and runs are not
    /// used.
    pub style: TextStyle,
    /// How far from the centre, a share of the height, down positive.
    pub offset_y: f64,
}

/// One caption to place.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionCue {
    /// Seconds on the timeline.
    pub start: f64,
    /// Seconds it stays up.
    pub duration: f64,
    /// What it says.
    pub text: String,
}

/// The two ways a cutout decides what stays.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CutoutMode {
    /// The model's mask.
    Auto,
    /// Then the strokes.
    Custom,
}

/// What a cutout keeps.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Subject {
    /// Whichever fits.
    #[default]
    Auto,
    /// A person.
    Person,
    /// The object.
    Object,
}

/// The four brushes.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrushTool {
    /// Keeps what might be the subject.
    SmartBrush,
    /// Keeps everything under it.
    Brush,
    /// Removes what the model was unsure of.
    SmartEraser,
    /// Removes everything under it.
    Eraser,
}

/// One brush stroke.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    /// What it does.
    pub tool: BrushTool,
    /// The brush's diameter.
    pub size: f64,
    /// The path, as shares of the source picture.
    pub points: Vec<[f64; 2]>,
    /// The source second it was painted at.
    #[serde(default)]
    pub at: Option<f64>,
}

/// A picture's background taken away by a model.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cutout {
    /// Automatic or custom.
    pub mode: CutoutMode,
    /// What the model looks for.
    #[serde(default)]
    pub subject: Subject,
    /// How far the edge is softened.
    #[serde(default = "feather")]
    pub feather: f64,
    /// The corrections painted.
    #[serde(default)]
    pub strokes: Vec<Stroke>,
}

fn feather() -> f64 {
    0.01
}

/// A clip's stabilization.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stabilize {
    /// How much of the movement is smoothed away.
    pub smoothness: f64,
    /// The most the picture may be zoomed.
    pub max_zoom: f64,
    /// Leave the roll as shot.
    pub keep_rotation: bool,
    /// Lock the frame.
    pub tripod: bool,
}

impl Default for Stabilize {
    fn default() -> Stabilize {
        Stabilize {
            smoothness: 0.5,
            max_zoom: 1.2,
            keep_rotation: false,
            tripod: false,
        }
    }
}

/// The two kinds of voice isolation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VoiceKind {
    /// Singing apart from music.
    Vocals,
    /// Speech apart from noise.
    #[default]
    Speech,
}

/// Voice isolation on a clip's sound.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VoiceMix {
    /// What the voice is set apart from.
    pub kind: VoiceKind,
    /// The voice's gain.
    pub voice: f64,
    /// Everything else's gain.
    pub background: f64,
}

impl Default for VoiceMix {
    fn default() -> VoiceMix {
        VoiceMix {
            kind: VoiceKind::Speech,
            voice: 1.0,
            background: 0.0,
        }
    }
}

/// `Some(None)` for a field sent as `null`, `Some(value)` for one sent,
/// and (by `#[serde(default)]`) `None` for one left out.
fn double<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A change to some of a clip's settings. A field left out is left alone;
/// for the double-optional ones, `null` takes the setting off.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClipPatch {
    /// Which audio stream it plays; `null` is the file's first.
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub audio_stream: Option<Option<u32>>,
    /// Its own background; `null` follows the timeline's.
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub background: Option<Option<Background>>,
    /// The blend mode's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<String>,
    /// The crop; `null` takes it off.
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub crop: Option<Option<Crop>>,
    /// Fade in, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<f64>,
    /// Fade out, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<f64>,
    /// The sound filters, whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<Vec<AppliedFilter>>,
    /// Mirrored left to right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_h: Option<bool>,
    /// Mirrored top to bottom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip_v: Option<bool>,
    /// The masks, whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masks: Option<Vec<Mask>>,
    /// The entrance, exit and loop; `null` takes them off.
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub motion: Option<Option<ClipMotion>>,
    /// Its own sound silenced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    /// Its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its opacity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// Voices kept at their pitch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserve_pitch: Option<bool>,
    /// A shape's figure and paint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ShapeStyle>,
    /// A title's words and look.
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub text: Option<Option<TextStyle>>,
    /// The transition on the cut into it; `null` takes it off.
    #[serde(
        default,
        deserialize_with = "double",
        skip_serializing_if = "Option::is_none"
    )]
    pub transition_in: Option<Option<Transition>>,
    /// The picture effects, whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_effects: Option<Vec<AppliedFilter>>,
    /// Its gain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
}

// ----- the commands --------------------------------------------------------

/// One edit, as the API's JSON says it.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[allow(missing_docs, clippy::large_enum_variant)]
pub enum Command {
    AddMedia {
        item: NewMedia,
    },
    RemoveMedia {
        media_id: String,
    },
    SetMediaPlaceholder {
        media_id: String,
        placeholder: bool,
    },
    FillSlot {
        media_id: String,
        item: NewMedia,
    },
    Batch {
        commands: Vec<Command>,
    },
    AddClip {
        media_id: String,
        track_id: String,
        start: f64,
        #[serde(default)]
        ripple: bool,
    },
    AddClipAtFirstFree {
        media_id: String,
        start: f64,
    },
    AddClipsInSequence {
        media_ids: Vec<String>,
        start: f64,
        #[serde(default)]
        track_id: Option<String>,
        #[serde(default)]
        ripple: bool,
    },
    AddTextClip {
        start: f64,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        track_id: Option<String>,
        #[serde(default)]
        above: bool,
        #[serde(default)]
        offset_y: Option<f64>,
        #[serde(default)]
        style: Option<TextStyle>,
    },
    AddShapeClip {
        start: f64,
        name: String,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        track_id: Option<String>,
        #[serde(default)]
        above: bool,
        #[serde(default)]
        style: Option<ShapeStyle>,
    },
    PlaceCaptions {
        look: CaptionLook,
        cues: Vec<CaptionCue>,
        #[serde(default)]
        track_id: Option<String>,
    },
    SetCaptionLook {
        track_id: String,
        #[serde(default)]
        look: Option<CaptionLook>,
    },
    AddLayerClip {
        start: f64,
        effect_id: String,
        name: String,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        sound: bool,
        #[serde(default)]
        track_id: Option<String>,
    },
    AddTransitionClip {
        start: f64,
        transition_id: String,
        name: String,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        track_id: Option<String>,
    },
    SetClipSpeedCurve {
        clip_id: String,
        #[serde(default)]
        curve: Option<Vec<SpeedPoint>>,
        #[serde(default)]
        ripple: bool,
    },
    SetClipKey {
        clip_id: String,
        property: KeyProperty,
        at: f64,
        value: f64,
        #[serde(default = "linear")]
        ease: KeyEase,
    },
    ClearClipKey {
        clip_id: String,
        property: KeyProperty,
        at: f64,
    },
    ClearClipKeys {
        clip_id: String,
        property: KeyProperty,
    },
    SetEffectKey {
        clip_id: String,
        entry: usize,
        key: String,
        at: f64,
        value: f64,
        #[serde(default = "linear")]
        ease: KeyEase,
    },
    ClearEffectKey {
        clip_id: String,
        entry: usize,
        key: String,
        at: f64,
    },
    ClearEffectKeys {
        clip_id: String,
        entry: usize,
        key: String,
    },
    SetMaskKey {
        clip_id: String,
        mask_id: String,
        number: String,
        at: f64,
        value: f64,
        #[serde(default = "linear")]
        ease: KeyEase,
    },
    ClearMaskKey {
        clip_id: String,
        mask_id: String,
        number: String,
        at: f64,
    },
    ClearMaskKeys {
        clip_id: String,
        mask_id: String,
        number: String,
    },
    SetClipCutout {
        clip_id: String,
        #[serde(default)]
        cutout: Option<Cutout>,
    },
    SetClipStabilize {
        clip_id: String,
        #[serde(default)]
        stabilize: Option<Stabilize>,
    },
    SetClipVoice {
        clip_id: String,
        #[serde(default)]
        voice: Option<VoiceMix>,
    },
    AddCutoutStroke {
        clip_id: String,
        stroke: Stroke,
    },
    MoveClips {
        moves: Vec<ClipMove>,
    },
    MoveClipsToNewTrack {
        moves: Vec<ClipMove>,
        #[serde(default)]
        bottom: bool,
    },
    TrimClip {
        clip_id: String,
        edge: TrimEdge,
        delta: f64,
        #[serde(default)]
        ripple: bool,
    },
    SplitClips {
        clip_ids: Vec<String>,
        time: f64,
    },
    DuplicateClips {
        clip_ids: Vec<String>,
    },
    ReplaceClipMedia {
        clip_id: String,
        item: NewMedia,
        #[serde(default)]
        enhanced: bool,
        #[serde(default)]
        offset: f64,
        #[serde(default)]
        source_start: Option<f64>,
    },
    SetClipEnhanced {
        clip_id: String,
        on: bool,
    },
    FreezeFrame {
        clip_id: String,
        time: f64,
        #[serde(default)]
        duration: Option<f64>,
        #[serde(default)]
        still: Option<NewMedia>,
    },
    MergeClips {
        clip_ids: Vec<String>,
    },
    ConsolidateClips {
        clip_ids: Vec<String>,
        item: NewMedia,
    },
    RemoveClips {
        clip_ids: Vec<String>,
        #[serde(default)]
        ripple: bool,
    },
    UpdateClip {
        clip_id: String,
        patch: ClipPatch,
    },
    StyleTextRange {
        clip_id: String,
        start: u32,
        end: u32,
        patch: RunStyle,
    },
    ClearTextStyles {
        clip_id: String,
        #[serde(default)]
        start: Option<u32>,
        #[serde(default)]
        end: Option<u32>,
    },
    SetClipSpeed {
        clip_id: String,
        speed: f64,
        #[serde(default)]
        ripple: bool,
    },
    SetClipTransform {
        clip_id: String,
        #[serde(default)]
        offset_x: Option<f64>,
        #[serde(default)]
        offset_y: Option<f64>,
        #[serde(default)]
        rotation: Option<f64>,
        #[serde(default)]
        scale: Option<f64>,
        #[serde(default)]
        stretch_x: Option<f64>,
        #[serde(default)]
        stretch_y: Option<f64>,
    },
    FrameClip {
        clip_id: String,
        framing: Framing,
        #[serde(default)]
        at: f64,
    },
    DetachAudio {
        clip_id: String,
    },
    ReattachAudio {
        clip_id: String,
    },
    AddTrack {
        #[serde(default)]
        bottom: bool,
    },
    RemoveTrack {
        track_id: String,
    },
    SetTrackFlag {
        track_id: String,
        flag: TrackFlag,
        value: bool,
    },
    AddTimeline,
    RemoveTimeline {
        timeline_id: String,
    },
    SetTimelineVideo {
        timeline_id: String,
        video: VideoSettings,
    },
    SetTimelineBackground {
        timeline_id: String,
        background: Background,
    },
    SetTimelineLinkage {
        timeline_id: String,
        on: bool,
    },
    RenameTimeline {
        timeline_id: String,
        name: String,
    },
    SelectTimeline {
        timeline_id: String,
    },
    MoveTimeline {
        timeline_id: String,
        index: usize,
    },
    SetCover {
        #[serde(default)]
        cover: Option<Cover>,
    },
    AddFont {
        family: String,
        path: String,
    },
    RemoveFont {
        family: String,
    },
    UpdateMediaPath {
        media_id: String,
        new_path: String,
    },
    SetMediaColorRange {
        media_id: String,
        #[serde(default)]
        range: Option<ColorRange>,
    },
    SetMediaBeats {
        media_id: String,
        #[serde(default)]
        marks: Option<BeatMarks>,
    },
}

// ----- the translation -----------------------------------------------------

/// A typed id from its string.
fn id<I: FromStr>(text: &str) -> Result<I, String>
where
    I::Err: Display,
{
    text.parse::<I>()
        .map_err(|error| format!("{text:?} is no id here: {error}"))
}

fn ids<I: FromStr>(texts: &[String]) -> Result<Vec<I>, String>
where
    I::Err: Display,
{
    texts.iter().map(|text| id(text)).collect()
}

fn finite(value: f64, what: &str) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{what} is not a number"))
    }
}

/// A place in seconds; below zero is zero where a place cannot be.
fn place(seconds: f64, what: &str) -> Result<Tick, String> {
    Tick::from_display_seconds(finite(seconds, what)?)
        .ok_or_else(|| format!("{what} is out of range"))
}

/// A place in seconds, floored at zero.
fn floored(seconds: impl std::borrow::Borrow<f64>, what: &str) -> Result<Tick, String> {
    place(finite(*seconds.borrow(), what)?.max(0.0), what)
}

/// A distance in seconds, floored at zero.
fn floored_distance(seconds: impl std::borrow::Borrow<f64>, what: &str) -> Result<TickDur, String> {
    distance(finite(*seconds.borrow(), what)?.max(0.0), what)
}

/// A distance in seconds.
fn distance(seconds: f64, what: &str) -> Result<TickDur, String> {
    TickDur::from_display_seconds(finite(seconds, what)?)
        .ok_or_else(|| format!("{what} is out of range"))
}

/// A length in seconds, which must be more than nothing.
fn length(seconds: f64, what: &str) -> Result<TickLen, String> {
    let ticks = place(seconds, what)?.get();
    TickLen::try_from(ticks).map_err(|_| format!("{what} must be longer than nothing"))
}

fn optional_length(seconds: Option<f64>, what: &str) -> Result<Option<TickLen>, String> {
    match seconds {
        None => Ok(None),
        // A length of nothing or less is the editorial default.
        Some(seconds) if finite(seconds, what)? <= 0.0 => Ok(None),
        Some(seconds) => length(seconds, what).map(Some),
    }
}

fn ease(ease: KeyEase) -> m::Ease {
    let [x1, y1, x2, y2] = ease;
    m::Ease::Bezier { x1, y1, x2, y2 }
}

/// The item a command names, from the model.
fn item<'a>(model: &'a Model, clip_id: &str) -> Result<(ItemId, &'a m::Item), String> {
    let id: ItemId = id(clip_id)?;
    let item = model
        .items
        .get(id)
        .ok_or_else(|| format!("no clip {clip_id} on any timeline"))?;
    Ok((id, item.as_ref()))
}

/// A share of a clip as a tick of its content time.
fn share_at(item: &m::Item, share: f64) -> Result<Tick, String> {
    let share = finite(share, "at")?.clamp(0.0, 1.0);
    let into = (share * item.len.get() as f64).round() as i64;
    Ok(Tick::new(item.offset.get() + into))
}

/// Keys at shares of a clip as an animated number's keys.
fn keyed(
    item: &m::Item,
    value: f64,
    keys: Option<&Vec<ParamKey>>,
) -> Result<Animated<f64>, String> {
    let mut number = Animated::fixed(value);
    for key in keys.into_iter().flatten() {
        number.keys.push(m::Key {
            at: share_at(item, key.at)?,
            value: finite(key.value, "a key's value")?,
            ease: ease(key.ease),
        });
    }
    number.keys.sort_by_key(|key| key.at);
    number.keys.dedup_by_key(|key| key.at);
    Ok(number)
}

/// A chain of links as the document's effects: each keeps the id of the
/// effect at its place in the item's chain when that is the same
/// package, and is given a new one otherwise.
fn effects(
    item: &m::Item,
    held: &[Entry<m::Effect>],
    links: &[AppliedFilter],
) -> Result<Vec<m::Effect>, String> {
    let known: Vec<&m::Effect> = held.iter().filter_map(Entry::known).collect();
    links
        .iter()
        .enumerate()
        .map(|(at, link)| {
            let effect_id = known
                .get(at)
                .filter(|effect| effect.package == link.id)
                .map_or_else(placeholder, |effect| effect.id);
            let mut effect = m::Effect::new(effect_id, link.id.clone());
            effect.enabled = link.enabled;
            for (name, value) in &link.params {
                effect
                    .params
                    .insert(name.clone(), m::Param::fixed(Some(finite(*value, name)?)));
            }
            for (name, keys) in &link.keys {
                let base = link.params.get(name).copied();
                let mut param = m::Param::fixed(base);
                param.keys = keyed(item, 0.0, Some(keys))?.keys;
                if !param.keys.is_empty() {
                    effect.params.insert(name.clone(), param);
                }
            }
            if let Some(kept) = known.get(at).filter(|effect| effect.package == link.id) {
                effect.mask = kept.mask;
            }
            Ok(effect)
        })
        .collect()
}

/// A wire mask as the document's.
fn mask(item: &m::Item, mask: &Mask) -> Result<m::Mask, String> {
    let held: Vec<m::MaskId> = item
        .masks
        .iter()
        .filter_map(Entry::known)
        .map(|mask| mask.id)
        .collect();
    let mask_id = mask
        .id
        .parse::<m::MaskId>()
        .ok()
        .filter(|id| held.contains(id))
        .unwrap_or_else(placeholder);
    let k = |name: &str, value: f64| keyed(item, value, mask.keys.get(name));
    let MaskSource::Shape {
        shape,
        width,
        height,
        roundness,
    } = &mask.source;
    Ok(m::Mask {
        id: mask_id,
        name: mask.name.clone(),
        mode: match mask.mode {
            MaskMode::Add => m::MaskMode::Add,
            MaskMode::Subtract => m::MaskMode::Subtract,
            MaskMode::Intersect => m::MaskMode::Intersect,
            MaskMode::None => m::MaskMode::None,
        },
        invert: mask.invert,
        source: m::MaskSource::Shape {
            shape: match shape {
                MaskShape::Linear => m::MaskShape::Linear,
                MaskShape::Mirror => m::MaskShape::Mirror,
                MaskShape::Circle => m::MaskShape::Circle,
                MaskShape::Rectangle => m::MaskShape::Rectangle,
                MaskShape::Heart => m::MaskShape::Heart,
                MaskShape::Star => m::MaskShape::Star,
            },
            width: k("width", width.unwrap_or(0.66))?,
            height: k("height", height.unwrap_or(0.66))?,
            roundness: k("roundness", *roundness)?,
        },
        x: k("x", mask.x)?,
        y: k("y", mask.y)?,
        scale: k("scale", mask.scale)?,
        rotation: k("rotation", mask.rotation)?,
        feather: k("feather", mask.feather)?,
        expand: k("expand", mask.expand)?,
        opacity: k("opacity", mask.opacity)?,
        residual: m::Residual::default(),
    })
}

fn mask_number(name: &str) -> Result<doc::MaskNumber, String> {
    Ok(match name {
        "x" => doc::MaskNumber::X,
        "y" => doc::MaskNumber::Y,
        "scale" => doc::MaskNumber::Scale,
        "rotation" => doc::MaskNumber::Rotation,
        "feather" => doc::MaskNumber::Feather,
        "expand" => doc::MaskNumber::Expand,
        "opacity" => doc::MaskNumber::Opacity,
        "width" => doc::MaskNumber::Width,
        "height" => doc::MaskNumber::Height,
        "roundness" => doc::MaskNumber::Roundness,
        other => return Err(format!("{other:?} is no number of a mask")),
    })
}

fn background(model: &Model, background: &Background) -> Result<m::Background, String> {
    let _ = model;
    Ok(match background {
        Background::None => m::Background::None,
        Background::Blur { amount } => m::Background::Blur {
            amount: finite(*amount, "amount")?,
        },
        Background::Color { color } => m::Background::Color(colour(color)?),
        Background::Image { media_id } => m::Background::Image(id(media_id)?),
    })
}

fn motion(motion: &ClipMotion) -> Result<m::Motion, String> {
    let moving = |m: &MotionMove| -> Result<m::Move, String> {
        Ok(m::Move {
            name: m.kind.clone(),
            len: length(m.seconds, "a move's seconds")?,
        })
    };
    Ok(m::Motion {
        enter: motion.enter.as_ref().map(moving).transpose()?,
        exit: motion.exit.as_ref().map(moving).transpose()?,
        repeat: motion
            .repeat
            .as_ref()
            .map(|l| -> Result<m::Loop, String> {
                Ok(m::Loop {
                    name: l.kind.clone(),
                    lap: length(l.seconds, "a loop's seconds")?,
                    amount: finite(l.amount, "a loop's amount")?,
                })
            })
            .transpose()?,
    })
}

fn caption_look(look: &CaptionLook) -> Result<m::CaptionLook, String> {
    let mut title = look.style.doc()?;
    // A caption lane's look has no words of its own: each caption's are
    // the caption's.
    title.content.clear();
    title.runs.clear();
    Ok(m::CaptionLook {
        title,
        offset_y: finite(look.offset_y, "offsetY")?,
    })
}

fn cutout(cutout: &Cutout) -> Result<m::Cutout, String> {
    Ok(m::Cutout {
        mode: match cutout.mode {
            CutoutMode::Auto => m::CutoutMode::Auto,
            CutoutMode::Custom => m::CutoutMode::Custom,
        },
        subject: subject(cutout.subject),
        feather: finite(cutout.feather, "feather")?,
        strokes: cutout
            .strokes
            .iter()
            .map(stroke)
            .collect::<Result<_, _>>()?,
    })
}

fn subject(subject: Subject) -> m::Subject {
    match subject {
        Subject::Auto => m::Subject::Auto,
        Subject::Person => m::Subject::Person,
        Subject::Object => m::Subject::Object,
    }
}

fn stroke(stroke: &Stroke) -> Result<m::Stroke, String> {
    Ok(m::Stroke {
        tool: match stroke.tool {
            BrushTool::SmartBrush => m::BrushTool::SmartBrush,
            BrushTool::Brush => m::BrushTool::Brush,
            BrushTool::SmartEraser => m::BrushTool::SmartEraser,
            BrushTool::Eraser => m::BrushTool::Eraser,
        },
        size: finite(stroke.size, "size")?,
        points: stroke.points.clone(),
        at: stroke.at.map(|at| place(at, "at")).transpose()?,
    })
}

fn moves(moves: &[ClipMove]) -> Result<Vec<doc::ClipMove>, String> {
    moves
        .iter()
        .map(|mv| {
            Ok(doc::ClipMove {
                item: id(&mv.clip_id)?,
                start: floored(mv.start, "start")?,
                lane: mv.track_id.as_deref().map(id).transpose()?,
            })
        })
        .collect()
}

fn speed(value: f64) -> Result<m::Speed, String> {
    doc::speed_from_decimal(finite(value, "speed")?).ok_or_else(|| "a speed is not zero".to_owned())
}

/// The effect at `entry` of an item's picture chain.
fn entry(item: &m::Item, entry: usize) -> Option<m::EffectId> {
    item.effects
        .iter()
        .filter_map(Entry::known)
        .nth(entry)
        .map(|effect| effect.id)
}

/// The patch as the document's.
fn patch(item: &m::Item, model: &Model, patch: &ClipPatch) -> Result<doc::ItemPatch, String> {
    Ok(doc::ItemPatch {
        stream: patch.audio_stream,
        background: patch
            .background
            .as_ref()
            .map(|b| b.as_ref().map(|b| background(model, b)).transpose())
            .transpose()?,
        blend: patch.blend.as_deref().map(|name| match name {
            "multiply" => m::Blend::Multiply,
            "screen" => m::Blend::Screen,
            "add" => m::Blend::Add,
            "lighten" => m::Blend::Lighten,
            "darken" => m::Blend::Darken,
            _ => m::Blend::Normal,
        }),
        crop: patch.crop.map(|crop| {
            crop.map(|c| m::Crop {
                left: c.left,
                top: c.top,
                right: c.right,
                bottom: c.bottom,
            })
        }),
        fade_in: patch
            .fade_in
            .map(|s| floored_distance(s, "fadeIn"))
            .transpose()?,
        fade_out: patch
            .fade_out
            .map(|s| floored_distance(s, "fadeOut"))
            .transpose()?,
        filters: patch
            .filters
            .as_ref()
            .map(|links| effects(item, &item.filters, links))
            .transpose()?,
        effects: patch
            .video_effects
            .as_ref()
            .map(|links| effects(item, &item.effects, links))
            .transpose()?,
        flip_h: patch.flip_h,
        flip_v: patch.flip_v,
        masks: patch
            .masks
            .as_ref()
            .map(|masks| {
                masks
                    .iter()
                    .map(|m| mask(item, m))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?,
        motion: patch
            .motion
            .as_ref()
            .map(|motion_of| match motion_of {
                Some(given) => motion(given),
                None => Ok(m::Motion::default()),
            })
            .transpose()?,
        muted: patch.muted,
        name: patch.name.clone(),
        opacity: patch.opacity.map(|o| finite(o, "opacity")).transpose()?,
        keep_pitch: patch.preserve_pitch,
        shape: patch.shape.as_ref().map(ShapeStyle::doc).transpose()?,
        title: match &patch.text {
            Some(Some(style)) => Some(style.doc()?),
            _ => None,
        },
        cut_in: patch
            .transition_in
            .as_ref()
            .map(|transition| {
                transition
                    .as_ref()
                    .map(|t| -> Result<m::CutIn, String> {
                        let mut effect = m::Effect::new(placeholder(), t.id.clone());
                        for (name, value) in &t.params {
                            effect
                                .params
                                .insert(name.clone(), m::Param::fixed(Some(finite(*value, name)?)));
                        }
                        Ok(m::CutIn {
                            effect,
                            len: length(t.duration, "duration")?,
                        })
                    })
                    .transpose()
            })
            .transpose()?,
        volume: patch
            .volume
            .map(|v| finite(v, "volume").map(|v| v.max(0.0)))
            .transpose()?,
    })
}

/// A wire command as the document's.
///
/// Times in seconds become ticks, a key's share of its clip a tick of the
/// clip's content time, string ids typed ids, a chain's place an effect's
/// id. What names something the model has not got is refused here only
/// where the translation needs it (a key's clip); otherwise the document's
/// own command says.
pub fn to_doc(model: &Model, command: &Command) -> Result<doc::Command, String> {
    use Command as W;
    use doc::Command as D;
    Ok(match command {
        W::AddMedia { item } => D::AddMedia { media: item.doc()? },
        W::RemoveMedia { media_id } => D::RemoveMedia {
            media: id(media_id)?,
        },
        W::SetMediaPlaceholder {
            media_id,
            placeholder,
        } => D::SetMediaPlaceholder {
            media: id(media_id)?,
            placeholder: *placeholder,
        },
        W::FillSlot { media_id, item } => D::FillSlot {
            media: id(media_id)?,
            file: item.doc()?,
        },
        W::Batch { commands } => D::Batch {
            commands: commands
                .iter()
                .map(|command| to_doc(model, command))
                .collect::<Result<_, _>>()?,
        },
        W::AddClip {
            media_id,
            track_id,
            start,
            ripple,
        } => D::AddClip {
            media: id(media_id)?,
            lane: id(track_id)?,
            start: floored(start, "start")?,
            ripple: *ripple,
        },
        W::AddClipAtFirstFree { media_id, start } => D::AddClipAtFirstFree {
            media: id(media_id)?,
            start: floored(start, "start")?,
        },
        W::AddClipsInSequence {
            media_ids,
            start,
            track_id,
            ripple,
        } => D::AddClipsInSequence {
            media: ids(media_ids)?,
            start: floored(start, "start")?,
            lane: track_id.as_deref().map(id).transpose()?,
            ripple: *ripple,
        },
        W::AddTextClip {
            start,
            duration,
            track_id,
            above,
            offset_y,
            style,
        } => D::AddTextClip {
            start: floored(start, "start")?,
            len: optional_length(*duration, "duration")?,
            lane: track_id.as_deref().map(id).transpose()?,
            above: *above,
            offset_y: offset_y.map(|y| finite(y, "offsetY")).transpose()?,
            title: style.as_ref().map(TextStyle::doc).transpose()?,
        },
        W::AddShapeClip {
            start,
            name,
            duration,
            track_id,
            above,
            style,
        } => D::AddShapeClip {
            start: floored(start, "start")?,
            len: optional_length(*duration, "duration")?,
            name: name.clone(),
            style: style.as_ref().map(ShapeStyle::doc).transpose()?,
            lane: track_id.as_deref().map(id).transpose()?,
            above: *above,
        },
        W::PlaceCaptions {
            look,
            cues,
            track_id,
        } => D::PlaceCaptions {
            cues: cues
                .iter()
                .map(|cue| {
                    Ok(doc::Cue {
                        start: floored(cue.start, "a cue's start")?,
                        len: length(cue.duration, "a cue's duration")?,
                        words: cue.text.clone(),
                    })
                })
                .collect::<Result<_, String>>()?,
            look: caption_look(look)?,
            lane: track_id.as_deref().map(id).transpose()?,
        },
        W::SetCaptionLook { track_id, look } => D::SetCaptionLook {
            lane: id(track_id)?,
            look: look.as_ref().map(caption_look).transpose()?,
        },
        W::AddLayerClip {
            start,
            effect_id,
            name,
            duration,
            sound,
            track_id,
        } => D::AddLayerClip {
            start: floored(start, "start")?,
            len: optional_length(*duration, "duration")?,
            package: effect_id.clone(),
            name: name.clone(),
            sound: *sound,
            lane: track_id.as_deref().map(id).transpose()?,
        },
        W::AddTransitionClip {
            start,
            transition_id,
            name,
            duration,
            track_id,
        } => D::AddTransitionClip {
            start: floored(start, "start")?,
            len: optional_length(*duration, "duration")?,
            package: transition_id.clone(),
            name: name.clone(),
            lane: track_id.as_deref().map(id).transpose()?,
        },
        W::SetClipSpeedCurve {
            clip_id,
            curve,
            ripple,
        } => D::SetClipSpeedCurve {
            item: id(clip_id)?,
            curve: curve.as_ref().map(|points| {
                points
                    .iter()
                    .map(|p| m::SpeedPoint {
                        at: p.at,
                        speed: p.speed,
                        slope: p.slope,
                    })
                    .collect()
            }),
            ripple: *ripple,
        },
        W::SetClipKey {
            clip_id,
            property,
            at,
            value,
            ease: e,
        } => {
            let (item_id, it) = item(model, clip_id)?;
            D::SetClipKey {
                item: item_id,
                property: property.doc(),
                at: share_at(it, *at)?,
                value: finite(*value, "value")?,
                ease: ease(*e),
            }
        }
        W::ClearClipKey {
            clip_id,
            property,
            at,
        } => {
            let (item_id, it) = item(model, clip_id)?;
            D::ClearClipKey {
                item: item_id,
                property: property.doc(),
                at: share_at(it, *at)?,
            }
        }
        W::ClearClipKeys { clip_id, property } => D::ClearClipKeys {
            item: id(clip_id)?,
            property: property.doc(),
        },
        W::SetEffectKey {
            clip_id,
            entry: at_entry,
            key,
            at,
            value,
            ease: e,
        } => {
            let (item_id, it) = item(model, clip_id)?;
            let effect = entry(it, *at_entry)
                .ok_or_else(|| format!("clip {clip_id} has no effect {at_entry}"))?;
            D::SetEffectKey {
                item: item_id,
                effect,
                param: key.clone(),
                at: share_at(it, *at)?,
                value: finite(*value, "value")?,
                ease: ease(*e),
            }
        }
        W::ClearEffectKey {
            clip_id,
            entry: at_entry,
            key,
            at,
        } => {
            let (item_id, it) = item(model, clip_id)?;
            let effect = entry(it, *at_entry)
                .ok_or_else(|| format!("clip {clip_id} has no effect {at_entry}"))?;
            D::ClearEffectKey {
                item: item_id,
                effect,
                param: key.clone(),
                at: share_at(it, *at)?,
            }
        }
        W::ClearEffectKeys {
            clip_id,
            entry: at_entry,
            key,
        } => {
            let (item_id, it) = item(model, clip_id)?;
            let effect = entry(it, *at_entry)
                .ok_or_else(|| format!("clip {clip_id} has no effect {at_entry}"))?;
            D::ClearEffectKeys {
                item: item_id,
                effect,
                param: key.clone(),
            }
        }
        W::SetMaskKey {
            clip_id,
            mask_id,
            number,
            at,
            value,
            ease: e,
        } => {
            let (item_id, it) = item(model, clip_id)?;
            D::SetMaskKey {
                item: item_id,
                mask: id(mask_id)?,
                number: mask_number(number)?,
                at: share_at(it, *at)?,
                value: finite(*value, "value")?,
                ease: ease(*e),
            }
        }
        W::ClearMaskKey {
            clip_id,
            mask_id,
            number,
            at,
        } => {
            let (item_id, it) = item(model, clip_id)?;
            D::ClearMaskKey {
                item: item_id,
                mask: id(mask_id)?,
                number: mask_number(number)?,
                at: share_at(it, *at)?,
            }
        }
        W::ClearMaskKeys {
            clip_id,
            mask_id,
            number,
        } => D::ClearMaskKeys {
            item: id(clip_id)?,
            mask: id(mask_id)?,
            number: mask_number(number)?,
        },
        W::SetClipCutout { clip_id, cutout: c } => D::SetClipCutout {
            item: id(clip_id)?,
            cutout: c.as_ref().map(cutout).transpose()?,
        },
        W::SetClipStabilize { clip_id, stabilize } => D::SetClipStabilize {
            item: id(clip_id)?,
            stabilize: stabilize.as_ref().map(|s| m::Stabilize {
                smoothness: s.smoothness,
                max_zoom: s.max_zoom,
                keep_rotation: s.keep_rotation,
                tripod: s.tripod,
            }),
        },
        W::SetClipVoice { clip_id, voice } => D::SetClipVoice {
            item: id(clip_id)?,
            voice: voice.as_ref().map(|v| m::VoiceMix {
                kind: match v.kind {
                    VoiceKind::Vocals => m::VoiceKind::Vocals,
                    VoiceKind::Speech => m::VoiceKind::Speech,
                },
                voice: v.voice,
                background: v.background,
            }),
        },
        W::AddCutoutStroke { clip_id, stroke: s } => D::AddCutoutStroke {
            item: id(clip_id)?,
            stroke: stroke(s)?,
        },
        W::MoveClips { moves: m } => D::MoveClips { moves: moves(m)? },
        W::MoveClipsToNewTrack { moves: m, bottom } => D::MoveClipsToNewTrack {
            moves: moves(m)?,
            bottom: *bottom,
        },
        W::TrimClip {
            clip_id,
            edge,
            delta,
            ripple,
        } => D::TrimClip {
            item: id(clip_id)?,
            edge: match edge {
                TrimEdge::Start => doc::Edge::Head,
                TrimEdge::End => doc::Edge::Tail,
            },
            delta: distance(*delta, "delta")?,
            ripple: *ripple,
        },
        W::SplitClips { clip_ids, time } => D::SplitClips {
            items: ids(clip_ids)?,
            at: place(*time, "time")?,
        },
        W::DuplicateClips { clip_ids } => D::DuplicateClips {
            items: ids(clip_ids)?,
        },
        W::ReplaceClipMedia {
            clip_id,
            item,
            enhanced,
            offset,
            source_start,
        } => D::ReplaceClipMedia {
            item: id(clip_id)?,
            file: item.doc()?,
            enhanced: *enhanced,
            copy_start: floored_distance(offset, "offset")?,
            source_start: source_start
                .map(|s| floored(s, "sourceStart"))
                .transpose()?,
        },
        W::SetClipEnhanced { clip_id, on } => D::SetClipEnhanced {
            item: id(clip_id)?,
            on: *on,
        },
        W::FreezeFrame {
            clip_id,
            time,
            duration,
            still,
        } => D::FreezeFrame {
            item: id(clip_id)?,
            at: place(*time, "time")?,
            len: optional_length(*duration, "duration")?,
            still: still.as_ref().map(NewMedia::doc).transpose()?,
        },
        W::MergeClips { clip_ids } => D::MergeClips {
            items: ids(clip_ids)?,
        },
        W::ConsolidateClips { clip_ids, item } => D::ConsolidateClips {
            items: ids(clip_ids)?,
            file: item.doc()?,
        },
        W::RemoveClips { clip_ids, ripple } => D::RemoveClips {
            items: ids(clip_ids)?,
            ripple: *ripple,
        },
        W::UpdateClip { clip_id, patch: p } => {
            let (item_id, it) = item(model, clip_id)?;
            D::UpdateClip {
                item: item_id,
                patch: patch(it, model, p)?,
            }
        }
        W::StyleTextRange {
            clip_id,
            start,
            end,
            patch: p,
        } => D::StyleTextRange {
            item: id(clip_id)?,
            start: *start,
            end: *end,
            style: doc::RunStyle {
                color: p.color.as_deref().map(colour).transpose()?,
                font_scale: p.font_scale,
                font_weight: p.font_weight,
                italic: p.italic,
                underline: p.underline,
                strikethrough: p.strikethrough,
                stroke_color: p.stroke_color.as_deref().map(colour).transpose()?,
            },
        },
        W::ClearTextStyles {
            clip_id,
            start,
            end,
        } => D::ClearTextStyles {
            item: id(clip_id)?,
            start: *start,
            end: *end,
        },
        W::SetClipSpeed {
            clip_id,
            speed: s,
            ripple,
        } => D::SetClipSpeed {
            item: id(clip_id)?,
            speed: speed(*s)?,
            ripple: *ripple,
        },
        W::SetClipTransform {
            clip_id,
            offset_x,
            offset_y,
            rotation,
            scale,
            stretch_x,
            stretch_y,
        } => {
            let f = |v: &Option<f64>, what| v.map(|v| finite(v, what)).transpose();
            D::SetClipTransform {
                item: id(clip_id)?,
                offset_x: f(offset_x, "offsetX")?,
                offset_y: f(offset_y, "offsetY")?,
                rotation: f(rotation, "rotation")?,
                scale: f(scale, "scale")?,
                stretch_x: f(stretch_x, "stretchX")?,
                stretch_y: f(stretch_y, "stretchY")?,
            }
        }
        W::FrameClip {
            clip_id,
            framing,
            at,
        } => {
            let (item_id, it) = item(model, clip_id)?;
            D::FrameClip {
                item: item_id,
                framing: match framing {
                    Framing::Fit => doc::Framing::Fit,
                    Framing::Fill => doc::Framing::Fill,
                },
                at: share_at(it, *at)?,
            }
        }
        W::DetachAudio { clip_id } => D::DetachAudio { item: id(clip_id)? },
        W::ReattachAudio { clip_id } => D::ReattachAudio { item: id(clip_id)? },
        W::AddTrack { bottom } => D::AddTrack { bottom: *bottom },
        W::RemoveTrack { track_id } => D::RemoveTrack {
            lane: id(track_id)?,
        },
        W::SetTrackFlag {
            track_id,
            flag,
            value,
        } => D::SetTrackFlag {
            lane: id(track_id)?,
            flag: match flag {
                TrackFlag::Visible => doc::LaneFlag::Visible,
                TrackFlag::Muted => doc::LaneFlag::Muted,
            },
            value: *value,
        },
        W::AddTimeline => D::AddTimeline,
        W::RemoveTimeline { timeline_id } => D::RemoveTimeline {
            sequence: id(timeline_id)?,
        },
        W::SetTimelineVideo { timeline_id, video } => D::SetTimelineVideo {
            sequence: id(timeline_id)?,
            frame: video.doc()?,
        },
        W::SetTimelineBackground {
            timeline_id,
            background: b,
        } => D::SetTimelineBackground {
            sequence: id(timeline_id)?,
            background: background(model, b)?,
        },
        W::SetTimelineLinkage { timeline_id, on } => D::SetTimelineLinkage {
            sequence: id(timeline_id)?,
            on: *on,
        },
        W::RenameTimeline { timeline_id, name } => D::RenameTimeline {
            sequence: id(timeline_id)?,
            name: name.clone(),
        },
        W::SelectTimeline { timeline_id } => D::SelectTimeline {
            sequence: id(timeline_id)?,
        },
        W::MoveTimeline { timeline_id, index } => D::MoveTimeline {
            sequence: id(timeline_id)?,
            index: *index,
        },
        W::SetCover { cover } => D::SetCover {
            cover: match cover {
                None => None,
                Some(Cover::Frame { time }) => Some(m::Cover::Frame {
                    into: floored_distance(time, "time")?,
                }),
                Some(Cover::Picture { path }) => {
                    // The picture is a file of the bin; the API adds it
                    // first (see `Api::apply`).
                    let media = model
                        .media
                        .iter()
                        .find(|(_, media)| media.path == *path)
                        .map(|(media_id, _)| media_id)
                        .ok_or_else(|| format!("{path} is not in the bin"))?;
                    Some(m::Cover::Picture { media })
                }
            },
        },
        W::AddFont { family, path } => D::AddFont {
            family: family.clone(),
            path: path.clone(),
        },
        W::RemoveFont { family } => D::RemoveFont {
            family: family.clone(),
        },
        W::UpdateMediaPath { media_id, new_path } => D::UpdateMediaPath {
            media: id(media_id)?,
            path: new_path.clone(),
        },
        W::SetMediaColorRange { media_id, range } => D::SetMediaColorRange {
            media: id(media_id)?,
            range: range.map(|range| match range {
                ColorRange::Limited => m::ColorRange::Limited,
                ColorRange::Full => m::ColorRange::Full,
            }),
        },
        W::SetMediaBeats { media_id, marks } => D::SetMediaBeats {
            media: id(media_id)?,
            marks: marks.map(|marks| match marks {
                BeatMarks::Zoom => m::BeatMarks::Zoom,
                BeatMarks::Every => m::BeatMarks::Every,
                BeatMarks::Second => m::BeatMarks::Second,
                BeatMarks::Bar => m::BeatMarks::Bar,
            }),
        },
    })
}

/// Whether a typed id is of the kind `I`, for a caller that wants to say
/// what a created id is.
pub fn is<I: TypedId>(any: concat_doc::AnyId) -> bool {
    any.typed::<I>().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_reads_as_the_contract_spells_it() {
        let command: Command = serde_json::from_value(serde_json::json!({
            "op": "addTextClip", "start": 1.5, "duration": 2.0,
            "style": { "content": "Hello" }
        }))
        .expect("reads");
        let document = concat_doc::Document::<Model>::new();
        let doc::Command::AddTextClip {
            start, len, title, ..
        } = to_doc(&document, &command).expect("translates")
        else {
            panic!("a title");
        };
        assert_eq!(start.display_seconds(), 1.5);
        assert_eq!(len.map(|l| TickDur::from(l).display_seconds()), Some(2.0));
        let title = title.expect("a look");
        assert_eq!(title.content, "Hello");
        assert_eq!(title.font_family, m::Title::default().font_family);
    }

    #[test]
    fn a_double_optional_field_tells_null_from_absent() {
        let patch: ClipPatch =
            serde_json::from_value(serde_json::json!({ "crop": null })).expect("reads");
        assert_eq!(patch.crop, Some(None));
        let patch: ClipPatch = serde_json::from_value(serde_json::json!({})).expect("reads");
        assert_eq!(patch.crop, None);
    }

    #[test]
    fn a_number_that_is_not_one_is_refused_with_its_name() {
        let document = concat_doc::Document::<Model>::new();
        let command = Command::AddClipAtFirstFree {
            media_id: "m1".to_owned(),
            start: f64::NAN,
        };
        let error = to_doc(&document, &command).expect_err("refused");
        assert!(error.contains("start"), "{error}");
    }
}
