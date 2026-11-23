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

/// The fields of a record that this build does not know, by name, in
/// the order the file had them, each as its text.
///
/// A list and not a map: a file may say a name twice, and both are kept
/// as they came. Whether such a field is safe to keep once the record
/// it is in has been edited is for the file's format to say.
#[derive(Clone, PartialEq, Eq, Hash, Default, Debug)]
pub struct Residual(pub Vec<(String, Raw)>);

impl Residual {
    /// Whether there is nothing here.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// About how many bytes it holds.
    pub fn weight(&self) -> usize {
        self.0
            .iter()
            .map(|(name, raw)| name.len() + raw.0.len() + 48)
            .sum()
    }
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
