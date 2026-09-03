// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Every edit operation, as data.
//!
//! A [`Command`] is what the window sends; [`apply`] is the one place its
//! meaning lives. The clamps and tolerances documented on each variant are
//! the contract the window's gesture echo mirrors and tests against.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::model::{
    AppliedFilter, Clip, ClipKind, CustomFont, MediaItem, MediaKind, Project, TextStyle, Timeline,
    Track, Transition,
};

/// Fallback length for media whose container reports no duration.
const UNKNOWN_DURATION: f64 = 5.0;
/// How long a still lasts when first placed. Editorial default, not a fact.
const DEFAULT_IMAGE_DURATION: f64 = 5.0;
/// How long a title lasts when first placed.
const DEFAULT_TEXT_DURATION: f64 = 4.0;
const MIN_CLIP_DURATION: f64 = 1.0 / 60.0;
/// The engine's speed range (concat-media `SPEED_RANGE`), verbatim.
const MIN_SPEED: f64 = 0.0625;
const MAX_SPEED: f64 = 16.0;
const MIN_SCALE: f64 = 0.05;
const MAX_SCALE: f64 = 8.0;
const MAX_OFFSET: f64 = 3.0;
/// How far apart two clips may sit and still count as touching, in seconds.
const JOIN_EPSILON: f64 = 1e-6;

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
}

/// Every edit, as the window sends it: a tagged `op` plus camelCase
/// fields. [`apply`] is the single place each variant's meaning lives; the
/// notes here state the contract - clamps, tolerances, what gets minted -
/// so a caller need not read `apply` to know what a command will do.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
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
    /// Renames a lane. Whitespace-only names are ignored so a track can
    /// never end up blank; unknown ids are tolerated.
    RenameTrack {
        /// The lane to rename.
        track_id: String,
        /// The new label; trimmed before it lands.
        name: String,
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
    /// Renames a timeline tab, with the same trim-and-ignore-blank rule as
    /// [`Command::RenameTrack`].
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

fn default_clip(id: String, track_id: String, media: &MediaItem, start: f64) -> Clip {
    Clip {
        id,
        track_id,
        media_id: media.id.clone(),
        name: media.name.clone(),
        kind: match media.kind {
            MediaKind::Video => ClipKind::Video,
            MediaKind::Audio => ClipKind::Audio,
            MediaKind::Image => ClipKind::Image,
        },
        start: start.max(0.0),
        duration: match media.kind {
            MediaKind::Image => DEFAULT_IMAGE_DURATION,
            _ => media.duration.unwrap_or(UNKNOWN_DURATION),
        },
        source_start: 0.0,
        volume: 1.0,
        fade_in: 0.0,
        fade_out: 0.0,
        scale: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
        rotation: 0.0,
        opacity: 1.0,
        speed: 1.0,
        preserve_pitch: true,
        filters: Vec::new(),
        video_effects: Vec::new(),
        muted: None,
        detached_from: None,
        transition_in: None,
        text: None,
    }
}

/// The lowest track with nothing occupying `[start, start + duration)`,
/// falling back to the bottom track.
fn first_free_track(timeline: &Timeline, start: f64, duration: f64) -> Option<String> {
    let end = start + duration;
    timeline
        .tracks
        .iter()
        .find(|track| {
            !timeline.clips.iter().any(|clip| {
                clip.track_id == track.id && clip.start < end && start < clip.start + clip.duration
            })
        })
        .or(timeline.tracks.first())
        .map(|track| track.id.clone())
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
fn next_numbered(name: &str, existing: impl Iterator<Item = String>) -> String {
    let highest = existing
        .filter_map(|candidate| {
            let digits: String = candidate.chars().filter(char::is_ascii_digit).collect();
            digits.parse::<u64>().ok()
        })
        .max()
        .unwrap_or(0);
    format!("{name} {}", highest + 1)
}

/// Why these clips cannot be merged, or None if they can. A sentence, because
/// a disabled button that will not say why is worse than no button.
pub fn why_not_merge(timeline: &Timeline, clip_ids: &[String]) -> Option<String> {
    if clip_ids.len() < 2 {
        return Some("Select two or more clips to merge.".to_owned());
    }
    let clips: Vec<&Clip> = clip_ids.iter().filter_map(|id| timeline.clip(id)).collect();
    if clips.len() < 2 {
        return Some("Select two or more clips to merge.".to_owned());
    }
    if clips.iter().any(|clip| clip.track_id != clips[0].track_id) {
        return Some("Merged clips must be on the same track.".to_owned());
    }
    if clips.iter().any(|clip| clip.media_id != clips[0].media_id) {
        return Some("Merged clips must come from the same file.".to_owned());
    }
    if clips.iter().any(|clip| clip.speed != clips[0].speed) {
        return Some("Merged clips must play at the same speed.".to_owned());
    }

    let mut ordered = clips.clone();
    ordered.sort_by(|left, right| left.start.total_cmp(&right.start));
    for pair in ordered.windows(2) {
        let (previous, current) = (pair[0], pair[1]);
        if (current.start - (previous.start + previous.duration)).abs() > JOIN_EPSILON {
            return Some("Merged clips must touch, with no gap or overlap.".to_owned());
        }
        if (current.source_start - (previous.source_start + previous.duration * previous.speed))
            .abs()
            > JOIN_EPSILON
        {
            return Some("These pieces are no longer in their original order.".to_owned());
        }
    }
    None
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
    match command {
        Command::AddMedia { item } => {
            if project
                .media
                .iter()
                .any(|existing| existing.path == item.path)
            {
                return Ok(Outcome::default());
            }
            let id = mint.next("m");
            project.media.push(MediaItem {
                id: id.clone(),
                path: item.path,
                name: item.name,
                duration: item.duration,
                kind: item.kind,
                width: item.width,
                height: item.height,
                frame_rate: item.frame_rate,
                frame_rate_fraction: item.frame_rate_fraction,
                video_codec: item.video_codec,
                audio_codec: item.audio_codec,
                has_audio: item.has_audio,
                placeholder: false,
            });
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::SetMediaPlaceholder {
            media_id,
            placeholder,
        } => {
            let applied = project
                .media
                .iter_mut()
                .find(|item| item.id == media_id)
                .is_some_and(|item| assign(&mut item.placeholder, placeholder));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::FillSlot { media_id, item } => {
            let media = project
                .media
                .iter_mut()
                .find(|existing| existing.id == media_id)
                .ok_or(CommandError::SlotGone)?;
            if !media.placeholder {
                return Err(CommandError::NotASlot);
            }

            // The slot keeps its id, so every clip that references it keeps
            // working; only the identity behind the id changes.
            media.path = item.path;
            media.name = item.name.clone();
            media.duration = item.duration;
            media.kind = item.kind;
            media.width = item.width;
            media.height = item.height;
            media.frame_rate = item.frame_rate;
            media.frame_rate_fraction = item.frame_rate_fraction;
            media.video_codec = item.video_codec;
            media.audio_codec = item.audio_codec;
            media.has_audio = item.has_audio;
            media.placeholder = false;
            let kind = match item.kind {
                MediaKind::Video => ClipKind::Video,
                MediaKind::Audio => ClipKind::Audio,
                MediaKind::Image => ClipKind::Image,
            };

            // Slot timing is the template's: start, duration and speed stay
            // put, which is what keeps cuts on the beat. The in-point resets
            // because it referred to the old footage; a clip shorter than its
            // slot freeze-frames on its last frame downstream, which is the
            // renderer's existing behaviour for a trim past the media's end.
            // All timelines, like RemoveMedia: slots are not per-timeline.
            for timeline in &mut project.timelines {
                for clip in &mut timeline.clips {
                    if clip.media_id == media_id {
                        clip.source_start = 0.0;
                        clip.kind = kind;
                        clip.name = item.name.clone();
                    }
                }
            }
            // Filling always changes the project: the target was a
            // placeholder and is one no longer.
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

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

        Command::RemoveMedia { media_id } => {
            // All timelines, not just the active one: a shelved clip whose
            // media is gone would linger as a dead reference.
            let media_count = project.media.len();
            project.media.retain(|item| item.id != media_id);
            let mut applied = project.media.len() != media_count;
            for timeline in &mut project.timelines {
                let clip_count = timeline.clips.len();
                timeline.clips.retain(|clip| clip.media_id != media_id);
                applied |= timeline.clips.len() != clip_count;
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::AddClip {
            media_id,
            track_id,
            start,
        } => {
            let media = project
                .media_by_id(&media_id)
                .ok_or(CommandError::MediaGone)?
                .clone();
            let timeline = project.active_mut();
            if timeline.track(&track_id).is_none() {
                return Err(CommandError::TrackGone);
            }
            let id = mint.next("c");
            timeline
                .clips
                .push(default_clip(id.clone(), track_id, &media, start));
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::AddClipAtFirstFree { media_id, start } => {
            let media = project
                .media_by_id(&media_id)
                .ok_or(CommandError::MediaGone)?
                .clone();
            let duration = match media.kind {
                MediaKind::Image => DEFAULT_IMAGE_DURATION,
                _ => media.duration.unwrap_or(UNKNOWN_DURATION),
            };
            let timeline = project.active_mut();
            let track_id =
                first_free_track(timeline, start, duration).ok_or(CommandError::NoTracks)?;
            let id = mint.next("c");
            timeline
                .clips
                .push(default_clip(id.clone(), track_id, &media, start));
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::AddTextClip {
            track_id,
            start,
            style,
            duration,
            offset_y,
        } => {
            let style = style.unwrap_or_default();
            let duration = duration
                .unwrap_or(DEFAULT_TEXT_DURATION)
                .max(MIN_CLIP_DURATION);
            let timeline = project.active_mut();
            let track_id = match track_id {
                Some(id) if timeline.track(&id).is_some() => id,
                Some(_) => return Err(CommandError::TrackGone),
                None => {
                    first_free_track(timeline, start, duration).ok_or(CommandError::NoTracks)?
                }
            };
            let id = mint.next("c");
            timeline.clips.push(Clip {
                id: id.clone(),
                track_id,
                media_id: String::new(),
                name: first_line(&style.content),
                kind: ClipKind::Text,
                start: start.max(0.0),
                duration,
                source_start: 0.0,
                volume: 1.0,
                fade_in: 0.0,
                fade_out: 0.0,
                scale: 1.0,
                offset_x: 0.0,
                offset_y: offset_y.unwrap_or(0.0).clamp(-MAX_OFFSET, MAX_OFFSET),
                rotation: 0.0,
                opacity: 1.0,
                speed: 1.0,
                preserve_pitch: true,
                filters: Vec::new(),
                video_effects: Vec::new(),
                muted: None,
                detached_from: None,
                transition_in: None,
                text: Some(style),
            });
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::MoveClips { moves } => {
            let timeline = project.active_mut();
            let track_ids: HashSet<String> = timeline
                .tracks
                .iter()
                .map(|track| track.id.clone())
                .collect();
            let mut applied = false;
            for wanted in moves {
                if let Some(clip) = timeline.clip_mut(&wanted.clip_id) {
                    applied |= assign(&mut clip.start, wanted.start.max(0.0));
                    if track_ids.contains(&wanted.track_id) {
                        applied |= assign(&mut clip.track_id, wanted.track_id);
                    }
                }
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::TrimClip {
            clip_id,
            edge,
            delta,
        } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let applied = match edge {
                TrimEdge::End => {
                    let duration = (clip.duration + delta).max(MIN_CLIP_DURATION);
                    assign(&mut clip.duration, duration)
                }
                TrimEdge::Start => {
                    // Dragging the head moves the in-point too, so the pixels
                    // under the remaining part of the clip do not slide.
                    let shift = delta.min(clip.duration - MIN_CLIP_DURATION);
                    let start = (clip.start + shift).max(0.0);
                    let moved = start - clip.start;
                    let duration = clip.duration - moved;
                    let source_start = (clip.source_start + moved * clip.speed).max(0.0);
                    // Bitwise so no assignment is short-circuited away.
                    assign(&mut clip.start, start)
                        | assign(&mut clip.duration, duration)
                        | assign(&mut clip.source_start, source_start)
                }
            };
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SplitClips { clip_ids, time } => {
            let timeline = project.active_mut();
            let mut created = None;
            for clip_id in clip_ids {
                let Some(index) = timeline.clips.iter().position(|clip| clip.id == clip_id) else {
                    continue;
                };
                let clip = &timeline.clips[index];
                let offset = time - clip.start;
                if offset <= MIN_CLIP_DURATION || offset >= clip.duration - MIN_CLIP_DURATION {
                    continue;
                }
                let mut tail = clip.clone();
                tail.id = mint.next("c");
                tail.start = clip.start + offset;
                tail.duration = clip.duration - offset;
                tail.source_start = clip.source_start + offset * clip.speed;
                // The transition belongs to the cut at the original clip's
                // start, which the head keeps.
                tail.transition_in = None;
                created = Some(tail.id.clone());
                timeline.clips[index].duration = offset;
                timeline.clips.insert(index + 1, tail);
            }
            // A split always mints the tail, so "minted anything" and
            // "changed anything" are the same fact here.
            let applied = created.is_some();
            Ok(Outcome {
                created_id: created,
                applied,
            })
        }

        Command::MergeClips { clip_ids } => {
            let timeline = project.active_mut();
            if let Some(reason) = why_not_merge(timeline, &clip_ids) {
                return Err(CommandError::CannotMerge { reason });
            }
            let mut ordered: Vec<Clip> = clip_ids
                .iter()
                .filter_map(|id| timeline.clip(id).cloned())
                .collect();
            ordered.sort_by(|left, right| left.start.total_cmp(&right.start));
            let first = ordered.first().expect("validated above").clone();
            let last = ordered.last().expect("validated above");
            let merged_duration = last.start + last.duration - first.start;

            // A set, not a Vec: the retain below tests every clip on the
            // timeline against it.
            let doomed: HashSet<String> =
                ordered.iter().skip(1).map(|clip| clip.id.clone()).collect();
            timeline.clips.retain(|clip| !doomed.contains(&clip.id));
            let survivor = timeline
                .clip_mut(&first.id)
                .expect("the first piece survives the retain");
            survivor.duration = merged_duration;
            // A validated merge always absorbs at least one piece.
            Ok(Outcome {
                created_id: Some(first.id),
                applied: true,
            })
        }

        Command::RemoveClips { clip_ids } => {
            let timeline = project.active_mut();
            let doomed: HashSet<&str> = clip_ids.iter().map(String::as_str).collect();
            let clip_count = timeline.clips.len();
            timeline
                .clips
                .retain(|clip| !doomed.contains(clip.id.as_str()));
            let applied = timeline.clips.len() != clip_count;
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::UpdateClip { clip_id, patch } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let mut applied = false;
            if let Some(name) = patch.name {
                applied |= assign(&mut clip.name, name);
            }
            if let Some(volume) = patch.volume {
                applied |= assign(&mut clip.volume, volume.max(0.0));
            }
            if let Some(fade_in) = patch.fade_in {
                applied |= assign(&mut clip.fade_in, fade_in.max(0.0));
            }
            if let Some(fade_out) = patch.fade_out {
                applied |= assign(&mut clip.fade_out, fade_out.max(0.0));
            }
            if let Some(opacity) = patch.opacity {
                applied |= assign(&mut clip.opacity, opacity.clamp(0.0, 1.0));
            }
            if let Some(preserve) = patch.preserve_pitch {
                applied |= assign(&mut clip.preserve_pitch, preserve);
            }
            if let Some(filters) = patch.filters {
                applied |= assign(&mut clip.filters, filters);
            }
            if let Some(effects) = patch.video_effects {
                applied |= assign(&mut clip.video_effects, effects);
            }
            if let Some(transition) = patch.transition_in {
                applied |= assign(&mut clip.transition_in, transition);
            }
            if let Some(text) = patch.text {
                // The name follows the words, like addTextClip snapshots it.
                if let Some(style) = &text {
                    applied |= assign(&mut clip.name, first_line(&style.content));
                }
                applied |= assign(&mut clip.text, text);
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetClipSpeed { clip_id, speed } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            // The amount of source covered is held constant - that is what
            // makes this a speed change rather than a trim.
            let next = speed.clamp(MIN_SPEED, MAX_SPEED);
            let source_covered = clip.duration * clip.speed;
            // Bitwise so no assignment is short-circuited away.
            let applied = assign(&mut clip.speed, next)
                | assign(
                    &mut clip.duration,
                    (source_covered / next).max(MIN_CLIP_DURATION),
                );
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetClipTransform {
            clip_id,
            scale,
            offset_x,
            offset_y,
            rotation,
        } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip_mut(&clip_id) else {
                return Ok(Outcome::default());
            };
            let mut applied = false;
            if let Some(scale) = scale {
                applied |= assign(&mut clip.scale, scale.clamp(MIN_SCALE, MAX_SCALE));
            }
            if let Some(offset) = offset_x {
                applied |= assign(&mut clip.offset_x, offset.clamp(-MAX_OFFSET, MAX_OFFSET));
            }
            if let Some(offset) = offset_y {
                applied |= assign(&mut clip.offset_y, offset.clamp(-MAX_OFFSET, MAX_OFFSET));
            }
            if let Some(rotation) = rotation {
                // Kept in (-180, 180] so a full drag never accumulates turns.
                let wrapped = ((rotation % 360.0) + 540.0) % 360.0 - 180.0;
                let next = if wrapped == -180.0 { 180.0 } else { wrapped };
                applied |= assign(&mut clip.rotation, next);
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::DetachAudio { clip_id } => {
            let has_audio = {
                let timeline = project.active();
                let Some(clip) = timeline.clip(&clip_id) else {
                    return Ok(Outcome::default());
                };
                clip.kind == ClipKind::Video
                    && clip.muted != Some(true)
                    && project
                        .media_by_id(&clip.media_id)
                        .is_some_and(|media| media.has_audio)
                    && !timeline
                        .clips
                        .iter()
                        .any(|other| other.detached_from.as_deref() == Some(clip_id.as_str()))
            };
            if !has_audio {
                return Ok(Outcome::default());
            }

            let timeline = project.active_mut();
            let clip = timeline.clip(&clip_id).expect("checked above").clone();
            // A lane free for the whole span, or a fresh one.
            let track_id = {
                let end = clip.start + clip.duration;
                let free = timeline.tracks.iter().find(|track| {
                    !timeline.clips.iter().any(|other| {
                        other.track_id == track.id
                            && other.start < end
                            && clip.start < other.start + other.duration
                    })
                });
                match free {
                    Some(track) => track.id.clone(),
                    None => {
                        let id = mint.next("t");
                        let name = next_numbered(
                            "Track",
                            timeline.tracks.iter().map(|track| track.name.clone()),
                        );
                        timeline.tracks.push(Track {
                            id: id.clone(),
                            name,
                            visible: true,
                            muted: false,
                        });
                        id
                    }
                }
            };

            let mut sound = clip.clone();
            sound.id = mint.next("c");
            sound.track_id = track_id;
            sound.kind = ClipKind::Audio;
            sound.video_effects = Vec::new();
            sound.transition_in = None;
            sound.detached_from = Some(clip_id.clone());
            sound.muted = None;
            let sound_id = sound.id.clone();
            timeline.clips.push(sound);

            let video = timeline.clip_mut(&clip_id).expect("still present");
            video.muted = Some(true);
            video.filters = Vec::new();
            Ok(Outcome {
                created_id: Some(sound_id),
                applied: true,
            })
        }

        Command::ReattachAudio { clip_id } => {
            let timeline = project.active_mut();
            let Some(clip) = timeline.clip(&clip_id) else {
                return Ok(Outcome::default());
            };
            let video_id = match (&clip.kind, &clip.detached_from) {
                (ClipKind::Audio, Some(source)) => source.clone(),
                _ => clip.id.clone(),
            };
            if timeline.clip(&video_id).is_none() {
                return Ok(Outcome::default());
            }
            let sounds: Vec<Clip> = timeline
                .clips
                .iter()
                .filter(|other| other.detached_from.as_deref() == Some(video_id.as_str()))
                .cloned()
                .collect();
            if sounds.is_empty() {
                return Ok(Outcome::default());
            }
            // A set, not a Vec: the retain below tests every clip on the
            // timeline against it.
            let doomed: HashSet<String> = sounds.iter().map(|sound| sound.id.clone()).collect();
            timeline.clips.retain(|other| !doomed.contains(&other.id));
            let video = timeline.clip_mut(&video_id).expect("checked above");
            video.muted = None;
            video.filters = sounds[0].filters.clone();
            // Reaching here means at least one sound clip was deleted.
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        Command::AddTrack => {
            let timeline = project.active_mut();
            let id = mint.next("t");
            let name = next_numbered(
                "Track",
                timeline.tracks.iter().map(|track| track.name.clone()),
            );
            timeline.tracks.push(Track {
                id: id.clone(),
                name,
                visible: true,
                muted: false,
            });
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::RemoveTrack { track_id } => {
            let timeline = project.active_mut();
            if timeline.tracks.len() <= 1 {
                return Err(CommandError::LastTrack);
            }
            let track_count = timeline.tracks.len();
            timeline.tracks.retain(|track| track.id != track_id);
            timeline.clips.retain(|clip| clip.track_id != track_id);
            // Clips only ever sit on existing tracks, so an unknown id - the
            // tolerated no-op - removes neither.
            let applied = timeline.tracks.len() != track_count;
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::RenameTrack { track_id, name } => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Ok(Outcome::default());
            }
            let timeline = project.active_mut();
            let applied = timeline
                .tracks
                .iter_mut()
                .find(|track| track.id == track_id)
                .is_some_and(|track| assign(&mut track.name, trimmed.to_owned()));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SetTrackFlag {
            track_id,
            flag,
            value,
        } => {
            let timeline = project.active_mut();
            let applied = timeline
                .tracks
                .iter_mut()
                .find(|track| track.id == track_id)
                .is_some_and(|track| match flag {
                    TrackFlag::Visible => assign(&mut track.visible, value),
                    TrackFlag::Muted => assign(&mut track.muted, value),
                });
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::AddTimeline => {
            let id = mint.next("tl");
            let name = next_numbered(
                "Timeline",
                project
                    .timelines
                    .iter()
                    .map(|timeline| timeline.name.clone()),
            );
            let tracks = (1..=4)
                .map(|number| Track {
                    id: mint.next("t"),
                    name: format!("Track {number}"),
                    visible: true,
                    muted: false,
                })
                .collect();
            project.timelines.push(Timeline {
                id: id.clone(),
                name,
                tracks,
                clips: Vec::new(),
            });
            project.active_timeline_id = id.clone();
            Ok(Outcome {
                created_id: Some(id),
                applied: true,
            })
        }

        Command::RemoveTimeline { timeline_id } => {
            if project.timelines.len() <= 1 {
                return Err(CommandError::LastTimeline);
            }
            let Some(index) = project
                .timelines
                .iter()
                .position(|timeline| timeline.id == timeline_id)
            else {
                return Ok(Outcome::default());
            };
            if project.active_timeline_id == timeline_id {
                let neighbour = if index + 1 < project.timelines.len() {
                    index + 1
                } else {
                    index - 1
                };
                project.active_timeline_id = project.timelines[neighbour].id.clone();
            }
            project.timelines.remove(index);
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        Command::RenameTimeline { timeline_id, name } => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Ok(Outcome::default());
            }
            let applied = project
                .timelines
                .iter_mut()
                .find(|timeline| timeline.id == timeline_id)
                .is_some_and(|timeline| assign(&mut timeline.name, trimmed.to_owned()));
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::SelectTimeline { timeline_id } => {
            // Short-circuiting is right here: an unknown id must not touch
            // the selection at all.
            let applied = project
                .timelines
                .iter()
                .any(|timeline| timeline.id == timeline_id)
                && assign(&mut project.active_timeline_id, timeline_id);
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::MoveTimeline { timeline_id, index } => {
            let Some(from) = project
                .timelines
                .iter()
                .position(|timeline| timeline.id == timeline_id)
            else {
                return Ok(Outcome::default());
            };
            let to = index.min(project.timelines.len() - 1);
            if to == from {
                return Ok(Outcome::default());
            }
            let timeline = project.timelines.remove(from);
            project.timelines.insert(to, timeline);
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        Command::AddFont { family, path } => {
            if project.fonts.iter().any(|font| font.path == path) {
                return Ok(Outcome::default());
            }
            project.fonts.push(CustomFont { family, path });
            Ok(Outcome {
                created_id: None,
                applied: true,
            })
        }

        Command::RemoveFont { family } => {
            // Clips keep the family name: the face may come back when the
            // file does.
            let font_count = project.fonts.len();
            project.fonts.retain(|font| font.family != family);
            let applied = project.fonts.len() != font_count;
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }
    }
}
