// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Sequences and their lanes, markers, the document's fonts and cover,
//! the view and what is held unread.

use std::num::NonZeroU32;

use crate::document::Weigh;
use crate::time::{Rate, Tick, TickDur};

use super::ids::{Entry, MarkerId, MediaId, Raw, Residual, SequenceId};
use super::look::CaptionLook;
use super::media::ColorSpace;
use super::order::OrderKey;

/// A timeline: the frame it is drawn to, the rate it runs at, and what
/// fills the frame behind its pictures. Its lanes and their items are
/// in their own tables and name it.
///
/// Every sequence is one entry of the document's table, whether it is
/// shown as a tab or placed inside another as an item.
#[derive(Clone, PartialEq, Debug)]
pub struct Sequence {
    /// What its tab is called. Never blank.
    pub name: String,
    /// The frame and the rate.
    pub frame: Frame,
    /// What fills the frame behind the pictures of its lowest picture
    /// lane, where they do not reach.
    pub background: Background,
    /// Whether the other lanes follow an edit that closes up or opens
    /// the main lane, so what was placed against the footage stays on
    /// the frames it was placed against.
    pub linkage: bool,
    /// Markers on its timeline.
    pub markers: Vec<Entry<Marker>>,
    /// Fields this build does not know.
    pub residual: Residual,
}

impl Sequence {
    /// A sequence of this name and frame, black behind, linked, with no
    /// markers.
    pub fn new(name: impl Into<String>, frame: Frame) -> Sequence {
        Sequence {
            name: name.into(),
            frame,
            background: Background::None,
            linkage: true,
            markers: Vec::new(),
            residual: Residual::default(),
        }
    }
}

/// The frame a sequence is drawn to, and the rate it runs at.
#[derive(Clone, PartialEq, Debug)]
pub struct Frame {
    /// Pixels across.
    pub width: NonZeroU32,
    /// Pixels down.
    pub height: NonZeroU32,
    /// Frames a second.
    pub rate: Rate,
    /// What shows where no picture covers the frame; alpha below 255
    /// lets it show through.
    pub canvas: Color,
    /// The colour it is output in.
    pub color: ColorSpace,
}

impl Frame {
    /// 1920 × 1080 at 30 a second, black, SDR.
    pub fn hd() -> Frame {
        Frame {
            width: NonZeroU32::new(1920).unwrap_or(NonZeroU32::MIN),
            height: NonZeroU32::new(1080).unwrap_or(NonZeroU32::MIN),
            rate: Rate::FPS_30,
            canvas: Color::BLACK,
            color: ColorSpace::Sdr,
        }
    }
}

/// A colour with its opacity, eight bits each, not premultiplied.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Color {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// How much it covers: 255 is opaque.
    pub a: u8,
}

impl Color {
    /// Opaque black.
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    /// Opaque white.
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
}

/// What fills the frame where the picture does not reach.
#[derive(Clone, PartialEq, Debug)]
pub enum Background {
    /// Black: nothing is drawn.
    None,
    /// The item's own frame, enlarged to cover and blurred. `amount` is
    /// from 0 to 1.
    Blur {
        /// How soft.
        amount: f64,
    },
    /// One colour.
    Color(Color),
    /// A still from the bin, enlarged to cover.
    Image(MediaId),
    /// A background this build does not know.
    Unknown(Raw),
}

impl Background {
    /// The file it shows, if it shows one.
    pub fn media(&self) -> Option<MediaId> {
        match self {
            Background::Image(media) => Some(*media),
            _ => None,
        }
    }
}

/// A lane: one layer of a sequence's stack. Any kind of item goes on
/// any lane, a caption lane included.
#[derive(Clone, PartialEq, Debug)]
pub struct Lane {
    /// The sequence it is in. A lane stays in the sequence it was made
    /// in.
    pub sequence: SequenceId,
    /// Where it is in the stack: a lane with a lower key is beneath one
    /// with a higher. No two lanes of a sequence share a key.
    pub order: OrderKey,
    /// Whether its pictures are drawn.
    pub visible: bool,
    /// Whether its sound is silent.
    pub muted: bool,
    /// A caption lane's look, which its captions are placed in and
    /// restyled to as one edit; none for any other lane.
    pub captions: Option<CaptionLook>,
    /// Fields this build does not know.
    pub residual: Residual,
}

impl Lane {
    /// A visible, sounding lane of a sequence, at a place in its stack,
    /// not a caption lane.
    pub fn new(sequence: SequenceId, order: OrderKey) -> Lane {
        Lane {
            sequence,
            order,
            visible: true,
            muted: false,
            captions: None,
            residual: Residual::default(),
        }
    }
}

/// A marked place or stretch: on a sequence's timeline, or on an item's
/// content time.
#[derive(Clone, PartialEq, Debug)]
pub struct Marker {
    /// Its id: what a command names it by.
    pub id: MarkerId,
    /// Where it is.
    pub at: Tick,
    /// How long it is; zero for a single place. Never below zero.
    pub len: TickDur,
    /// What it says.
    pub name: String,
    /// Its colour, or the window's own.
    pub color: Option<Color>,
    /// Fields this build does not know.
    pub residual: Residual,
}

/// A font the person added from disk, for every title to use.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Font {
    /// The family name titles refer to it by.
    pub family: String,
    /// Where the file is. No two fonts share one.
    pub path: String,
}

/// What a project is shown by: its card among the projects, the square
/// at the head of the phone's timeline.
///
/// Choosing it is an edit like any other, and a step of undo; which
/// sequence is showing is not, so a frame is of the one showing when the
/// cover is drawn, and follows the edit.
#[derive(Clone, PartialEq, Debug)]
pub enum Cover {
    /// A frame of the sequence showing, as it renders, titles and all.
    Frame {
        /// How far into the sequence; not below zero.
        into: TickDur,
    },
    /// A still from the bin, fitted to the frame and cut at its edges.
    /// A file of the bin and not a path, so it travels with the project
    /// when the project is moved or packed as a template.
    Picture {
        /// The still. Not a video or a recording, and not taken out of
        /// the bin while it is the cover.
        media: MediaId,
    },
    /// A cover this build does not know.
    Unknown(Raw),
}

/// What is showing: view state, saved with the document and never a
/// step of undo.
///
/// Undo and redo keep the view as it is now wherever the document they
/// restore can hold it: see [`Model`](super::Model)'s `carry_view`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct View {
    /// The sequence that is showing, which commands act on. Set whenever
    /// the document has a sequence.
    pub showing: Option<SequenceId>,
    /// The sequences open as tabs, in the order of the tabs. Each once.
    pub tabs: Vec<SequenceId>,
}

/// An entry of a file this build could not read at all, or that broke a
/// rule when it was read: kept as its text and written back.
#[derive(Clone, PartialEq, Debug)]
pub struct Held {
    /// Where in the file it was: the name of the list it came from, and
    /// the id of what that list belongs to, as the file wrote them.
    pub place: String,
    /// Its text.
    pub raw: Raw,
    /// Why it could not be taken in, for the report the person reads.
    pub why: String,
}

impl Weigh for Sequence {
    fn weight(&self) -> usize {
        std::mem::size_of::<Sequence>()
            + self.name.len()
            + self.markers.len() * (std::mem::size_of::<Marker>() + 16)
            + self.residual.weight()
    }
}

impl Weigh for Lane {
    fn weight(&self) -> usize {
        let look = self.captions.as_ref().map_or(0, |look| {
            std::mem::size_of::<CaptionLook>() + look.title.font_family.len()
        });
        std::mem::size_of::<Lane>() + self.order.as_str().len() + look + self.residual.weight()
    }
}

impl Weigh for Held {
    fn weight(&self) -> usize {
        std::mem::size_of::<Held>() + self.place.len() + self.raw.0.len() + self.why.len()
    }
}
