// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What is placed on a lane.
//!
//! Every item has the same frame: a lane, a place, a length, a window on
//! its content time, a picture side, a sound side, its chains, masks and
//! markers. What it shows is its [`Content`]: a cut of a file, a nested
//! sequence, a title, a shape, a layer that treats what is beneath it,
//! or a transition. A field that a kind does not read (a title's level)
//! is kept and does nothing, as the published API has it.

use crate::document::Weigh;
use crate::id::{AnyId, TypedId};
use crate::time::{Tick, TickDur, TickLen};

use super::animated::Animated;
use super::effect::{Effect, Mask};
use super::ids::{Entry, ItemId, LaneId, MediaId, Raw, Residual, SequenceId, known};
use super::look::{ShapeStyle, Title};
use super::sequence::{Background, Marker};
use super::timing::TimeMap;

/// One thing placed on a lane.
#[derive(Clone, PartialEq, Debug)]
pub struct Item {
    /// The lane it is on.
    pub lane: LaneId,
    /// Where it starts on the lane.
    pub start: Tick,
    /// How long it is on the lane.
    pub len: TickLen,
    /// Where on its content time the window it shows begins: it shows
    /// content `[offset, offset + len)`.
    pub offset: Tick,
    /// What it shows.
    pub content: Content,
    /// What the lane calls it.
    pub name: String,
    /// How its picture is placed and treated.
    pub picture: Picture,
    /// How its sound is mixed.
    pub sound: Sound,
    /// Picture effects, in the order they apply. On a layer, the chain
    /// it runs over everything beneath it.
    pub effects: Vec<Entry<Effect>>,
    /// Sound filters, in order. On a sound layer, the chain it runs over
    /// the mix.
    pub filters: Vec<Entry<Effect>>,
    /// The shapes that keep part of its picture, top first.
    pub masks: Vec<Entry<Mask>>,
    /// Markers on its content time.
    pub markers: Vec<Entry<Marker>>,
    /// A transition on the cut into it, if any.
    pub cut_in: Option<CutIn>,
    /// What it is tied to, if anything.
    pub link: Option<Link>,
    /// Fields this build does not know.
    pub residual: Residual,
}

impl Item {
    /// An item of this content at a place, showing its content from
    /// zero, with every other setting at its default.
    pub fn new(lane: LaneId, start: Tick, len: TickLen, content: Content) -> Item {
        Item {
            lane,
            start,
            len,
            offset: Tick::ZERO,
            content,
            name: String::new(),
            picture: Picture::default(),
            sound: Sound::default(),
            effects: Vec::new(),
            filters: Vec::new(),
            masks: Vec::new(),
            markers: Vec::new(),
            cut_in: None,
            link: None,
            residual: Residual::default(),
        }
    }

    /// Where it ends on the lane, if that fits in a tick.
    pub fn end(&self) -> Option<Tick> {
        self.start.checked_add(self.len)
    }

    /// Where the window it shows ends on its content time, if that fits.
    pub fn window_end(&self) -> Option<Tick> {
        self.offset.checked_add(self.len)
    }

    /// Whether this is a transition placed as an item.
    pub fn is_transition(&self) -> bool {
        matches!(self.content, Content::Transition(_))
    }

    /// Every file it names: what it is cut from, an enhanced copy of
    /// that, and a still behind it.
    pub fn media(&self) -> impl Iterator<Item = MediaId> + '_ {
        let (cut, copy) = match &self.content {
            Content::Media(cut) => (Some(cut.media), cut.enhanced.as_ref().map(|e| e.copy)),
            _ => (None, None),
        };
        let behind = self.picture.background.as_ref().and_then(Background::media);
        cut.into_iter().chain(copy).chain(behind)
    }

    /// The sequence it nests, if it nests one.
    pub fn nests(&self) -> Option<SequenceId> {
        match &self.content {
            Content::Nest(nest) => Some(nest.sequence),
            _ => None,
        }
    }

    /// The ids of everything it holds that has one of its own: its
    /// effects and filters, the package of the transition it is or has on
    /// its cut, its masks and its markers.
    pub fn nested_ids(&self) -> Vec<AnyId> {
        let mut ids = Vec::new();
        let chains = known(&self.effects).chain(known(&self.filters));
        ids.extend(chains.map(|effect| effect.id.any()));
        if let Content::Transition(effect) = &self.content {
            ids.push(effect.id.any());
        }
        if let Some(cut) = &self.cut_in {
            ids.push(cut.effect.id.any());
        }
        ids.extend(known(&self.masks).map(|mask| mask.id.any()));
        ids.extend(known(&self.markers).map(|marker| marker.id.any()));
        ids
    }
}

/// What an item shows.
#[derive(Clone, PartialEq, Debug)]
pub enum Content {
    /// A cut of a file in the bin.
    Media(MediaCut),
    /// Another sequence, played as one item.
    Nest(Nest),
    /// A title: words with no file behind them.
    Title(Title),
    /// A figure drawn over the picture.
    Shape(ShapeStyle),
    /// A treatment of everything beneath it for as long as it runs: its
    /// chain is the item's effects, its strength the item's opacity.
    Layer,
    /// A treatment of the sound for as long as it runs: its chain is the
    /// item's filters, working on the mix of everything audible, and the
    /// item's opacity is how much of the treated sound replaces the
    /// rest.
    SoundLayer,
    /// A transition placed as an item of its own, acting on what is
    /// beneath it for its span: across a cut under it, from the outgoing
    /// picture into the incoming one; over one shot, the package played
    /// over that shot. The effect is its package and its knobs.
    Transition(Effect),
    /// A kind of item this build does not know: its name, and the text of
    /// everything in it this build could not place.
    Unknown {
        /// The kind's name as the file wrote it.
        kind: String,
        /// The rest of it.
        raw: Raw,
    },
}

/// A cut of a file.
#[derive(Clone, PartialEq, Debug)]
pub struct MediaCut {
    /// The file. With an enhanced copy, the original.
    pub media: MediaId,
    /// What of it the item plays.
    pub take: Take,
    /// Which sound stream it plays, by the stream's index in the file;
    /// none is the first.
    pub stream: Option<u32>,
    /// From content time to the file's time.
    pub time: TimeMap,
    /// The item's Enhance, once it has a copy.
    pub enhanced: Option<Enhanced>,
}

impl MediaCut {
    /// The whole of a file, from its start, at its own pace.
    pub fn whole(media: MediaId) -> MediaCut {
        MediaCut {
            media,
            take: Take::Whole,
            stream: None,
            time: TimeMap::default(),
            enhanced: None,
        }
    }
}

/// What of its file an item plays.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Take {
    /// Everything the file has: the picture of a video or a still, the
    /// sound of a video or a recording.
    Whole,
    /// The sound alone, of a video or a recording: what detaching a
    /// video's sound makes.
    Sound,
    /// A take this build does not know.
    Unknown(Raw),
}

/// A clip's Enhance: an enhanced copy of its file, and which of the two
/// it shows.
///
/// The cut's times stay in the original's: while the copy shows, a
/// place `t` of the original is `t - copy_start` of the copy. So
/// switching between the two moves nothing.
#[derive(Clone, PartialEq, Debug)]
pub struct Enhanced {
    /// The file holding the copy. Not the original.
    pub copy: MediaId,
    /// Where in the original the copy starts; not below zero.
    pub copy_start: TickDur,
    /// Whether the copy shows.
    pub on: bool,
}

/// Another sequence, placed as an item.
#[derive(Clone, PartialEq, Debug)]
pub struct Nest {
    /// The sequence. Never one that holds, at any depth, the sequence
    /// this item is in.
    pub sequence: SequenceId,
    /// From content time to the nested sequence's time.
    pub time: TimeMap,
}

/// How an item's picture is placed and treated.
#[derive(Clone, PartialEq, Debug)]
pub struct Picture {
    /// Across from centred, as a share of the frame's width.
    pub offset_x: Animated<f64>,
    /// Down from centred, as a share of the frame's height.
    pub offset_y: Animated<f64>,
    /// A factor over the fitted size; above zero.
    pub scale: Animated<f64>,
    /// Clockwise degrees about the picture's centre.
    pub rotation: Animated<f64>,
    /// A factor on the width beyond the scale; above zero.
    pub stretch_x: Animated<f64>,
    /// The same on the height.
    pub stretch_y: Animated<f64>,
    /// How strongly it covers what is beneath, 0 to 1.
    pub opacity: Animated<f64>,
    /// Mirrored left to right.
    pub flip_h: bool,
    /// Mirrored top to bottom.
    pub flip_v: bool,
    /// How its colour meets what is beneath it.
    pub blend: Blend,
    /// What is cut off each edge of the source before it is fitted.
    pub crop: Option<Crop>,
    /// Its background taken away by a mask, not a key colour.
    pub cutout: Option<Cutout>,
    /// The camera's shake taken out.
    pub stabilize: Option<Stabilize>,
    /// Its own background over its sequence's; none follows the
    /// sequence's.
    pub background: Option<Background>,
    /// How it comes on, goes off and moves in between.
    pub motion: Motion,
}

impl Default for Picture {
    fn default() -> Picture {
        Picture {
            offset_x: Animated::fixed(0.0),
            offset_y: Animated::fixed(0.0),
            scale: Animated::fixed(1.0),
            rotation: Animated::fixed(0.0),
            stretch_x: Animated::fixed(1.0),
            stretch_y: Animated::fixed(1.0),
            opacity: Animated::fixed(1.0),
            flip_h: false,
            flip_v: false,
            blend: Blend::Normal,
            crop: None,
            cutout: None,
            stabilize: None,
            background: None,
            motion: Motion::default(),
        }
    }
}

/// How a picture's colour meets what is beneath it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Blend {
    /// Over it.
    Normal,
    /// Multiplied.
    Multiply,
    /// Screened.
    Screen,
    /// Added.
    Add,
    /// The lighter of the two.
    Lighten,
    /// The darker of the two.
    Darken,
    /// A blend this build does not know.
    Unknown(Raw),
}

/// What is cut off each edge of a source, as shares of its width and
/// height. Each is from 0 to below 1, and two opposite edges leave
/// something between them.
#[derive(Clone, Copy, PartialEq, Debug)]
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

/// A picture's background taken away by the cutout model, alone or
/// corrected by hand.
#[derive(Clone, PartialEq, Debug)]
pub struct Cutout {
    /// The model's mask alone, or with the strokes over it.
    pub mode: CutoutMode,
    /// What the model looks for.
    pub subject: Subject,
    /// How far the edge is softened, as a share of the picture's width;
    /// not below zero.
    pub feather: f64,
    /// Corrections painted on the monitor, in the order they were made.
    pub strokes: Vec<Stroke>,
}

/// The two ways a cutout decides what stays.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum CutoutMode {
    /// The model's mask as it is.
    Auto,
    /// The model's mask, then the strokes.
    Custom,
    /// A mode this build does not know.
    Unknown(Raw),
}

/// What a cutout keeps.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Subject {
    /// A person when one is found, the thing the picture is of
    /// otherwise.
    Auto,
    /// The person model, always.
    Person,
    /// The object model, always.
    Object,
    /// A subject this build does not know.
    Unknown(Raw),
}

/// One brush stroke over a cutout.
#[derive(Clone, PartialEq, Debug)]
pub struct Stroke {
    /// What it does to the mask.
    pub tool: BrushTool,
    /// The brush's width as a share of the picture's; above zero.
    pub size: f64,
    /// Where it went, as shares of the source picture from its top left.
    pub points: Vec<[f64; 2]>,
    /// The place in the source it was painted at, when that was kept.
    pub at: Option<Tick>,
}

/// The four brushes of a custom cutout.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum BrushTool {
    /// Keeps what the model thought might be the subject.
    SmartBrush,
    /// Keeps everything under it.
    Brush,
    /// Removes what the model was unsure of.
    SmartEraser,
    /// Removes everything under it.
    Eraser,
    /// A brush this build does not know.
    Unknown(Raw),
}

/// The settings a picture's stabilization is worked out from.
#[derive(Clone, PartialEq, Debug)]
pub struct Stabilize {
    /// How much of the camera's movement is smoothed away, 0 to 1.
    pub smoothness: f64,
    /// The most the picture may be zoomed to hide the edges; at least 1.
    pub max_zoom: f64,
    /// Leave the camera's roll as shot.
    pub keep_rotation: bool,
    /// Lock the frame, as on a tripod.
    pub tripod: bool,
}

/// How an item comes on, goes off and moves in between. Each is applied
/// where the item is placed, over its keys. A move's name this build
/// does not know is kept and does nothing.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Motion {
    /// How it comes on.
    pub enter: Option<Move>,
    /// How it goes off.
    pub exit: Option<Move>,
    /// What it does all along.
    pub repeat: Option<Loop>,
}

/// An entrance or an exit: a move by name, over a length.
#[derive(Clone, PartialEq, Debug)]
pub struct Move {
    /// The move's name: `fade`, `slide-left`, `pop`, ...
    pub name: String,
    /// How long it takes.
    pub len: TickLen,
}

/// A loop by name: a lap every `lap`, at `amount` (1 as designed).
#[derive(Clone, PartialEq, Debug)]
pub struct Loop {
    /// The loop's name: `pulse`, `float`, `wiggle`, ...
    pub name: String,
    /// How long a lap takes.
    pub lap: TickLen,
    /// How much of it, 0 to 2.
    pub amount: f64,
}

/// How an item's sound is mixed.
#[derive(Clone, PartialEq, Debug)]
pub struct Sound {
    /// The gain, 1 as recorded; not below zero.
    pub volume: Animated<f64>,
    /// A ramp in from silence at the head; not below zero.
    pub fade_in: TickDur,
    /// A ramp out to silence at the tail; not below zero.
    pub fade_out: TickDur,
    /// The item's own sound silenced.
    pub muted: bool,
    /// Voices kept at their pitch when the speed is not 1.
    pub keep_pitch: bool,
    /// The voice and everything else at levels of their own.
    pub voice: Option<VoiceMix>,
}

impl Default for Sound {
    fn default() -> Sound {
        Sound {
            volume: Animated::fixed(1.0),
            fade_in: TickDur::ZERO,
            fade_out: TickDur::ZERO,
            muted: false,
            keep_pitch: true,
            voice: None,
        }
    }
}

/// Voice isolation: the voice and the rest of a sound, each at a gain of
/// its own, 1 as recorded and not below zero.
#[derive(Clone, PartialEq, Debug)]
pub struct VoiceMix {
    /// What the voice is set apart from.
    pub kind: VoiceKind,
    /// The voice's gain.
    pub voice: f64,
    /// Everything else's.
    pub background: f64,
}

/// The two kinds of voice isolation.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum VoiceKind {
    /// Singing apart from the music.
    Vocals,
    /// Speech apart from noise and the room.
    Speech,
    /// A kind this build does not know.
    Unknown(Raw),
}

/// A transition on the cut into an item.
#[derive(Clone, PartialEq, Debug)]
pub struct CutIn {
    /// Its package and knobs.
    pub effect: Effect,
    /// How long it covers.
    pub len: TickLen,
}

/// An item tied to another. Kept on the item that depends, so a tie is
/// never one-sided: what it is tied from is found by the document's
/// index.
#[derive(Clone, PartialEq, Debug)]
pub struct Link {
    /// What it is tied to: another item of the same sequence.
    pub to: ItemId,
    /// How.
    pub kind: LinkKind,
}

/// How one item is tied to another.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum LinkKind {
    /// This item is the sound detached from `to`, a cut of the same file
    /// that plays the whole of it. Several may be detached from one, as
    /// splitting a detached sound makes.
    SoundOf,
    /// A tie this build does not know.
    Unknown(Raw),
}

fn effect_weight(effect: &Effect) -> usize {
    let params: usize = effect
        .params
        .iter()
        .map(|(name, param)| 64 + name.len() + param.keys.len() * 40)
        .sum();
    std::mem::size_of::<Effect>() + effect.package.len() + params + effect.residual.weight()
}

fn entry_weight<T>(entries: &[Entry<T>], each: impl Fn(&T) -> usize) -> usize {
    entries
        .iter()
        .map(|entry| match entry {
            Entry::Known(value) => each(value),
            Entry::Unread(raw) => 32 + raw.0.len(),
        })
        .sum()
}

fn keys_weight(animated: &Animated<f64>) -> usize {
    animated.keys.len() * 40
}

impl Weigh for Item {
    fn weight(&self) -> usize {
        let picture = &self.picture;
        let keys = [
            &picture.offset_x,
            &picture.offset_y,
            &picture.scale,
            &picture.rotation,
            &picture.stretch_x,
            &picture.stretch_y,
            &picture.opacity,
            &self.sound.volume,
        ]
        .into_iter()
        .map(keys_weight)
        .sum::<usize>();
        let content = match &self.content {
            Content::Media(cut) => cut.time.points().len() * 40,
            Content::Nest(nest) => nest.time.points().len() * 40,
            Content::Title(title) => title.content.len() + title.runs.len() * 64,
            Content::Transition(effect) => effect_weight(effect),
            Content::Unknown { kind, raw } => kind.len() + raw.0.len(),
            Content::Shape(_) | Content::Layer | Content::SoundLayer => 0,
        };
        let strokes = picture.cutout.as_ref().map_or(0, |cutout| {
            cutout
                .strokes
                .iter()
                .map(|stroke| 48 + stroke.points.len() * 16)
                .sum()
        });
        std::mem::size_of::<Item>()
            + self.name.len()
            + keys
            + content
            + strokes
            + entry_weight(&self.effects, effect_weight)
            + entry_weight(&self.filters, effect_weight)
            + entry_weight(&self.masks, |mask| {
                std::mem::size_of::<Mask>() + mask.name.len()
            })
            + entry_weight(&self.markers, |marker| 96 + marker.name.len())
            + self.residual.weight()
    }
}
