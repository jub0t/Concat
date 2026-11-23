// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The bin: the files a project's items are cut from.

use std::num::NonZeroU32;

use crate::document::Weigh;
use crate::time::{Rate, TickLen};

use super::ids::{Raw, Residual};

/// One file in the bin, and what was learned about it when it was
/// imported.
///
/// What the probe found is kept rather than found again, so a project
/// opens and means the same with the file gone: an item of a missing
/// file is offline, never deleted.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Media {
    /// Where the file is on the person's disk. Empty for a template's
    /// slot packed into a bundle.
    pub path: String,
    /// What the bin calls it, normally the file's name.
    pub name: String,
    /// What the file is.
    pub kind: MediaKind,
    /// How long the file plays, when its container said. A still has no
    /// length of its own.
    pub length: Option<TickLen>,
    /// The picture's frame rate, exactly, when the file has moving
    /// pictures and the probe found one.
    pub rate: Option<Rate>,
    /// The picture's size in pixels, when there is a picture and the
    /// probe found it.
    pub size: Option<Size>,
    /// Whether the file has sound.
    pub has_sound: bool,
    /// Every sound stream, in the file's order, when the probe listed
    /// them. A file that lists none may still have sound: one from before
    /// the list was kept.
    pub streams: Vec<SoundStream>,
    /// The picture's codec as the probe named it. For people only.
    pub video_codec: Option<String>,
    /// The sound's codec, the same.
    pub audio_codec: Option<String>,
    /// What the picture was recorded in.
    pub color: ColorSpace,
    /// The levels the picture is read as, over what the file says; none
    /// reads the file's own tag.
    pub range: Option<ColorRange>,
    /// Which of the file's beats are marked along its items; none when
    /// nobody asked for its beats.
    pub beats: Option<BeatMarks>,
    /// What made the file, when it was the editor and not an import.
    pub origin: Option<MediaOrigin>,
    /// Whether this is a template's slot: a stand-in that says what kind
    /// of file belongs here, for the person's own to replace.
    pub placeholder: bool,
    /// Fields this build does not know.
    pub residual: Residual,
}

/// A picture's size in pixels; neither side is zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Size {
    /// Across.
    pub width: NonZeroU32,
    /// Down.
    pub height: NonZeroU32,
}

impl Size {
    /// A size, if neither side is zero.
    pub fn new(width: u32, height: u32) -> Option<Size> {
        Some(Size {
            width: NonZeroU32::new(width)?,
            height: NonZeroU32::new(height)?,
        })
    }
}

/// One sound stream of a file, as the probe reported it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SoundStream {
    /// The stream's index within the file: what an item names to play
    /// it. No two streams of a file share one.
    pub index: u32,
    /// How many channels: 1 is mono, 2 stereo. Zero when not reported.
    pub channels: u32,
    /// Samples a second; zero when not reported.
    pub sample_rate: u32,
    /// The codec's short name. For people only.
    pub codec: String,
    /// The language tag; empty when the file does not say.
    pub language: String,
    /// The name the file gives the stream; empty when it gives none.
    pub title: String,
}

/// What a file is.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum MediaKind {
    /// Moving pictures, with or without sound.
    #[default]
    Video,
    /// Sound alone.
    Audio,
    /// A still, whose length on a timeline is the editor's choice.
    Image,
    /// A kind this build does not know.
    Unknown(Raw),
}

impl MediaKind {
    /// Whether an item can show a picture of it. `None` for a kind this
    /// build does not know.
    pub fn has_picture(&self) -> Option<bool> {
        match self {
            MediaKind::Video | MediaKind::Image => Some(true),
            MediaKind::Audio => Some(false),
            MediaKind::Unknown(_) => None,
        }
    }

    /// Whether an item can play its sound alone. `None` for a kind this
    /// build does not know.
    pub fn can_sound(&self) -> Option<bool> {
        match self {
            MediaKind::Video | MediaKind::Audio => Some(true),
            MediaKind::Image => Some(false),
            MediaKind::Unknown(_) => None,
        }
    }
}

/// The colour a picture is in.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum ColorSpace {
    /// Rec. 709, standard range of light.
    #[default]
    Sdr,
    /// Rec. 2020 with hybrid log-gamma.
    Hlg,
    /// Rec. 2020 with the perceptual quantiser.
    Pq,
    /// A space this build does not know.
    Unknown(Raw),
}

/// The levels a picture spans, when the person says so over the file.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ColorRange {
    /// 16-235.
    Limited,
    /// 0-255.
    Full,
    /// A range this build does not know.
    Unknown(Raw),
}

/// Which beats of a file are marked along its items.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum BeatMarks {
    /// As many as the zoom has room for.
    Zoom,
    /// Every beat.
    Every,
    /// Every second beat, from each bar's first.
    Second,
    /// The first beat of each bar.
    Bar,
    /// A choice this build does not know.
    Unknown(Raw),
}

/// What made a file, when the editor did.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MediaOrigin {
    /// Read aloud by the speech sheet.
    Speech,
    /// An item's sound rendered as it played.
    Processed,
    /// An origin this build does not know.
    Unknown(Raw),
}

impl Weigh for Media {
    fn weight(&self) -> usize {
        let streams: usize = self
            .streams
            .iter()
            .map(|s| 96 + s.codec.len() + s.language.len() + s.title.len())
            .sum();
        std::mem::size_of::<Media>()
            + self.path.len()
            + self.name.len()
            + streams
            + self.residual.weight()
    }
}
