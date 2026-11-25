// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The kinds of id the model has, and what holds the text of a file
//! that this build could not read.
//!
//! Every id comes from the document's one count, so the number in `c12`
//! is in no other id of any kind. The letters are the ones the published
//! API already writes (`m` for a file in the bin, `tl` for a timeline,
//! `t` for a lane, `c` for anything placed on one), so an id a script
//! holds keeps its spelling; the kinds the API has no letter for take
//! new ones.

use std::fmt;

crate::typed_id! {
    /// A file in the bin.
    pub struct MediaId = "m";
    /// A sequence: a timeline of lanes, shown as a tab or placed inside
    /// another as a clip.
    pub struct SequenceId = "tl";
    /// A lane of a sequence.
    pub struct LaneId = "t";
    /// Anything placed on a lane: a clip of a file, a nested sequence, a
    /// title, a shape, a layer, a transition.
    pub struct ItemId = "c";
    /// One effect in a chain, or the package of a transition.
    pub struct EffectId = "fx";
    /// One mask of an item.
    pub struct MaskId = "mk";
    /// One marker, on a sequence or on an item.
    pub struct MarkerId = "mr";
    /// An entry of a file that this build could not read, kept to be
    /// written back.
    pub struct HeldId = "h";
}

/// Text from a file, exactly as it was read, for a value this build does
/// not understand: an enum's name it does not know, a field it has no
/// place for, an entry it cannot read.
///
/// Nothing here parses it. The file's reader puts it here and its writer
/// puts it back, so a project from a newer build loses nothing by being
/// opened and saved by this one.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Raw(pub String);

impl Raw {
    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Raw {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Raw({})", self.0)
    }
}

impl From<&str> for Raw {
    fn from(text: &str) -> Raw {
        Raw(text.to_owned())
    }
}

/// What a record of a file held that this build does not know, kept to
/// be written back: its unknown fields, and what the file marked as
/// worked out from the rest of the record.
///
/// A record is a row of a table (a file of the bin, a sequence, a lane,
/// an item), an effect, a mask, a marker, or the document itself. The
/// parts of a record - an item's `picture`, a key of its opacity - have
/// no residual of their own: a field this build does not know inside
/// one is the record's, named by its path from the record (see
/// [`Residual::fields`]).
#[derive(Clone, PartialEq, Eq, Hash, Default, Debug)]
pub struct Residual {
    /// The fields this build does not know, each as its text, in the
    /// order the file had them, and kept however the record is edited.
    ///
    /// Each is named by its path from the record: `glow` for one of the
    /// record's own fields, `picture/glow` for one inside its picture.
    /// An element of a list is named by its tick for the keys of an
    /// animated number and of a time map (`picture/scale/keys/0/tilt`),
    /// by its index for a file's sound streams, and by its place in the
    /// list otherwise, so that an unknown field of a key stays on that
    /// key when the keys around it are edited. A `/` in a name is
    /// written `~1` and a `~` is written `~0`, so a path has one reading.
    ///
    /// A list and not a map: a file may say a name twice, and both are
    /// kept as they came.
    pub fields: Vec<(String, Raw)>,
    /// What the file marked as derived: worked out from the rest of the
    /// record by a build that knew how. Kept while this build leaves the
    /// record as it read it, and dropped once it changes the record,
    /// since this build cannot work it out again.
    pub derived: Option<Derived>,
}

impl Residual {
    /// Whether there is nothing here.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.derived.is_none()
    }

    /// About how many bytes it holds.
    pub fn weight(&self) -> usize {
        let fields: usize = self
            .fields
            .iter()
            .map(|(name, raw)| name.len() + raw.0.len() + 48)
            .sum();
        fields + self.derived.as_ref().map_or(0, |d| d.raw.0.len() + 32)
    }
}

/// What a record of a file marked as derived from the rest of it: its
/// text, and a seal of the record as it was read.
///
/// The seal is a hash of the record as this build writes it, worked out
/// when it was read and never saved. When the record is written, the
/// seal is worked out again: the same, and the record is as it was read,
/// so what was derived from it still holds and is written back; not, and
/// it is dropped. After the unsafe-to-copy chunks of PNG.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Derived {
    /// The text, as the file had it.
    pub raw: Raw,
    /// The record as it was read, in short.
    pub seal: u64,
}

/// One entry of a list in a file: one this build read, or the text of
/// one it could not, kept in its place.
#[derive(Clone, PartialEq, Debug)]
pub enum Entry<T> {
    /// An entry that was read.
    Known(T),
    /// The text of one that could not be.
    Unread(Raw),
}

impl<T> Entry<T> {
    /// The entry, if it was read.
    pub fn known(&self) -> Option<&T> {
        match self {
            Entry::Known(value) => Some(value),
            Entry::Unread(_) => None,
        }
    }

    /// The same, to change.
    pub fn known_mut(&mut self) -> Option<&mut T> {
        match self {
            Entry::Known(value) => Some(value),
            Entry::Unread(_) => None,
        }
    }
}

/// The entries of a list that were read, in order.
pub fn known<T>(entries: &[Entry<T>]) -> impl Iterator<Item = &T> {
    entries.iter().filter_map(Entry::known)
}
