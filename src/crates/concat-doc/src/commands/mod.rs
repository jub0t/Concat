// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The edits a person makes, each one transaction on the model.
//!
//! A [`Command`] is one thing done in the window: a clip dropped, a trim,
//! a split, a drag of a selection, a speed change, a title typed, a lane
//! added. [`apply`] makes it in a [`Draft`] of the document; [`run`]
//! begins the draft, applies the command, and commits it, so one command
//! is one commit and one step of undo. A command that changes nothing
//! makes no commit: no step, no new revision, no id used up. A command
//! the model's rules refuse leaves the document as it was, and says why
//! in plain words ([`CommandError`]).
//!
//! The commands are said in the document's own terms: typed ids, ticks
//! and lengths, speeds as fractions, the model's own types for a look or
//! a file. Every variant names the operation of the published API it
//! serves, and its fields follow that operation's, so the API's JSON
//! becomes a command field for field: seconds become ticks, id strings
//! become ids, an effect's place in its chain becomes the effect's id.
//!
//! What the document keeps outside itself is handed in: a file is added
//! with what its probe found, a freeze frame with the still the host
//! took. Nothing here reads the disk or the network.
//!
//! # Placing without covering
//!
//! No two items overlap on a lane (rule 21). Where an edit would put a
//! clip over another, it goes elsewhere rather than being refused: a
//! drop names a lane and lands on a new lane just above it; a placement
//! that names none takes the lowest lane free for its span, or a new one
//! on top; a dragged clip that would land on another goes to the first
//! lane above that is free, or a new one on top.
//!
//! # The main lane
//!
//! A sequence's lowest lane is its main lane. With the sequence's
//! linkage on, an edit that opens or closes up the main lane (a ripple
//! trim, a ripple speed change, a drop that makes room, a freeze frame)
//! moves the other lanes' clips after the edit by as much, so what was
//! placed against the footage stays on its frames. Removing clips with
//! ripple closes up only the lanes it took clips from.

mod clips;
mod look;
mod place;
mod project;
mod speed;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::document::{ChangeSet, Commit, Document, Draft, Origin, Rejected};
use crate::id::{AnyId, MintError, RawId, TypedId, Writer};
use crate::model::{
    Background, BeatMarks, CaptionLook, Color, ColorRange, Cover, Crop, CutIn, Cutout, Ease,
    Effect, Frame, Item, ItemId, LaneId, Mask, MaskId, Media, MediaId, Model, Motion, SequenceId,
    ShapeStyle, Speed, Stabilize, Stroke, Title, Violation, VoiceMix,
};
use crate::table::TableError;
use crate::time::{Tick, TickDur, TickLen};

pub use speed::speed_from_decimal;

/// The shortest a trim, a split or a speed change leaves a clip: a
/// sixtieth of a second.
pub const MIN_LEN: TickLen = match TickLen::new(crate::time::TICKS_PER_SECOND / 60) {
    Some(len) => len,
    None => TickLen::ONE,
};

/// How long a still, or a file whose length is not known, lasts when it
/// is placed: five seconds.
pub const STILL_LEN: TickLen = seconds(5);

/// How long a title, a shape or a layer lasts when no length is given:
/// three seconds.
pub const TITLE_LEN: TickLen = seconds(3);

/// How long a transition lasts when no length is given: a second.
pub const TRANSITION_LEN: TickLen = seconds(1);

/// The shortest a transition is made: a tenth of a second.
pub const MIN_TRANSITION: TickLen = match TickLen::new(crate::time::TICKS_PER_SECOND / 10) {
    Some(len) => len,
    None => TickLen::ONE,
};

/// How long a freeze frame holds when no length is given: three seconds.
pub const FREEZE_LEN: TickLen = seconds(3);

/// How near two keys of one number may be before a new one replaces the
/// old: a millisecond of content time.
pub const HAIR: TickDur = match TickDur::from_millis(1) {
    Some(d) => d,
    None => TickDur::ZERO,
};

const fn seconds(n: i64) -> TickLen {
    match TickLen::new(n * crate::time::TICKS_PER_SECOND) {
        Some(len) => len,
        None => TickLen::ONE,
    }
}

/// The id to give a new effect or mask handed to a command: the command
/// gives it one of the document's own. Any id the item does not already
/// hold in that place is treated the same way.
pub fn placeholder<I: TypedId>() -> I {
    match RawId::from_parts(Writer(u16::MAX), crate::id::MAX_COUNTER) {
        Some(raw) => I::from_raw(raw),
        None => unreachable!("the last count is a count"),
    }
}

/// Which of an item's own numbers a key is on: `KeyProperty`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyProperty {
    /// `picture.scale`.
    Scale,
    /// `picture.offset_x`.
    OffsetX,
    /// `picture.offset_y`.
    OffsetY,
    /// `picture.rotation`.
    Rotation,
    /// `picture.stretch_x`.
    StretchX,
    /// `picture.stretch_y`.
    StretchY,
    /// `picture.opacity`.
    Opacity,
    /// `sound.volume`.
    Volume,
}

/// Which number of a mask a key is on: `Mask::NUMBERS`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MaskNumber {
    /// Across from centred.
    X,
    /// Down from centred.
    Y,
    /// Its size.
    Scale,
    /// Its turn.
    Rotation,
    /// Its edge's softness.
    Feather,
    /// How far its edge is moved out.
    Expand,
    /// How much it counts.
    Opacity,
    /// Its box's width.
    Width,
    /// Its box's height.
    Height,
    /// Its box's corners.
    Roundness,
}

/// Which end of a clip a trim drags: `TrimEdge`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Edge {
    /// The head: the in-point moves with it, so the frames left stay
    /// where they were.
    Head,
    /// The tail.
    Tail,
}

/// A lane's two toggles: `TrackFlag`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LaneFlag {
    /// Whether its pictures are drawn.
    Visible,
    /// Whether its sound is silent.
    Muted,
}

/// The two ways a picture is sized to the frame: `Framing`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Framing {
    /// All of it inside the frame.
    Fit,
    /// The frame covered.
    Fill,
}

/// One point of a speed curve, as the speed graph draws it: `SpeedPoint`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpeedPoint {
    /// Where in the clip, as a share of its length on the timeline.
    pub at: f64,
    /// Source per timeline there.
    pub speed: f64,
    /// How fast the speed changes there, per unit of `at`. When every
    /// point has one the curve bends through them; otherwise it runs
    /// straight between them.
    pub slope: Option<f64>,
}

/// One caption to place: `CaptionCue`.
#[derive(Clone, PartialEq, Debug)]
pub struct Cue {
    /// Where it starts on the timeline.
    pub start: Tick,
    /// How long it stays up.
    pub len: TickLen,
    /// What it says.
    pub words: String,
}

/// Where one clip of a drag is going: `ClipMove`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ClipMove {
    /// The clip.
    pub item: ItemId,
    /// Its new start; below zero is zero.
    pub start: Tick,
    /// Its new lane; none, or one not in its sequence, keeps its lane.
    pub lane: Option<LaneId>,
}

/// What a stretch of a title's words wears over the title's look:
/// `RunStyle`. A field left `None` is not changed.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct RunStyle {
    /// The fill.
    pub color: Option<Color>,
    /// The size as a multiple of the title's.
    pub font_scale: Option<f64>,
    /// The weight.
    pub font_weight: Option<f64>,
    /// Italic.
    pub italic: Option<bool>,
    /// Underlined.
    pub underline: Option<bool>,
    /// Struck through.
    pub strikethrough: Option<bool>,
    /// The outline's colour.
    pub stroke_color: Option<Color>,
}

impl RunStyle {
    fn is_empty(&self) -> bool {
        *self == RunStyle::default()
    }
}

/// A change to some of an item's settings: `ClipPatch`. A field left
/// `None` is left alone; an inner `None` takes the setting off.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ItemPatch {
    /// The sound stream it plays; `Some(None)` is the file's first.
    pub stream: Option<Option<u32>>,
    /// Its own background; `Some(None)` follows the sequence's.
    pub background: Option<Option<Background>>,
    /// How its colour meets what is beneath.
    pub blend: Option<crate::model::Blend>,
    /// What is cut off its edges.
    pub crop: Option<Option<Crop>>,
    /// The ramp in; below zero is zero.
    pub fade_in: Option<TickDur>,
    /// The ramp out; below zero is zero.
    pub fade_out: Option<TickDur>,
    /// The sound filters, whole. An effect keeps its id where the item
    /// already holds it in this chain; any other is given a new one.
    pub filters: Option<Vec<Effect>>,
    /// The picture effects, whole, on the same terms.
    pub effects: Option<Vec<Effect>>,
    /// Mirrored left to right.
    pub flip_h: Option<bool>,
    /// Mirrored top to bottom.
    pub flip_v: Option<bool>,
    /// The masks, top first, whole. A mask keeps its id where the item
    /// already holds it and no mask before it in the list has it; any
    /// other is given a new one.
    pub masks: Option<Vec<Mask>>,
    /// How it comes on, goes off and loops; the default is none of them.
    pub motion: Option<Motion>,
    /// Its own sound silenced.
    pub muted: Option<bool>,
    /// What the lane calls it.
    pub name: Option<String>,
    /// How strongly it covers, held to 0 to 1.
    pub opacity: Option<f64>,
    /// Voices kept at their pitch.
    pub keep_pitch: Option<bool>,
    /// A shape's figure and paint; only a shape takes it.
    pub shape: Option<ShapeStyle>,
    /// A title's words and look, which also name it after their first
    /// line; only a title takes it.
    pub title: Option<Title>,
    /// The transition on the cut into it. Its package keeps the id the
    /// one there had.
    pub cut_in: Option<Option<CutIn>>,
    /// Its gain; below zero is zero.
    pub volume: Option<f64>,
}

/// One edit. Each is one transaction, and so one step of undo.
// A command is made once per edit and dropped after it; its size is no
// cost worth a box in every field that holds a file or a patch.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, PartialEq, Debug)]
pub enum Command {
    // ----- The bin -----
    /// `addMedia`: a file into the bin, with what its probe found. A path
    /// already in the bin adds nothing, and answers with that file's id.
    AddMedia {
        /// The file.
        media: Media,
    },
    /// `removeMedia`: a file out of the bin, with every clip cut from it
    /// on every sequence, every enhanced copy of it a clip shows, every
    /// background it is, and the cover if it is the cover. An unknown
    /// file is nothing.
    RemoveMedia {
        /// The file.
        media: MediaId,
    },
    /// `setMediaPlaceholder`: a file made a template's slot, or back.
    SetMediaPlaceholder {
        /// The file.
        media: MediaId,
        /// A slot or not.
        placeholder: bool,
    },
    /// `fillSlot`: a person's file put in a slot's place. The slot keeps
    /// its id, its clips their places, lengths and speeds; each clip's
    /// in-point goes back to the file's start and its name follows the
    /// file.
    FillSlot {
        /// The slot.
        media: MediaId,
        /// The file taking its place.
        file: Media,
    },
    /// `updateMediaPath`: where a file is now (relink).
    UpdateMediaPath {
        /// The file.
        media: MediaId,
        /// Its new path.
        path: String,
    },
    /// `setMediaColorRange`: the levels a file's picture spans, over its
    /// own tag; none reads the tag.
    SetMediaColorRange {
        /// The file.
        media: MediaId,
        /// The levels.
        range: Option<ColorRange>,
    },
    /// `setMediaBeats`: which of a file's beats are marked along its
    /// clips; none for none. The beats themselves are the host's.
    SetMediaBeats {
        /// The file.
        media: MediaId,
        /// Which beats.
        marks: Option<BeatMarks>,
    },

    // ----- Several at once -----
    /// `batch`: commands one after another as one edit; if any is
    /// refused, none is made.
    Batch {
        /// The commands, in order.
        commands: Vec<Command>,
    },

    // ----- Placing -----
    /// `addClip`: a clip of a file dropped on a lane. Its length is the
    /// file's, or [`STILL_LEN`]. With `ripple`, what it would cover from
    /// `start` on moves right to make room; what it would still cover
    /// sends it to a new lane just above.
    AddClip {
        /// The file.
        media: MediaId,
        /// The lane.
        lane: LaneId,
        /// Where; below zero is zero.
        start: Tick,
        /// Make room rather than go to a lane of its own.
        ripple: bool,
    },
    /// `addClipAtFirstFree`: a clip on the lowest lane of the sequence
    /// showing that is free for its span, or a new lane on top.
    AddClipAtFirstFree {
        /// The file.
        media: MediaId,
        /// Where; below zero is zero.
        start: Tick,
    },
    /// `addClipsInSequence`: several files end to end. On a lane named,
    /// each lands as [`Command::AddClip`] lands one; with none, the run
    /// goes on the lowest lane free for all of it, or a new one on top.
    AddClipsInSequence {
        /// The files, in order.
        media: Vec<MediaId>,
        /// Where the first starts.
        start: Tick,
        /// The lane, if one is named.
        lane: Option<LaneId>,
        /// On a named lane, make room.
        ripple: bool,
    },
    /// `addTextClip`: a title, named after its first line.
    AddTextClip {
        /// Where.
        start: Tick,
        /// How long; [`TITLE_LEN`] when none.
        len: Option<TickLen>,
        /// The lane; none picks one (see `above`).
        lane: Option<LaneId>,
        /// With no lane: the first free lane above everything over its
        /// span, rather than the lowest free.
        above: bool,
        /// Down from centred, a share of the frame's height, held to ±3.
        offset_y: Option<f64>,
        /// The words and their look; the default title when none.
        title: Option<Title>,
    },
    /// `addShapeClip`: a figure over the picture.
    AddShapeClip {
        /// Where.
        start: Tick,
        /// How long; [`TITLE_LEN`] when none.
        len: Option<TickLen>,
        /// What the lane calls it.
        name: String,
        /// The figure; the default when none.
        style: Option<ShapeStyle>,
        /// The lane; none picks one as `above` says.
        lane: Option<LaneId>,
        /// As [`Command::AddTextClip`]'s.
        above: bool,
    },
    /// `addLayerClip`: a treatment of everything beneath for its span,
    /// its chain one package at its defaults. With `sound`, of the mix.
    /// With no lane, the first free above everything over its span.
    AddLayerClip {
        /// Where.
        start: Tick,
        /// How long; [`TITLE_LEN`] when none.
        len: Option<TickLen>,
        /// The package.
        package: String,
        /// What the lane calls it.
        name: String,
        /// A sound layer.
        sound: bool,
        /// The lane.
        lane: Option<LaneId>,
    },
    /// `addTransitionClip`: a transition as an item of its own over what
    /// is beneath it. With no lane, the first free above everything over
    /// its span.
    AddTransitionClip {
        /// Where.
        start: Tick,
        /// How long; [`TRANSITION_LEN`] when none, never under
        /// [`MIN_TRANSITION`].
        len: Option<TickLen>,
        /// The package.
        package: String,
        /// What the lane calls it.
        name: String,
        /// The lane.
        lane: Option<LaneId>,
    },
    /// `placeCaptions`: captions as titles on a caption lane. With no
    /// lane a caption lane is made on top in `look`; a lane named keeps
    /// its look, or takes `look` when it has none. Cues go in time
    /// order; one that runs into the next ends where it begins; one that
    /// would cover a clip moves to the nearest gap; one with no words is
    /// left out. Answers with the lane.
    PlaceCaptions {
        /// The captions.
        cues: Vec<Cue>,
        /// The look a new caption lane takes. Its words and runs are not
        /// read.
        look: CaptionLook,
        /// The lane.
        lane: Option<LaneId>,
    },
    /// `setCaptionLook`: a caption lane's look, and every title on it
    /// restyled to it, each keeping its words and runs. None makes it a
    /// plain lane and leaves the titles as they are.
    SetCaptionLook {
        /// The lane.
        lane: LaneId,
        /// The look.
        look: Option<CaptionLook>,
    },

    // ----- Keys -----
    /// `setClipKey`: a key on one of an item's numbers, replacing one
    /// within [`HAIR`] of it.
    SetClipKey {
        /// The item.
        item: ItemId,
        /// Which number.
        property: KeyProperty,
        /// Where, on the item's content time.
        at: Tick,
        /// The value there.
        value: f64,
        /// How it is reached.
        ease: Ease,
    },
    /// `clearClipKey`: the key within [`HAIR`] of `at` taken off.
    ClearClipKey {
        /// The item.
        item: ItemId,
        /// Which number.
        property: KeyProperty,
        /// Where, on content time.
        at: Tick,
    },
    /// `clearClipKeys`: every key of one number taken off.
    ClearClipKeys {
        /// The item.
        item: ItemId,
        /// Which number.
        property: KeyProperty,
    },
    /// `setEffectKey`: a key on one knob of one of an item's effects.
    SetEffectKey {
        /// The item.
        item: ItemId,
        /// The effect: in its picture or sound chain, on its cut, or the
        /// transition it is.
        effect: crate::model::EffectId,
        /// The knob's key in the package's manifest.
        param: String,
        /// Where, on content time.
        at: Tick,
        /// The value there.
        value: f64,
        /// How it is reached.
        ease: Ease,
    },
    /// `clearEffectKey`: the key within [`HAIR`] of `at` taken off a
    /// knob. A knob left with no value and no keys goes.
    ClearEffectKey {
        /// The item.
        item: ItemId,
        /// The effect.
        effect: crate::model::EffectId,
        /// The knob.
        param: String,
        /// Where.
        at: Tick,
    },
    /// `clearEffectKeys`: every key taken off a knob.
    ClearEffectKeys {
        /// The item.
        item: ItemId,
        /// The effect.
        effect: crate::model::EffectId,
        /// The knob.
        param: String,
    },
    /// `setMaskKey`: a key on one number of one of an item's masks, held
    /// to the number's range.
    SetMaskKey {
        /// The item.
        item: ItemId,
        /// The mask.
        mask: MaskId,
        /// Which number.
        number: MaskNumber,
        /// Where, on content time.
        at: Tick,
        /// The value there.
        value: f64,
        /// How it is reached.
        ease: Ease,
    },
    /// `clearMaskKey`.
    ClearMaskKey {
        /// The item.
        item: ItemId,
        /// The mask.
        mask: MaskId,
        /// Which number.
        number: MaskNumber,
        /// Where.
        at: Tick,
    },
    /// `clearMaskKeys`.
    ClearMaskKeys {
        /// The item.
        item: ItemId,
        /// The mask.
        mask: MaskId,
        /// Which number.
        number: MaskNumber,
    },

    // ----- The picture's treatments -----
    /// `setClipCutout`: the background taken away, or not.
    SetClipCutout {
        /// The item.
        item: ItemId,
        /// The cutout.
        cutout: Option<Cutout>,
    },
    /// `setClipStabilize`: the shake taken out, or not.
    SetClipStabilize {
        /// The item.
        item: ItemId,
        /// The settings.
        stabilize: Option<Stabilize>,
    },
    /// `setClipVoice`: the voice and the rest at levels of their own, or
    /// not. Nothing for an item with no sound.
    SetClipVoice {
        /// The item.
        item: ItemId,
        /// The levels.
        voice: Option<VoiceMix>,
    },
    /// `addCutoutStroke`: one brush stroke on a cutout, which becomes a
    /// custom one.
    AddCutoutStroke {
        /// The item.
        item: ItemId,
        /// The stroke.
        stroke: Stroke,
    },

    // ----- Moving and cutting -----
    /// `moveClips`: a drag of a selection, as one edit.
    MoveClips {
        /// Where each goes.
        moves: Vec<ClipMove>,
    },
    /// `moveClipsToNewTrack`: clips onto a lane made for them, on top or
    /// under every other. A move that lands nothing makes no lane.
    /// Answers with the lane.
    MoveClipsToNewTrack {
        /// Where each goes in time; the lanes are not read.
        moves: Vec<ClipMove>,
        /// Under every other lane.
        bottom: bool,
    },
    /// `trimClip`: one edge dragged. The head moves the window with it;
    /// either edge stops at [`MIN_LEN`], at the ends of the source, and,
    /// without ripple, at the clips beside it and at zero.
    TrimClip {
        /// The item.
        item: ItemId,
        /// Which edge.
        edge: Edge,
        /// How far the edge moves on the timeline; right is above zero.
        delta: TickDur,
        /// Close up or open the lane behind: a head trim then keeps the
        /// clip's start and moves what follows.
        ripple: bool,
    },
    /// `splitClips`: each clip under the playhead cut in two. The head
    /// keeps the id, the transition in and the fade in; the tail is new,
    /// shows the content time on from the cut, and keeps the fade out.
    /// Keys stay where they are in content time. A clip the time misses,
    /// or meets within [`MIN_LEN`] of an end, is left.
    SplitClips {
        /// The clips.
        items: Vec<ItemId>,
        /// The cut, on the timeline.
        at: Tick,
    },
    /// `duplicateClips`: each clip copied whole and laid after its own
    /// end, or in the first gap from there, the latest first. A copy has
    /// no transition in and no tie. Answers with the earliest's copy.
    DuplicateClips {
        /// The clips.
        items: Vec<ItemId>,
    },
    /// `replaceClipMedia`: a clip pointed at another file, added to the
    /// bin when it is not there. With `enhanced` the file is the clip's
    /// Enhance and the clip keeps its own as the original.
    ReplaceClipMedia {
        /// The clip.
        item: ItemId,
        /// The file.
        file: Media,
        /// It is the clip's enhanced copy.
        enhanced: bool,
        /// With `enhanced`, where in the original the copy starts.
        copy_start: TickDur,
        /// A new in-point in the file; none keeps the clip's.
        source_start: Option<Tick>,
    },
    /// `setClipEnhanced`: the enhanced copy shown, or the original.
    SetClipEnhanced {
        /// The clip.
        item: ItemId,
        /// The copy.
        on: bool,
    },
    /// `freezeFrame`: the clip split at `at`, a still held there for
    /// `len`, and what follows on the lane moved on by as much. A video
    /// needs the still the host took; a still is held from its own file.
    /// Answers with the held frame.
    FreezeFrame {
        /// The clip.
        item: ItemId,
        /// Where, strictly inside it.
        at: Tick,
        /// How long; [`FREEZE_LEN`] when none, never under [`MIN_LEN`].
        len: Option<TickLen>,
        /// The still.
        still: Option<Media>,
    },
    /// `mergeClips`: pieces cut from one clip joined back into the
    /// earliest, which keeps its id.
    MergeClips {
        /// The pieces, in any order.
        items: Vec<ItemId>,
    },
    /// `consolidateClips`: clips on one lane replaced by one clip of the
    /// file they were rendered to, over their whole span. The earliest
    /// keeps its id; what the render baked in is reset.
    ConsolidateClips {
        /// Two or more clips on one lane.
        items: Vec<ItemId>,
        /// The rendered file.
        file: Media,
    },
    /// `removeClips`: clips deleted, with anything tied to them untied
    /// and any transition left over nothing. With `ripple`, each lane
    /// they came off closes up.
    RemoveClips {
        /// The clips.
        items: Vec<ItemId>,
        /// Close up.
        ripple: bool,
    },
    /// `updateClip`: some of a clip's settings.
    UpdateClip {
        /// The clip.
        item: ItemId,
        /// The settings.
        patch: ItemPatch,
    },
    /// `styleTextRange`: a look laid over some of a title's words,
    /// counted in characters and held to the words.
    StyleTextRange {
        /// The title.
        item: ItemId,
        /// The first character.
        start: u32,
        /// One past the last.
        end: u32,
        /// The look.
        style: RunStyle,
    },
    /// `clearTextStyles`: some of a title's words, or all, given back to
    /// its look.
    ClearTextStyles {
        /// The title.
        item: ItemId,
        /// The first character; none for the start.
        start: Option<u32>,
        /// One past the last; none for the end.
        end: Option<u32>,
    },
    /// `setClipSpeed`: a new pace over the same source, so the clip's
    /// length follows. A speed curve ends. Keys keep their share of the
    /// clip.
    SetClipSpeed {
        /// The clip.
        item: ItemId,
        /// The pace, held from a sixteenth to sixteen times.
        speed: Speed,
        /// What follows on the lane follows the clip's end either way;
        /// without, it moves only to make room.
        ripple: bool,
    },
    /// `setClipSpeedCurve`: a speed curve over the same source, or a
    /// constant pace at the curve's mean.
    SetClipSpeedCurve {
        /// The clip.
        item: ItemId,
        /// The points; none, or none usable, for a constant pace.
        curve: Option<Vec<SpeedPoint>>,
        /// As [`Command::SetClipSpeed`]'s.
        ripple: bool,
    },
    /// `setClipTransform`: the picture's placement, each part optional.
    /// Offsets are held to ±3, the scale to 0.05 to 8, a stretch to 0.1
    /// to 10, and a rotation brought into (-180, 180].
    SetClipTransform {
        /// The clip.
        item: ItemId,
        /// Across.
        offset_x: Option<f64>,
        /// Down.
        offset_y: Option<f64>,
        /// Turned.
        rotation: Option<f64>,
        /// Size.
        scale: Option<f64>,
        /// Width beyond the size.
        stretch_x: Option<f64>,
        /// Height beyond the size.
        stretch_y: Option<f64>,
    },
    /// `frameClip`: a picture fitted inside the frame or covering it,
    /// centred and unstretched. A number with keys gets a key at `at`.
    FrameClip {
        /// The clip.
        item: ItemId,
        /// Fit or fill.
        framing: Framing,
        /// Where a keyed number takes its key, on content time.
        at: Tick,
    },
    /// `detachAudio`: a video's sound into a clip of its own on a free
    /// lane, tied to the picture, which is muted and hands it its sound
    /// filters.
    DetachAudio {
        /// The video.
        item: ItemId,
    },
    /// `reattachAudio`: the sounds detached from a video deleted, the
    /// video unmuted and given the filters back. Either side may be
    /// named.
    ReattachAudio {
        /// The video or its sound.
        item: ItemId,
    },

    // ----- Lanes -----
    /// `addTrack`: a lane on top of the sequence showing, or under every
    /// other. Answers with the lane.
    AddTrack {
        /// Under every other.
        bottom: bool,
    },
    /// `removeTrack`: a lane and every clip on it. A sequence keeps one.
    RemoveTrack {
        /// The lane.
        lane: LaneId,
    },
    /// `setTrackFlag`: one of a lane's toggles.
    SetTrackFlag {
        /// The lane.
        lane: LaneId,
        /// Which.
        flag: LaneFlag,
        /// To what.
        value: bool,
    },

    // ----- Timelines -----
    /// `addTimeline`: a sequence of four lanes, "Timeline N" after the
    /// highest in use, in the frame of the one showing, shown in a tab
    /// of its own. Answers with the sequence.
    AddTimeline,
    /// `removeTimeline`: a sequence with its lanes and clips; the tab
    /// showing moves to a neighbour. A project keeps one, and a sequence
    /// placed inside another is not taken out.
    RemoveTimeline {
        /// The sequence.
        sequence: SequenceId,
    },
    /// `setTimelineVideo`: a sequence's frame and rate.
    SetTimelineVideo {
        /// The sequence.
        sequence: SequenceId,
        /// The frame.
        frame: Frame,
    },
    /// `setTimelineBackground`: what fills its frame behind.
    SetTimelineBackground {
        /// The sequence.
        sequence: SequenceId,
        /// The background; a blur is held to 0 to 1.
        background: Background,
    },
    /// `setTimelineLinkage`: whether its other lanes follow its main
    /// lane.
    SetTimelineLinkage {
        /// The sequence.
        sequence: SequenceId,
        /// On.
        on: bool,
    },
    /// `renameTimeline`: a tab's name, trimmed; a blank one is ignored.
    RenameTimeline {
        /// The sequence.
        sequence: SequenceId,
        /// The name.
        name: String,
    },
    /// `selectTimeline`: which sequence shows. View state: no step.
    SelectTimeline {
        /// The sequence.
        sequence: SequenceId,
    },
    /// `moveTimeline`: a tab to a new place in the strip, counted with
    /// it taken out, held to the end. View state: no step.
    MoveTimeline {
        /// The sequence.
        sequence: SequenceId,
        /// Where.
        index: usize,
    },

    // ----- The project -----
    /// `setCover`: the project's cover; none for a frame early in the
    /// edit. A frame before zero is zero.
    SetCover {
        /// The cover.
        cover: Option<Cover>,
    },
    /// `addFont`: a font file for titles. A path already there is
    /// nothing.
    AddFont {
        /// The family titles name.
        family: String,
        /// The file.
        path: String,
    },
    /// `removeFont`: a family's fonts taken off. Titles keep the name.
    RemoveFont {
        /// The family.
        family: String,
    },
}

impl Command {
    /// What the edit is called where a person reads it: the undo menu.
    pub fn label(&self) -> String {
        let label = match self {
            Command::AddMedia { .. } => "Import file",
            Command::RemoveMedia { .. } => "Remove file",
            Command::SetMediaPlaceholder { .. } => "Mark slot",
            Command::FillSlot { .. } => "Fill slot",
            Command::UpdateMediaPath { .. } => "Relink file",
            Command::SetMediaColorRange { .. } => "Set colour range",
            Command::SetMediaBeats { .. } => "Mark beats",
            Command::Batch { commands } => match commands.as_slice() {
                [one] => return one.label(),
                _ => "Edit",
            },
            Command::AddClip { .. } | Command::AddClipAtFirstFree { .. } => "Add clip",
            Command::AddClipsInSequence { .. } => "Add clips",
            Command::AddTextClip { .. } => "Add title",
            Command::AddShapeClip { .. } => "Add shape",
            Command::AddLayerClip { .. } => "Add layer",
            Command::AddTransitionClip { .. } => "Add transition",
            Command::PlaceCaptions { .. } => "Add captions",
            Command::SetCaptionLook { .. } => "Restyle captions",
            Command::SetClipKey { .. }
            | Command::SetEffectKey { .. }
            | Command::SetMaskKey { .. } => "Add key",
            Command::ClearClipKey { .. }
            | Command::ClearEffectKey { .. }
            | Command::ClearMaskKey { .. } => "Remove key",
            Command::ClearClipKeys { .. }
            | Command::ClearEffectKeys { .. }
            | Command::ClearMaskKeys { .. } => "Remove keys",
            Command::SetClipCutout { .. } => "Remove background",
            Command::SetClipStabilize { .. } => "Stabilize",
            Command::SetClipVoice { .. } => "Isolate voice",
            Command::AddCutoutStroke { .. } => "Brush cutout",
            Command::MoveClips { .. } | Command::MoveClipsToNewTrack { .. } => "Move clips",
            Command::TrimClip { .. } => "Trim clip",
            Command::SplitClips { .. } => "Split clip",
            Command::DuplicateClips { .. } => "Duplicate",
            Command::ReplaceClipMedia { .. } => "Replace file",
            Command::SetClipEnhanced { .. } => "Enhance",
            Command::FreezeFrame { .. } => "Freeze frame",
            Command::MergeClips { .. } => "Merge clips",
            Command::ConsolidateClips { .. } => "Consolidate clips",
            Command::RemoveClips { .. } => "Delete",
            Command::UpdateClip { .. } => "Edit clip",
            Command::StyleTextRange { .. } | Command::ClearTextStyles { .. } => "Style words",
            Command::SetClipSpeed { .. } => "Change speed",
            Command::SetClipSpeedCurve { curve: Some(_), .. } => "Speed ramp",
            Command::SetClipSpeedCurve { curve: None, .. } => "Remove speed ramp",
            Command::SetClipTransform { .. } => "Move picture",
            Command::FrameClip {
                framing: Framing::Fit,
                ..
            } => "Fit to frame",
            Command::FrameClip {
                framing: Framing::Fill,
                ..
            } => "Fill frame",
            Command::DetachAudio { .. } => "Detach audio",
            Command::ReattachAudio { .. } => "Reattach audio",
            Command::AddTrack { .. } => "Add lane",
            Command::RemoveTrack { .. } => "Delete lane",
            Command::SetTrackFlag {
                flag: LaneFlag::Visible,
                ..
            } => "Hide lane",
            Command::SetTrackFlag {
                flag: LaneFlag::Muted,
                ..
            } => "Mute lane",
            Command::AddTimeline => "New timeline",
            Command::RemoveTimeline { .. } => "Delete timeline",
            Command::SetTimelineVideo { .. } => "Timeline format",
            Command::SetTimelineBackground { .. } => "Timeline background",
            Command::SetTimelineLinkage { .. } => "Link lanes",
            Command::RenameTimeline { .. } => "Rename timeline",
            Command::SelectTimeline { .. } => "Switch timeline",
            Command::MoveTimeline { .. } => "Move tab",
            Command::SetCover { .. } => "Set cover",
            Command::AddFont { .. } => "Add font",
            Command::RemoveFont { .. } => "Remove font",
        };
        label.to_owned()
    }
}

/// What a command gives back besides the document.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Applied {
    /// The one id the edit is about, which the API calls `createdId`:
    /// the clip placed, the lane made, the tail of the first split.
    pub created: Option<AnyId>,
    /// Every id the edit made, in the order it made them, nested ones
    /// included.
    pub minted: Vec<AnyId>,
    /// What to select after it, where the edit says; none leaves the
    /// selection as it is.
    pub selection: Option<Vec<ItemId>>,
}

/// Why a command was not made. The document is as it was.
#[derive(Clone, PartialEq, Debug)]
pub enum CommandError {
    /// Something it names is not in the document.
    Gone {
        /// What sort of thing, in words: "clip", "lane".
        what: &'static str,
        /// Which.
        id: AnyId,
    },
    /// The edit cannot be made as asked, for the reason given.
    Refused(String),
    /// The edit would break the model's rules.
    Broken(Vec<Violation>),
    /// The project has used every id it may.
    OutOfIds,
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::Gone { what, id } => write!(f, "The {what} {id} is no longer there."),
            CommandError::Refused(why) => f.write_str(why),
            CommandError::Broken(broken) => {
                f.write_str("That edit would leave the project broken: ")?;
                for (n, violation) in broken.iter().enumerate() {
                    if n > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{violation}")?;
                }
                f.write_str(".")
            }
            CommandError::OutOfIds => f.write_str("The project has no more room for new things."),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<MintError> for CommandError {
    fn from(_: MintError) -> CommandError {
        CommandError::OutOfIds
    }
}

impl From<TableError> for CommandError {
    fn from(error: TableError) -> CommandError {
        CommandError::Gone {
            what: "thing",
            id: error.id(),
        }
    }
}

fn refuse<T>(why: &str) -> Result<T, CommandError> {
    Err(CommandError::Refused(why.to_owned()))
}

/// A command made and committed: the commit to record, if it changed
/// anything, and what it gives back.
pub struct Outcome {
    /// The commit, for [`History::record`](crate::History::record); none
    /// when the command changed nothing.
    pub commit: Option<Commit<Model>>,
    /// What the command gives back.
    pub applied: Applied,
}

/// Makes a command on a document as one transaction: begins a draft,
/// applies it, and commits.
///
/// A command that changes nothing gives no commit, so it is no step of
/// undo, makes no new revision and uses up no id. One that fails, or
/// that the rules refuse, leaves the document as it was.
pub fn run(
    document: &Document<Model>,
    command: &Command,
    origin: Origin,
) -> Result<Outcome, CommandError> {
    let mut draft = document.begin(command.label(), origin);
    let mut applied = apply(&mut draft, command)?;
    if draft.changes().is_empty() {
        applied.minted.clear();
        return Ok(Outcome {
            commit: None,
            applied,
        });
    }
    let commit = draft.commit().map_err(|rejected| match rejected {
        Rejected::Invalid(broken) => CommandError::Broken(broken),
        Rejected::Unminted(_) => {
            CommandError::Refused("That edit holds a name the project did not make.".to_owned())
        }
    })?;
    Ok(Outcome {
        commit: Some(commit),
        applied,
    })
}

/// Makes a command in a draft. The draft may be left half-made when this
/// fails: drop it.
pub fn apply(draft: &mut Draft<Model>, command: &Command) -> Result<Applied, CommandError> {
    let mut ed = Ed {
        d: draft,
        out: Applied::default(),
    };
    ed.apply(command)?;
    Ok(ed.out)
}

/// A draft being edited by one command, and what the command gives back.
pub(crate) struct Ed<'a> {
    pub(crate) d: &'a mut Draft<Model>,
    pub(crate) out: Applied,
}

impl Ed<'_> {
    fn apply(&mut self, command: &Command) -> Result<(), CommandError> {
        use Command as C;
        match command {
            C::AddMedia { media } => {
                let id = self.add_media(media)?;
                self.out.created = Some(id.any());
                Ok(())
            }
            C::RemoveMedia { media } => self.remove_media(*media),
            C::SetMediaPlaceholder { media, placeholder } => {
                self.media_mut(*media, |m| m.placeholder = *placeholder)
            }
            C::FillSlot { media, file } => self.fill_slot(*media, file),
            C::UpdateMediaPath { media, path } => {
                self.media_mut(*media, |m| m.path.clone_from(path))
            }
            C::SetMediaColorRange { media, range } => {
                self.media_mut(*media, |m| m.range.clone_from(range))
            }
            C::SetMediaBeats { media, marks } => {
                self.media_mut(*media, |m| m.beats.clone_from(marks))
            }
            C::Batch { commands } => {
                for command in commands {
                    let created = self.out.created;
                    let selection = self.out.selection.take();
                    self.out.created = None;
                    self.apply(command)?;
                    self.out.created = self.out.created.or(created);
                    if self.out.selection.is_none() {
                        self.out.selection = selection;
                    }
                }
                Ok(())
            }
            C::AddClip {
                media,
                lane,
                start,
                ripple,
            } => {
                let id = self.add_clip(*media, *lane, *start, *ripple)?;
                self.created_item(id);
                Ok(())
            }
            C::AddClipAtFirstFree { media, start } => {
                let id = self.add_clip_first_free(*media, *start)?;
                self.created_item(id);
                Ok(())
            }
            C::AddClipsInSequence {
                media,
                start,
                lane,
                ripple,
            } => self.add_clips_in_sequence(media, *start, *lane, *ripple),
            C::AddTextClip {
                start,
                len,
                lane,
                above,
                offset_y,
                title,
            } => {
                let id = self.add_title(*start, *len, *lane, *above, *offset_y, title.as_ref())?;
                self.created_item(id);
                Ok(())
            }
            C::AddShapeClip {
                start,
                len,
                name,
                style,
                lane,
                above,
            } => {
                let id = self.add_shape(*start, *len, name, style.as_ref(), *lane, *above)?;
                self.created_item(id);
                Ok(())
            }
            C::AddLayerClip {
                start,
                len,
                package,
                name,
                sound,
                lane,
            } => {
                let id = self.add_layer(*start, *len, package, name, *sound, *lane)?;
                self.created_item(id);
                Ok(())
            }
            C::AddTransitionClip {
                start,
                len,
                package,
                name,
                lane,
            } => {
                let id = self.add_transition(*start, *len, package, name, *lane)?;
                self.created_item(id);
                Ok(())
            }
            C::PlaceCaptions { cues, look, lane } => self.place_captions(cues, look, *lane),
            C::SetCaptionLook { lane, look } => self.set_caption_look(*lane, look.as_ref()),
            C::SetClipKey {
                item,
                property,
                at,
                value,
                ease,
            } => self.set_clip_key(*item, *property, *at, *value, ease),
            C::ClearClipKey { item, property, at } => {
                self.clear_clip_key(*item, *property, Some(*at))
            }
            C::ClearClipKeys { item, property } => self.clear_clip_key(*item, *property, None),
            C::SetEffectKey {
                item,
                effect,
                param,
                at,
                value,
                ease,
            } => self.set_effect_key(*item, *effect, param, *at, *value, ease),
            C::ClearEffectKey {
                item,
                effect,
                param,
                at,
            } => self.clear_effect_key(*item, *effect, param, Some(*at)),
            C::ClearEffectKeys {
                item,
                effect,
                param,
            } => self.clear_effect_key(*item, *effect, param, None),
            C::SetMaskKey {
                item,
                mask,
                number,
                at,
                value,
                ease,
            } => self.set_mask_key(*item, *mask, *number, *at, *value, ease),
            C::ClearMaskKey {
                item,
                mask,
                number,
                at,
            } => self.clear_mask_key(*item, *mask, *number, Some(*at)),
            C::ClearMaskKeys { item, mask, number } => {
                self.clear_mask_key(*item, *mask, *number, None)
            }
            C::SetClipCutout { item, cutout } => self.set_cutout(*item, cutout.as_ref()),
            C::SetClipStabilize { item, stabilize } => {
                self.set_stabilize(*item, stabilize.as_ref())
            }
            C::SetClipVoice { item, voice } => self.set_voice(*item, voice.as_ref()),
            C::AddCutoutStroke { item, stroke } => self.add_stroke(*item, stroke),
            C::MoveClips { moves } => self.move_clips(moves),
            C::MoveClipsToNewTrack { moves, bottom } => self.move_to_new_lane(moves, *bottom),
            C::TrimClip {
                item,
                edge,
                delta,
                ripple,
            } => self.trim(*item, *edge, *delta, *ripple),
            C::SplitClips { items, at } => self.split_clips(items, *at),
            C::DuplicateClips { items } => self.duplicate(items),
            C::ReplaceClipMedia {
                item,
                file,
                enhanced,
                copy_start,
                source_start,
            } => self.replace_media(*item, file, *enhanced, *copy_start, *source_start),
            C::SetClipEnhanced { item, on } => self.set_enhanced(*item, *on),
            C::FreezeFrame {
                item,
                at,
                len,
                still,
            } => self.freeze(*item, *at, *len, still.as_ref()),
            C::MergeClips { items } => self.merge(items),
            C::ConsolidateClips { items, file } => self.consolidate(items, file),
            C::RemoveClips { items, ripple } => self.remove_clips(items, *ripple),
            C::UpdateClip { item, patch } => self.update_clip(*item, patch),
            C::StyleTextRange {
                item,
                start,
                end,
                style,
            } => self.style_text(*item, *start, *end, style),
            C::ClearTextStyles { item, start, end } => self.clear_text_styles(*item, *start, *end),
            C::SetClipSpeed {
                item,
                speed,
                ripple,
            } => self.set_speed(*item, *speed, *ripple),
            C::SetClipSpeedCurve {
                item,
                curve,
                ripple,
            } => self.set_speed_curve(*item, curve.as_deref(), *ripple),
            C::SetClipTransform {
                item,
                offset_x,
                offset_y,
                rotation,
                scale,
                stretch_x,
                stretch_y,
            } => self.set_transform(
                *item,
                [
                    *offset_x, *offset_y, *rotation, *scale, *stretch_x, *stretch_y,
                ],
            ),
            C::FrameClip { item, framing, at } => self.frame_clip(*item, *framing, *at),
            C::DetachAudio { item } => self.detach(*item),
            C::ReattachAudio { item } => self.reattach(*item),
            C::AddTrack { bottom } => {
                let sequence = self.showing()?;
                let at = if *bottom {
                    place::Stack::Bottom
                } else {
                    place::Stack::Top
                };
                let lane = self.new_lane(sequence, at)?;
                self.out.created = Some(lane.any());
                Ok(())
            }
            C::RemoveTrack { lane } => self.remove_lane(*lane),
            C::SetTrackFlag { lane, flag, value } => self.set_lane_flag(*lane, *flag, *value),
            C::AddTimeline => self.add_timeline(),
            C::RemoveTimeline { sequence } => self.remove_timeline(*sequence),
            C::SetTimelineVideo { sequence, frame } => {
                self.sequence_mut(*sequence, |s| s.frame = frame.clone())
            }
            C::SetTimelineBackground {
                sequence,
                background,
            } => {
                let background = look::tidy_background(background);
                self.sequence_mut(*sequence, |s| s.background = background)
            }
            C::SetTimelineLinkage { sequence, on } => {
                self.sequence_mut(*sequence, |s| s.linkage = *on)
            }
            C::RenameTimeline { sequence, name } => {
                let name = name.trim();
                if name.is_empty() {
                    return Ok(());
                }
                self.sequence_mut(*sequence, |s| s.name = name.to_owned())
            }
            C::SelectTimeline { sequence } => self.select_timeline(*sequence),
            C::MoveTimeline { sequence, index } => self.move_timeline(*sequence, *index),
            C::SetCover { cover } => {
                let cover = cover.clone().map(|cover| match cover {
                    Cover::Frame { into } if into < TickDur::ZERO => Cover::Frame {
                        into: TickDur::ZERO,
                    },
                    other => other,
                });
                self.d.cover = cover;
                Ok(())
            }
            C::AddFont { family, path } => {
                if !self.d.fonts.iter().any(|font| font.path == *path) {
                    self.d.fonts.push(crate::model::Font {
                        family: family.clone(),
                        path: path.clone(),
                    });
                }
                Ok(())
            }
            C::RemoveFont { family } => {
                self.d.fonts.retain(|font| font.family != *family);
                Ok(())
            }
        }
    }

    /// A new id, recorded as made.
    pub(crate) fn mint<I: TypedId>(&mut self) -> Result<I, CommandError> {
        let id: I = self.d.mint()?;
        self.out.minted.push(id.any());
        Ok(id)
    }

    /// A copy of an item with new ids for everything it holds, recorded
    /// as made.
    pub(crate) fn copy_of(&mut self, item: &Item) -> Result<Item, CommandError> {
        let copy = item.with_new_ids(self.d)?;
        self.out.minted.extend(copy.nested_ids());
        Ok(copy)
    }

    fn created_item(&mut self, id: ItemId) {
        self.out.created = Some(id.any());
        self.out.selection = Some(vec![id]);
    }

    /// The item under an id.
    pub(crate) fn item(&self, id: ItemId) -> Result<Arc<Item>, CommandError> {
        self.d.items.get(id).cloned().ok_or(CommandError::Gone {
            what: "clip",
            id: id.any(),
        })
    }

    /// Changes an item where it is.
    pub(crate) fn update(
        &mut self,
        id: ItemId,
        change: impl FnOnce(&mut Item),
    ) -> Result<(), CommandError> {
        self.d.items.update(id, change)?;
        Ok(())
    }

    /// The sequence showing, which commands that name no sequence act on.
    pub(crate) fn showing(&self) -> Result<SequenceId, CommandError> {
        match self.d.view.showing {
            Some(sequence) if self.d.sequences.contains(sequence) => Ok(sequence),
            _ => refuse("There is no timeline open."),
        }
    }

    /// The sequence a lane is in.
    pub(crate) fn sequence_of(&self, lane: LaneId) -> Result<SequenceId, CommandError> {
        self.d
            .lanes
            .get(lane)
            .map(|lane| lane.sequence)
            .ok_or(CommandError::Gone {
                what: "lane",
                id: lane.any(),
            })
    }

    fn media_mut(
        &mut self,
        media: MediaId,
        change: impl FnOnce(&mut Media),
    ) -> Result<(), CommandError> {
        if self.d.media.contains(media) {
            self.d.media.update(media, change)?;
        }
        Ok(())
    }

    fn sequence_mut(
        &mut self,
        sequence: SequenceId,
        change: impl FnOnce(&mut crate::model::Sequence),
    ) -> Result<(), CommandError> {
        if self.d.sequences.contains(sequence) {
            self.d.sequences.update(sequence, change)?;
        }
        Ok(())
    }
}

/// The first line of some words, for a title's name.
pub(crate) fn first_line(words: &str) -> String {
    words.lines().next().unwrap_or("").trim().to_owned()
}

/// Effects given to an edit, with the ids they keep: an id the chain
/// already holds, the first time it comes, stays; any other is new.
pub(crate) fn keep_ids<I: TypedId, T>(
    ed: &mut Ed<'_>,
    held: &[I],
    given: Vec<T>,
    id: impl Fn(&mut T) -> &mut I,
) -> Result<Vec<T>, CommandError> {
    let mut seen: BTreeMap<I, ()> = BTreeMap::new();
    let mut out = Vec::with_capacity(given.len());
    for mut value in given {
        let current = *id(&mut value);
        if !held.contains(&current) || seen.insert(current, ()).is_some() {
            *id(&mut value) = ed.mint()?;
        }
        out.push(value);
    }
    Ok(out)
}
