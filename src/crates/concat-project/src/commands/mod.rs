// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Every edit operation, as data.
//!
//! A [`Command`] is what the window sends; [`apply`] is the one place its
//! meaning lives. The clamps and tolerances documented on each variant are
//! the contract the window's gesture echo mirrors and tests against.

use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::model::{
    AnimationSlot, AppliedFilter, AudioTrack, Clip, ClipAnimation, ClipKind, Crop, CustomFont,
    Cutout, CutoutMode, KeyEase, KeyProperty, MediaItem, MediaKind, Project, SpeedPoint, Stroke,
    TextStyle, Timeline, Track, Transition, VideoSettings,
};

mod audio;
mod clips;
pub use clips::why_not_merge;
mod media;
mod properties;
mod timelines;
mod tracks;

/// Fallback length for media whose container reports no duration.
const UNKNOWN_DURATION: f64 = 5.0;
/// How long a still lasts when first placed. Editorial default, not a fact.
const DEFAULT_IMAGE_DURATION: f64 = 5.0;
/// How long a title lasts when first placed.
const DEFAULT_TEXT_DURATION: f64 = 4.0;
/// How long a layer covers when first placed. Editorial default, not a fact.
const DEFAULT_LAYER_DURATION: f64 = 5.0;
/// Default hold for a CapCut-style freeze when the caller omits duration.
const DEFAULT_FREEZE_DURATION: f64 = 1.0;
pub(crate) use crate::model::ranges::{
    MAX_OFFSET, MAX_SCALE, MAX_SPEED, MAX_STRETCH, MIN_CLIP_DURATION, MIN_SCALE, MIN_SPEED,
    MIN_STRETCH, wrap_rotation,
};

/// Which end of a clip a trim drags. The two are not symmetric: see
/// [`Command::TrimClip`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrimEdge {
    /// The head. Trimming here moves the in-point with the edge, so the
    /// remaining frames stay where they were on the timeline.
    Start,
    /// The tail. Trimming here only lengthens or shortens the clip.
    End,
}

/// Which per-track toggle a [`Command::SetTrackFlag`] flips.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackFlag {
    /// [`Track::visible`]: whether the track's video reaches the composite.
    Visible,
    /// [`Track::muted`]: whether the track's audio is silenced.
    Muted,
}

/// Where one clip is going, in a multi-clip move.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipMove {
    /// The clip to move. An unknown id is skipped, not an error - the rest
    /// of the batch of moves still lands.
    pub clip_id: String,
    /// New timeline position in seconds, floored at 0.
    pub start: f64,
    /// Destination track. An unknown id moves the clip in time but leaves it
    /// on its current track.
    pub track_id: String,
}

/// A partial update to one clip. Every field optional; `transition_in` and
/// `text` are double-optional so "clear it" and "leave it alone" stay
/// distinct on the wire (absent = untouched, null = cleared).
#[derive(Clone, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipPatch {
    /// New display name, taken verbatim. A later `text` patch overwrites it
    /// with the title's first line.
    pub name: Option<String>,
    /// New gain; floored at 0, deliberately not capped at 1 - boosting quiet
    /// footage is legitimate.
    pub volume: Option<f64>,
    /// New fade-in length in seconds, floored at 0.
    pub fade_in: Option<f64>,
    /// New fade-out length in seconds, floored at 0.
    pub fade_out: Option<f64>,
    /// New opacity, clamped into 0..=1.
    pub opacity: Option<f64>,
    /// New pitch-preservation setting, taken as sent.
    pub preserve_pitch: Option<bool>,
    /// Whether the clip's own sound is silenced: true mutes it, false lets
    /// it play again. A detach sets this on the video it took the sound
    /// from, so this is also how a video whose detached sound was later
    /// deleted gets its voice back.
    #[serde(default)]
    pub muted: Option<bool>,
    /// Play backwards.
    #[serde(default)]
    pub reverse: Option<bool>,
    /// Mirror left to right.
    #[serde(default)]
    pub flip_h: Option<bool>,
    /// Mirror top to bottom.
    #[serde(default)]
    pub flip_v: Option<bool>,
    /// The blend mode's name; empty is normal.
    #[serde(default)]
    pub blend: Option<String>,
    /// The crop, same three-way wire semantics as `transition_in`: absent
    /// leaves it alone, null takes it off, a value replaces it.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub crop: Option<Option<Crop>>,
    /// Wholesale replacement of the audio filter chain - the UI sends the
    /// full list, not a diff.
    pub filters: Option<Vec<AppliedFilter>>,
    /// Wholesale replacement of the video effect chain, like `filters`.
    pub video_effects: Option<Vec<AppliedFilter>>,
    /// The transition on the cut into the clip: absent leaves it alone,
    /// null clears it, a value replaces it.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub transition_in: Option<Option<Transition>>,
    /// The title styling, same three-way wire semantics as `transition_in`.
    /// Setting a style also renames the clip after its first line.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub text: Option<Option<TextStyle>>,
    /// Which of the media's audio streams the clip plays, by stream index:
    /// absent leaves it alone, null goes back to the file's first, a value
    /// names one. See `Clip::audio_stream`.
    #[serde(
        default,
        with = "double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub audio_stream: Option<Option<u32>>,
}

/// `Option<Option<T>>` over JSON: absent → None, null → Some(None).
mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T: Serialize, S: Serializer>(
        value: &Option<Option<T>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, T: Deserialize<'de>, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(deserializer).map(Some)
    }
}

/// A media item as probed by the host, before the model mints its id.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMedia {
    /// Absolute path on disk. Adding a path already in the bin is a no-op,
    /// so re-imports cannot duplicate media.
    pub path: String,
    /// Display name for the bin, normally the file's basename.
    pub name: String,
    /// Seconds, or None when the container did not report one - clips of
    /// such media get a five-second fallback length.
    pub duration: Option<f64>,
    /// What the host's probe decided the file is.
    pub kind: MediaKind,
    /// Pixel width, when the probe found one.
    pub width: Option<u32>,
    /// Pixel height, same terms as `width`.
    pub height: Option<u32>,
    /// Frames per second as a decimal, for display.
    pub frame_rate: Option<f64>,
    /// The exact rate fraction the engine works in, e.g. "30000/1001".
    pub frame_rate_fraction: Option<String>,
    /// Codec name as probed, e.g. "h264". Informational.
    pub video_codec: Option<String>,
    /// Codec of the embedded audio, when there is any.
    pub audio_codec: Option<String>,
    /// Whether the file carries an audio stream.
    pub has_audio: bool,
    /// Every audio stream, in file order; see `MediaItem::audio_tracks`.
    /// Defaulted so a caller from before the list can still add media.
    #[serde(default)]
    pub audio_tracks: Vec<AudioTrack>,
}

/// Every edit, as the window sends it: a tagged `op` plus camelCase
/// fields. [`apply`] is the single place each variant's meaning lives; the
/// notes here state the contract - clamps, tolerances, what gets minted -
/// so a caller need not read `apply` to know what a command will do.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
// A clip patch carries a whole text style, which is the largest variant by
// a couple of hundred bytes. Commands are made one at a time and kept only
// in the undo history, where a few hundred bytes each is nothing; boxing
// the patch would cost every caller for a saving nobody would measure.
#[allow(clippy::large_enum_variant)]
pub enum Command {
    /// Imports a file into the bin, minting an "m" id. A path already
    /// present is a tolerated no-op that mints nothing.
    AddMedia {
        /// The probed file, as described by the host.
        item: NewMedia,
    },
    /// Removes a bin item and every clip referencing it, on *all* timelines
    /// - a clip whose media is gone would linger as a dead reference.
    RemoveMedia {
        /// The bin item to remove. An unknown id is a no-op.
        media_id: String,
    },
    /// Marks a bin item as a template slot (or back to ordinary media),
    /// which is what [`Command::FillSlot`] requires of its target.
    SetMediaPlaceholder {
        /// The bin item to mark. An unknown id is a no-op.
        media_id: String,
        /// True to make it a slot, false to make it ordinary media again.
        placeholder: bool,
    },
    /// Swaps the user's file into a template slot in place. The slot keeps
    /// its id so clips keep working; start, duration and speed stay the
    /// template's, while the in-point resets and each clip's kind and name
    /// follow the new file, across all timelines. Errs if the id is unknown
    /// or the item is not a placeholder.
    FillSlot {
        /// The slot being filled - must have `placeholder` set.
        media_id: String,
        /// The user's file that takes the slot's place.
        item: NewMedia,
    },
    /// Several commands as one atomic edit: applied to a staged copy and
    /// committed only if every one succeeds, then recorded as a single undo
    /// step. The outcome carries the last id minted inside.
    Batch {
        /// The commands, applied in order. Nesting is legal.
        commands: Vec<Command>,
    },
    /// Places a clip of `media_id` on a named track, minting a "c" id.
    /// Duration comes from the media (five seconds for a still or unknown
    /// length); errs if the media or track no longer exists.
    AddClip {
        /// The bin item to cut from.
        media_id: String,
        /// The lane to place it on.
        track_id: String,
        /// Timeline position in seconds, floored at 0.
        start: f64,
        /// When true and the drop would overlap something on this track,
        /// every clip at or after `start` is shifted right by the new
        /// clip's duration, so the drop lands without covering anything.
        /// False for programmatic adds, true for a drop from the bin.
        #[serde(default)]
        ripple: bool,
    },
    /// [`Command::AddClip`] without naming a lane: lands on the lowest
    /// track with nothing in the clip's span, falling back to the bottom
    /// track (overlap and all) rather than refusing.
    AddClipAtFirstFree {
        /// The bin item to cut from.
        media_id: String,
        /// Timeline position in seconds, floored at 0.
        start: f64,
    },
    /// Places a title: a clip with no media behind it, named after the
    /// text's first line, minting a "c" id.
    AddTextClip {
        /// The lane to place it on. None picks the first free track like
        /// [`Command::AddClipAtFirstFree`]; naming a vanished track errs.
        track_id: Option<String>,
        /// Timeline position in seconds, floored at 0.
        start: f64,
        /// The words and their look. None means [`TextStyle::default`].
        style: Option<TextStyle>,
        /// Seconds on the timeline; the editorial default when absent. Here
        /// so a caption run can land as one batch instead of add-then-trim
        /// per clip - a batch cannot trim a clip whose id it cannot know yet.
        #[serde(default)]
        duration: Option<f64>,
        /// Vertical placement as a frame-height fraction, clamped like
        /// SetClipTransform. Lower thirds are made of this.
        #[serde(default)]
        offset_y: Option<f64>,
    },
    /// Places a layer: a look or an effect over a span of the timeline that
    /// treats everything beneath it. No media; the chain starts as the one
    /// package named, at its defaults, and the clip's opacity is how hard it
    /// is applied.
    AddLayerClip {
        /// The lane to place it on; None picks the first free track.
        track_id: Option<String>,
        /// Timeline position in seconds, floored at 0.
        start: f64,
        /// Seconds on the timeline; the editorial default when absent.
        #[serde(default)]
        duration: Option<f64>,
        /// The package the layer applies, e.g. "concat.warm".
        effect_id: String,
        /// What the lane calls it.
        name: String,
    },
    /// Gives a clip a speed curve, or takes it away. The source covered is
    /// held constant, as a speed change does, so the clip's timeline length
    /// follows the curve's mean; `speed` is kept at that mean.
    SetClipSpeedCurve {
        /// The clip to retime.
        clip_id: String,
        /// The curve, or None for a constant rate at the current mean.
        curve: Option<Vec<SpeedPoint>>,
    },
    /// Sets or clears the animation on one slot of a clip.
    SetClipAnimation {
        /// The clip.
        clip_id: String,
        /// Which end, or the whole.
        slot: AnimationSlot,
        /// The shape and its seconds, or None to take it off.
        animation: Option<ClipAnimation>,
    },
    /// Puts a key on one property at one point of a clip, replacing
    /// whichever key on that property was already within a hair of it.
    ///
    /// The value is the property's own - the number the inspector shows -
    /// not the relative factor the engine plays; see `model::ClipKey`.
    SetClipKey {
        /// The clip.
        clip_id: String,
        /// Which property is being keyed.
        property: KeyProperty,
        /// Where in the clip, `0..=1`.
        at: f64,
        /// The value there.
        value: f64,
        /// How the key is approached from the one before it.
        #[serde(default)]
        ease: KeyEase,
    },
    /// Takes the key at `at` off one property, if there is one there. The
    /// other half of the diamond: a filled diamond clicked is this.
    ClearClipKey {
        /// The clip.
        clip_id: String,
        /// Which property.
        property: KeyProperty,
        /// Where in the clip the key to remove sits, `0..=1`.
        at: f64,
    },
    /// Takes every key off one property, returning it to its constant.
    ClearClipKeys {
        /// The clip.
        clip_id: String,
        /// Which property.
        property: KeyProperty,
    },
    /// Puts a key on one parameter of one link of a clip's picture chain,
    /// replacing whichever key on it was already within a hair of `at`.
    /// The value is the parameter's own; the package's range is the
    /// caller's to keep, the model not knowing it.
    SetEffectKey {
        /// The clip.
        clip_id: String,
        /// Which link, as an index into `video_effects`. Out of range is a
        /// no-op.
        entry: usize,
        /// The parameter's manifest key.
        key: String,
        /// Where in the clip, `0..=1`.
        at: f64,
        /// The value there.
        value: f64,
        /// How the key is approached from the one before it.
        #[serde(default)]
        ease: KeyEase,
    },
    /// Takes the key at `at` off one parameter of one link, if there is one.
    ClearEffectKey {
        /// The clip.
        clip_id: String,
        /// Which link, as an index into `video_effects`.
        entry: usize,
        /// The parameter's manifest key.
        key: String,
        /// Where in the clip the key to remove sits, `0..=1`.
        at: f64,
    },
    /// Takes every key off one parameter of one link, returning it to the
    /// value it holds.
    ClearEffectKeys {
        /// The clip.
        clip_id: String,
        /// Which link, as an index into `video_effects`.
        entry: usize,
        /// The parameter's manifest key.
        key: String,
    },
    /// Sets or clears a picture's cutout: the mask that takes its
    /// background away. Tidied on the way in; see [`Cutout::tidy`].
    SetClipCutout {
        /// The clip. An unknown id is a no-op.
        clip_id: String,
        /// The cutout, or None to take it off.
        cutout: Option<Cutout>,
    },
    /// Paints one stroke onto a clip's cutout. A clip with no cutout gets a
    /// custom one; an automatic cutout becomes custom, since a stroke is a
    /// correction to it. A stroke with no points is a no-op.
    AddCutoutStroke {
        /// The clip. An unknown id is a no-op.
        clip_id: String,
        /// The stroke, in source fractions.
        stroke: Stroke,
    },
    /// Repositions any number of clips in one edit - one undo step for a
    /// whole multi-selection drag. Unknown clips and tracks are tolerated
    /// per [`ClipMove`].
    MoveClips {
        /// Where each clip is going.
        moves: Vec<ClipMove>,
    },
    /// Drags one edge of a clip. A head trim moves the in-point with the
    /// edge (scaled by speed) so the remaining pixels do not slide; either
    /// edge stops at the sixtieth-of-a-second minimum duration. An unknown
    /// clip is a no-op.
    TrimClip {
        /// The clip to trim.
        clip_id: String,
        /// Which edge is being dragged.
        edge: TrimEdge,
        /// Signed seconds of timeline the edge moves: positive drags the
        /// head right (shortening) or the tail right (lengthening).
        delta: f64,
    },
    /// Cuts each named clip in two at one playhead time. The head keeps the
    /// id and the transition; the tail is minted fresh and stays
    /// source-continuous. A clip the time misses (or grazes within the
    /// minimum duration) is skipped.
    SplitClips {
        /// The clips under the playhead - normally the selection.
        clip_ids: Vec<String>,
        /// The cut point, in timeline seconds.
        time: f64,
    },
    /// CapCut-style freeze at `time`: splits `clip_id`, inserts a still of
    /// `duration` on the same track, and ripples later clips on that track
    /// by `duration`. Video needs a probed `still` (host-extracted jpg);
    /// image clips may omit it and reuse their media. Audio and text are
    /// no-ops. `created_id` is the freeze clip.
    FreezeFrame {
        /// The picture clip under the playhead.
        clip_id: String,
        /// Timeline playhead; must fall strictly inside the clip.
        time: f64,
        /// Editorial length of the hold. Floored at [`MIN_CLIP_DURATION`];
        /// when absent or non-positive, uses [`DEFAULT_FREEZE_DURATION`].
        #[serde(default)]
        duration: Option<f64>,
        /// Probed still file. Required for video; ignored for image when
        /// reusing the existing media.
        #[serde(default)]
        still: Option<NewMedia>,
    },
    /// Rejoins split pieces into the earliest piece, which keeps its id.
    /// Errs with a user-facing sentence ([`why_not_merge`]) unless the
    /// pieces sit on one track, come from one file at one speed, touch
    /// within a microsecond, and are still in source order.
    MergeClips {
        /// The pieces to rejoin, any order.
        clip_ids: Vec<String>,
    },
    /// Deletes clips from the active timeline. Unknown ids are ignored.
    RemoveClips {
        /// The clips to delete.
        clip_ids: Vec<String>,
    },
    /// Applies a [`ClipPatch`]: only the fields present change, with the
    /// clamps documented on the patch. An unknown clip is a no-op.
    UpdateClip {
        /// The clip to patch.
        clip_id: String,
        /// Which properties change, and to what.
        patch: ClipPatch,
    },
    /// Changes playback rate while holding the amount of source covered
    /// constant - the clip's timeline duration is what stretches, which is
    /// what makes this a speed change rather than a trim.
    SetClipSpeed {
        /// The clip to retime. An unknown id is a no-op.
        clip_id: String,
        /// The new rate, clamped into 0.0625..=16 (the engine's range).
        speed: f64,
    },
    /// Adjusts the picture's placement. Each field is optional so a drag
    /// can send just the axis it moved; absent fields stay put.
    SetClipTransform {
        /// The clip to place. An unknown id is a no-op.
        clip_id: String,
        /// New scale, clamped into 0.05..=8.
        scale: Option<f64>,
        /// New horizontal offset as a frame-width fraction, clamped to ±3.
        offset_x: Option<f64>,
        /// New vertical offset as a frame-height fraction, clamped to ±3.
        offset_y: Option<f64>,
        /// New rotation in degrees, wrapped into (-180, 180] so a full drag
        /// never accumulates turns.
        rotation: Option<f64>,
        /// New width multiplier beyond the scale, clamped into 0.1..=10.
        #[serde(default)]
        stretch_x: Option<f64>,
        /// New height multiplier, on the same terms.
        #[serde(default)]
        stretch_y: Option<f64>,
    },
    /// Pulls a video clip's sound out into its own audio clip on a free
    /// lane (minting one if none is free), muting the video and moving its
    /// audio filters to the sound. A no-op unless the clip is an unmuted
    /// video whose media has audio and is not already detached.
    DetachAudio {
        /// The video clip to detach from.
        clip_id: String,
    },
    /// Undoes a detach: deletes the detached sound clip(s), unmutes the
    /// video and hands the sound's filters back. Accepts either the video's
    /// id or the sound's; a no-op when either side is gone.
    ReattachAudio {
        /// The video clip - or its detached sound.
        clip_id: String,
    },
    /// Appends a lane named after the highest "Track N" in use, minting a
    /// "t" id.
    AddTrack,
    /// Deletes a lane and every clip on it. Errs at the floor of one track.
    RemoveTrack {
        /// The lane to delete.
        track_id: String,
    },
    /// Flips one of a track's two toggles. An unknown id is a no-op.
    SetTrackFlag {
        /// The lane to change.
        track_id: String,
        /// Which toggle: visibility or mute.
        flag: TrackFlag,
        /// The new setting.
        value: bool,
    },
    /// Adds a fresh timeline - four new lanes, "Timeline N" after the
    /// highest in use - and makes it active. Mints a "tl" id.
    AddTimeline,
    /// Deletes a timeline, moving the active tab to a neighbour if it was
    /// this one. Errs at the floor of one timeline.
    RemoveTimeline {
        /// The timeline to delete. An unknown id is a no-op.
        timeline_id: String,
    },
    /// Sets a timeline's output frame and rate. A term no frame could have,
    /// such as a zero dimension or a zero rate, is refused as a no-op rather
    /// than clamped: there is no nearest real size to a zero.
    SetTimelineVideo {
        /// The timeline. An unknown id is a no-op.
        timeline_id: String,
        /// The frame and the rate, together.
        video: VideoSettings,
    },
    /// Renames a timeline tab. Whitespace-only names are ignored so a tab
    /// can never end up blank; unknown ids are tolerated.
    RenameTimeline {
        /// The timeline to rename.
        timeline_id: String,
        /// The new tab label; trimmed before it lands.
        name: String,
    },
    /// Switches which timeline subsequent commands act on. An unknown id
    /// leaves the selection where it was.
    SelectTimeline {
        /// The timeline to switch to.
        timeline_id: String,
    },
    /// Moves a timeline tab to a new position in the strip. Which timeline
    /// is active does not change - only the order of the tabs.
    MoveTimeline {
        /// The timeline to move. An unknown id is a no-op.
        timeline_id: String,
        /// Where it lands among its siblings, 0-based, counted with the
        /// timeline already removed from its old slot. Clamped to the end.
        index: usize,
    },
    /// Registers a font file for titles. A path already registered is a
    /// no-op, so re-adding cannot duplicate.
    AddFont {
        /// The family name titles will refer to.
        family: String,
        /// Where the font file lives on disk.
        path: String,
    },
    /// Unregisters a font family. Clips keep the family name: the face may
    /// come back when the file does.
    RemoveFont {
        /// The family to unregister.
        family: String,
    },
    /// Updates a media item's path on disk (relink). An unknown id is a no-op.
    UpdateMediaPath {
        /// The media item to update.
        media_id: String,
        /// The new absolute path on disk.
        new_path: String,
    },
}

/// What a command produced, beyond the new state: the ids it minted, so the
/// UI can select what it just created, and whether anything changed at all.
#[derive(Clone, Default, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// The id of what the command created - a clip, track, timeline, or
    /// media item - or, for a batch, the last id minted inside it. Absent
    /// when nothing was created, including tolerated no-ops.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_id: Option<String>,
    /// Whether the command actually changed the project. A tolerated no-op -
    /// a missing id, a field set to the value it already had - succeeds but
    /// reports false, which is how the editor knows not to record an undo
    /// snapshot for it. For a batch: whether any member changed anything.
    #[serde(default)]
    pub applied: bool,
}

/// Why a command was refused. Every variant renders through `Display` as the
/// exact user-facing sentence the UI shows - byte for byte the strings
/// the window treats as the contract; the enum only gives
/// those sentences names a host can match on.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum CommandError {
    /// [`Command::FillSlot`] named a media id no longer in the bin.
    #[error("That template slot no longer exists.")]
    SlotGone,
    /// [`Command::FillSlot`] targeted ordinary media rather than a slot.
    #[error("That media is not a template slot.")]
    NotASlot,
    /// A clip-placing command named media no longer in the bin.
    #[error("That media is no longer in the bin.")]
    MediaGone,
    /// A clip-placing command named a track no longer on the timeline.
    #[error("That track no longer exists.")]
    TrackGone,
    /// A first-free-track placement found a timeline with no tracks at all.
    #[error("There are no tracks.")]
    NoTracks,
    /// [`Command::MergeClips`] was refused, for whichever [`why_not_merge`]
    /// reason applied.
    #[error("{reason}")]
    CannotMerge {
        /// The [`why_not_merge`] sentence, verbatim.
        reason: String,
    },
    /// [`Command::RemoveTrack`] would have deleted the last track.
    #[error("A timeline needs at least one track.")]
    LastTrack,
    /// [`Command::RemoveTimeline`] would have deleted the last timeline.
    #[error("A project needs at least one timeline.")]
    LastTimeline,
    /// A number in the command is NaN or infinite. No edit means one, and
    /// one stored would poison every duration and key that touched it.
    #[error("A number in that edit is not finite.")]
    NotANumber,
}

/// Mints ids. Owned by the editor so restored projects advance it past every
/// id a file already uses - the collision class `adoptProject` fixed in the
/// UI is prevented here instead.
#[derive(Clone, Default, Debug)]
pub struct IdMint {
    counter: u64,
}

impl IdMint {
    /// The next fresh id: the prefix ("c", "t", "tl", "m") plus a counter
    /// shared across all prefixes, so no two ids ever share a number.
    pub fn next(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}{}", self.counter)
    }

    /// Advances the counter past `id`'s numeric suffix, if it has one.
    pub fn adopt(&mut self, id: &str) {
        let digits: String = id
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if let Ok(value) = digits.parse::<u64>() {
            self.counter = self.counter.max(value);
        }
    }

    /// Adopts every id a restored project uses - media, timelines, tracks,
    /// clips - so nothing minted afterwards can collide with the file.
    pub fn adopt_project(&mut self, project: &Project) {
        for item in &project.media {
            self.adopt(&item.id);
        }
        for timeline in &project.timelines {
            self.adopt(&timeline.id);
            for track in &timeline.tracks {
                self.adopt(&track.id);
            }
            for clip in &timeline.clips {
                self.adopt(&clip.id);
            }
        }
    }
}

/// A one-line label for a block of text.
fn first_line(content: &str) -> String {
    let line = content
        .lines()
        .find(|candidate| !candidate.trim().is_empty())
        .unwrap_or("Text")
        .trim();
    let label: String = line.chars().take(40).collect();
    if label.is_empty() {
        "Text".to_owned()
    } else {
        label
    }
}

/// "Track 5" from the highest number already in use, not from the count.
/// Only names of the exact shape `"{name} N"` count: a renamed "Timeline 2
/// v10" is somebody's own name, not a number in the sequence.
fn next_numbered(name: &str, existing: impl Iterator<Item = String>) -> String {
    let highest = existing
        .filter_map(|candidate| {
            candidate
                .strip_prefix(name)
                .and_then(|rest| rest.strip_prefix(' '))
                .and_then(|digits| digits.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0);
    format!("{name} {}", highest + 1)
}

impl Command {
    /// True for a command that changes what the person is looking at, not
    /// the edit: which timeline tab is showing and the order of the tabs.
    /// Saved with the document, never recorded in undo history.
    pub fn is_view_state(&self) -> bool {
        matches!(
            self,
            Command::SelectTimeline { .. } | Command::MoveTimeline { .. }
        )
    }

    /// True when any number in the command is NaN or infinite. JSON cannot
    /// spell one, but a caller in Rust can, and `NaN.clamp()` is NaN: one
    /// stored would poison every duration and key it touched, and the
    /// document reader would only repair it on the next load.
    pub fn has_non_finite(&self) -> bool {
        fn bad(values: impl IntoIterator<Item = f64>) -> bool {
            values.into_iter().any(|value| !value.is_finite())
        }
        fn bad_stroke(stroke: &Stroke) -> bool {
            bad([stroke.size]) || bad(stroke.at) || stroke.points.iter().any(|point| bad(*point))
        }
        fn bad_chain(chain: &[AppliedFilter]) -> bool {
            chain.iter().any(|entry| {
                bad(entry.params.values().copied())
                    || entry
                        .keys
                        .values()
                        .flatten()
                        .any(|key| bad([key.at, key.value]) || bad(key.ease.0))
            })
        }
        match self {
            Command::Batch { commands } => commands.iter().any(Command::has_non_finite),
            Command::AddMedia { item } | Command::FillSlot { item, .. } => {
                bad(item.duration) || bad(item.frame_rate)
            }
            Command::AddClip { start, .. } | Command::AddClipAtFirstFree { start, .. } => {
                bad([*start])
            }
            Command::AddTextClip {
                start,
                duration,
                offset_y,
                ..
            } => bad([*start]) || bad(*duration) || bad(*offset_y),
            Command::AddLayerClip {
                start, duration, ..
            } => bad([*start]) || bad(*duration),
            Command::SetClipSpeedCurve { curve, .. } => bad(curve
                .iter()
                .flatten()
                .flat_map(|point| [point.at, point.speed])),
            Command::SetClipAnimation { animation, .. } => {
                bad(animation.iter().map(|animation| animation.duration))
            }
            Command::SetClipKey {
                at, value, ease, ..
            }
            | Command::SetEffectKey {
                at, value, ease, ..
            } => bad([*at, *value]) || bad(ease.0),
            Command::ClearClipKey { at, .. } | Command::ClearEffectKey { at, .. } => bad([*at]),
            Command::SetClipCutout { cutout, .. } => cutout.as_ref().is_some_and(|cutout| {
                bad([cutout.feather]) || cutout.strokes.iter().any(bad_stroke)
            }),
            Command::AddCutoutStroke { stroke, .. } => bad_stroke(stroke),
            Command::MoveClips { moves } => bad(moves.iter().map(|wanted| wanted.start)),
            Command::TrimClip { delta, .. } => bad([*delta]),
            Command::SplitClips { time, .. } => bad([*time]),
            Command::FreezeFrame { time, duration, .. } => bad([*time]) || bad(*duration),
            Command::UpdateClip { patch, .. } => {
                bad(patch.volume)
                    || bad(patch.fade_in)
                    || bad(patch.fade_out)
                    || bad(patch.opacity)
                    || patch
                        .crop
                        .iter()
                        .flatten()
                        .any(|crop| bad([crop.left, crop.top, crop.right, crop.bottom]))
                    || patch.filters.as_deref().is_some_and(bad_chain)
                    || patch.video_effects.as_deref().is_some_and(bad_chain)
            }
            Command::SetClipSpeed { speed, .. } => bad([*speed]),
            Command::SetClipTransform {
                scale,
                offset_x,
                offset_y,
                rotation,
                stretch_x,
                stretch_y,
                ..
            } => bad([scale, offset_x, offset_y, rotation, stretch_x, stretch_y]
                .into_iter()
                .flatten()
                .copied()),
            _ => false,
        }
    }
}

/// Assigns `value` into `slot`, reporting whether that changed anything.
/// This is how command arms notice a field being set to the value it already
/// holds - which must count as "nothing happened" ([`Outcome::applied`]
/// false), or the undo history would record phantom edits.
fn assign<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        false
    } else {
        *slot = value;
        true
    }
}

/// Applies one command. Errors are [`CommandError`]s, each rendering as a
/// user-meaningful sentence; a command that legitimately does nothing (a
/// no-op rename, an out-of-range split) returns Ok with no created id and
/// [`Outcome::applied`] false.
pub fn apply(
    project: &mut Project,
    mint: &mut IdMint,
    command: Command,
) -> Result<Outcome, CommandError> {
    if command.has_non_finite() {
        return Err(CommandError::NotANumber);
    }
    match command {
        Command::Batch { commands } => {
            // All or nothing: apply to a staged copy and commit only a fully
            // successful run, so one bad command cannot leave a half-applied
            // batch behind (and the editor records it as one undo step).
            let mut staged = project.clone();
            let mut created = None;
            let mut applied = false;
            for command in commands {
                let outcome = apply(&mut staged, mint, command)?;
                if outcome.created_id.is_some() {
                    created = outcome.created_id;
                }
                applied |= outcome.applied;
            }
            *project = staged;
            Ok(Outcome {
                created_id: created,
                applied,
            })
        }

        command @ (Command::AddMedia { .. }
        | Command::SetMediaPlaceholder { .. }
        | Command::FillSlot { .. }
        | Command::RemoveMedia { .. }
        | Command::AddFont { .. }
        | Command::RemoveFont { .. }
        | Command::UpdateMediaPath { .. }) => media::apply(project, mint, command),
        command @ (Command::AddClip { .. }
        | Command::AddClipAtFirstFree { .. }
        | Command::AddTextClip { .. }
        | Command::AddLayerClip { .. }
        | Command::MoveClips { .. }
        | Command::TrimClip { .. }
        | Command::SplitClips { .. }
        | Command::FreezeFrame { .. }
        | Command::MergeClips { .. }
        | Command::RemoveClips { .. }) => clips::apply(project, mint, command),
        command @ (Command::UpdateClip { .. }
        | Command::SetClipSpeed { .. }
        | Command::SetClipCutout { .. }
        | Command::AddCutoutStroke { .. }
        | Command::SetClipAnimation { .. }
        | Command::SetClipKey { .. }
        | Command::ClearClipKey { .. }
        | Command::ClearClipKeys { .. }
        | Command::SetEffectKey { .. }
        | Command::ClearEffectKey { .. }
        | Command::ClearEffectKeys { .. }
        | Command::SetClipSpeedCurve { .. }
        | Command::SetClipTransform { .. }) => properties::apply(project, mint, command),
        command @ (Command::DetachAudio { .. } | Command::ReattachAudio { .. }) => {
            audio::apply(project, mint, command)
        }
        command @ (Command::AddTrack
        | Command::RemoveTrack { .. }
        | Command::SetTrackFlag { .. }) => tracks::apply(project, mint, command),
        command @ (Command::AddTimeline
        | Command::SetTimelineVideo { .. }
        | Command::RemoveTimeline { .. }
        | Command::RenameTimeline { .. }
        | Command::SelectTimeline { .. }
        | Command::MoveTimeline { .. }) => timelines::apply(project, mint, command),
    }
}
