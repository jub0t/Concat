// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Names for the things in a document: typed, short, and never used
//! twice.
//!
//! A thing in a document needs a name that still means the same thing
//! after a save, an undo, a paste, and an edit by somebody else. A place
//! in a list does not: it moves when a neighbour does. A slot number does
//! not: it means nothing in the next run. A long random name does, at the
//! cost of thirty-six characters in every line of the file, for a promise
//! of being unique in the world that a paste has to break anyway.
//!
//! An id here is one non-zero 64-bit number: sixteen bits for the
//! [`Writer`] that made it and forty-eight for that writer's count. The
//! count lives in the document's [`Mint`], is saved with it, and only
//! goes up, so an id that has been in a document never comes to mean a
//! second thing: undoing a delete brings the same id back, and what is
//! made after it gets a new one.
//!
//! # A type per kind
//!
//! [`typed_id!`](crate::typed_id) declares a kind of id as a type of its
//! own, with a letter:
//!
//! ```
//! concat_doc::typed_id! {
//!     /// A clip.
//!     pub struct ClipId = "c";
//!     /// A file the clips are cut from.
//!     pub struct MediaId = "m";
//! }
//!
//! let clip: ClipId = "c12".parse().unwrap();
//! assert_eq!(clip.to_string(), "c12");
//! assert!("m12".parse::<ClipId>().is_err());
//! ```
//!
//! and one kind cannot be handed over where another is wanted:
//!
//! ```compile_fail
//! concat_doc::typed_id! {
//!     pub struct ClipId = "c";
//!     pub struct MediaId = "m";
//! }
//!
//! fn remove(clip: ClipId) {}
//! let media: MediaId = "m3".parse().unwrap();
//! remove(media);
//! ```
//!
//! # In a file
//!
//! An id is a short string: the letter, then the count, as `"c12"`; for a
//! writer other than the first, the letter, the writer, a dot, the count,
//! as `"c3.12"`. A string and not a number, so that no reader can round
//! it; a letter, so that a line of a file says what it points at and a
//! mistyped one fails to read instead of pointing into the wrong table.
//! There is one spelling per id: no leading zeros, no sign, no spaces,
//! and no `"c0.12"` for what is written `"c12"`.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::Hash;
use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

/// How many bits of an id are its writer's count.
pub const COUNTER_BITS: u32 = 48;

/// The last count a writer can reach: 2^48 - 1. At a million ids a second
/// that is nearly nine years.
pub const MAX_COUNTER: u64 = (1 << COUNTER_BITS) - 1;

/// Which writer's count an id came from.
///
/// Today there is one, [`Writer::LOCAL`]: the open project itself, which
/// the window and every other caller in the process write through, so
/// they share one count and cannot collide. The sixteen bits are kept so
/// that something that counts for itself (a copy of the project edited
/// somewhere else) can later make ids that do not clash with these.
///
/// This is not who made an edit; that is an
/// [`Origin`](crate::Origin).
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Writer(pub u16);

impl Writer {
    /// The project's own count, written without a prefix.
    pub const LOCAL: Writer = Writer(0);
}

impl fmt::Display for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The number inside every id, whatever its kind.
///
/// A kind's own type wraps one of these and is what code should hold.
/// This is here for what must treat ids of all kinds alike: the
/// [`Mint`], a table's keys, an [`AnyId`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RawId(NonZeroU64);

impl RawId {
    /// The id a writer's count names. `None` unless the count is from 1
    /// to [`MAX_COUNTER`].
    pub const fn from_parts(writer: Writer, counter: u64) -> Option<RawId> {
        if counter == 0 || counter > MAX_COUNTER {
            return None;
        }
        match NonZeroU64::new(((writer.0 as u64) << COUNTER_BITS) | counter) {
            Some(bits) => Some(RawId(bits)),
            None => None,
        }
    }

    /// The id these 64 bits are. `None` when the count in them is zero,
    /// which is no id.
    pub const fn from_bits(bits: u64) -> Option<RawId> {
        RawId::from_parts(Writer((bits >> COUNTER_BITS) as u16), bits & MAX_COUNTER)
    }

    /// The id as 64 bits: the writer above the count.
    pub const fn bits(self) -> u64 {
        self.0.get()
    }

    /// Whose count it came from.
    pub const fn writer(self) -> Writer {
        Writer((self.0.get() >> COUNTER_BITS) as u16)
    }

    /// The count: from 1 to [`MAX_COUNTER`].
    pub const fn counter(self) -> u64 {
        self.0.get() & MAX_COUNTER
    }
}

impl fmt::Debug for RawId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_id("#", *self, f)
    }
}

/// A kind of id: a type made by [`typed_id!`](crate::typed_id).
///
/// Do not implement this by hand; the macro does, and checks the letter.
pub trait TypedId: Copy + Ord + Hash + fmt::Debug + fmt::Display + Send + Sync + 'static {
    /// The kind's letter: what its ids start with in a file. One to four
    /// lower-case letters.
    const TAG: &'static str;

    /// The id of this kind with this number inside.
    ///
    /// For what handles every kind alike. Code that knows its kinds has
    /// no use for it: going through here to turn a clip's id into a media
    /// file's is the mistake the types exist to stop.
    fn from_raw(raw: RawId) -> Self;

    /// The number inside.
    fn raw(self) -> RawId;

    /// Whose count it came from.
    fn writer(self) -> Writer {
        self.raw().writer()
    }

    /// The count: the number after the letter.
    fn counter(self) -> u64 {
        self.raw().counter()
    }

    /// The same id with its kind as a value instead of a type.
    fn any(self) -> AnyId {
        AnyId {
            tag: Self::TAG,
            raw: self.raw(),
        }
    }
}

/// An id of any kind, for the places that must name ids of several kinds
/// in one list: what a refused commit names, what a stale plan is
/// missing.
///
/// It prints as the id does: `c12`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnyId {
    tag: &'static str,
    raw: RawId,
}

impl AnyId {
    /// The kind's letter.
    pub const fn tag(self) -> &'static str {
        self.tag
    }

    /// The number inside.
    pub const fn raw(self) -> RawId {
        self.raw
    }

    /// The id as its own type, if it is of that kind.
    pub fn typed<I: TypedId>(self) -> Option<I> {
        (self.tag == I::TAG).then(|| I::from_raw(self.raw))
    }
}

impl fmt::Display for AnyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_id(self.tag, self.raw, f)
    }
}

impl fmt::Debug for AnyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_id(self.tag, self.raw, f)
    }
}

/// Why a string is not an id of the kind asked for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IdError {
    /// It does not start with this kind's letter: it is another kind's
    /// id, or not an id.
    WrongKind {
        /// The letter that was wanted.
        expected: &'static str,
    },
    /// What follows the letter is not a number, or a number, a dot and a
    /// number: it is empty, or has a sign, a space, or anything else that
    /// is not a digit.
    Malformed,
    /// A number written with a zero in front. Each id has one spelling.
    LeadingZero,
    /// A count of zero, which no id has; or a writer of zero written out,
    /// which is spelled without the writer.
    Zero,
    /// A count past 2^48 - 1 or a writer past 65,535.
    TooLarge,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdError::WrongKind { expected } => {
                write!(
                    f,
                    "not an id of this kind: it does not start with \"{expected}\""
                )
            }
            IdError::Malformed => f.write_str("an id is its letter and then a number"),
            IdError::LeadingZero => f.write_str("an id's number has no zero in front"),
            IdError::Zero => f.write_str("an id's number is never zero"),
            IdError::TooLarge => f.write_str("the id's number is too large"),
        }
    }
}

impl std::error::Error for IdError {}

/// Whether a kind's letter is one to four lower-case letters. Checked
/// when the crate that declares the kind is built.
#[doc(hidden)]
pub const fn tag_is_valid(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    if bytes.is_empty() || bytes.len() > 4 {
        return false;
    }
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_lowercase() {
            return false;
        }
        at += 1;
    }
    true
}

/// Writes an id as a file spells it.
#[doc(hidden)]
pub fn write_id(tag: &str, raw: RawId, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if raw.writer() == Writer::LOCAL {
        write!(f, "{tag}{}", raw.counter())
    } else {
        write!(f, "{tag}{}.{}", raw.writer(), raw.counter())
    }
}

/// Reads an id as a file spells it, for the kind with this letter.
#[doc(hidden)]
pub fn parse_id(tag: &'static str, text: &str) -> Result<RawId, IdError> {
    // The letters at the front are the kind, all of them: "cx1" is not a
    // "c" followed by something odd, it is some other kind's id.
    let letters = text.bytes().take_while(u8::is_ascii_lowercase).count();
    let (kind, number) = text.split_at(letters);
    if kind != tag {
        return Err(IdError::WrongKind { expected: tag });
    }
    let (writer, counter) = match number.split_once('.') {
        None => (Writer::LOCAL, parse_number(number, MAX_COUNTER)?),
        Some((writer, counter)) => {
            let writer = parse_number(writer, u64::from(u16::MAX))?;
            // In range by the limit just given.
            (Writer(writer as u16), parse_number(counter, MAX_COUNTER)?)
        }
    };
    RawId::from_parts(writer, counter).ok_or(IdError::Zero)
}

/// A whole number from 1 to `max`, written in digits and nothing else.
fn parse_number(digits: &str, max: u64) -> Result<u64, IdError> {
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(IdError::Malformed);
    }
    if digits == "0" {
        return Err(IdError::Zero);
    }
    if digits.starts_with('0') {
        return Err(IdError::LeadingZero);
    }
    let mut number: u64 = 0;
    for digit in digits.bytes() {
        number = number
            .checked_mul(10)
            .and_then(|number| number.checked_add(u64::from(digit - b'0')))
            .filter(|number| *number <= max)
            .ok_or(IdError::TooLarge)?;
    }
    Ok(number)
}

/// Reads an id of one kind from a string in a file.
#[doc(hidden)]
pub fn deserialize_id<'de, I, D>(deserializer: D) -> Result<I, D::Error>
where
    I: TypedId,
    D: serde::Deserializer<'de>,
{
    struct Text<I>(std::marker::PhantomData<I>);

    impl<I: TypedId> serde::de::Visitor<'_> for Text<I> {
        type Value = I;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "an id such as \"{}12\"", I::TAG)
        }

        fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<I, E> {
            parse_id(I::TAG, text).map(I::from_raw).map_err(E::custom)
        }
    }

    deserializer.deserialize_str(Text(std::marker::PhantomData))
}

/// Declares kinds of id, each a type of its own.
///
/// Each line is a visibility, a name and the kind's letter (one to four
/// lower-case letters, checked when the crate is built). The type is
/// `Copy`, ordered, hashable and eight bytes; it prints, parses and is
/// saved as the short string described in [the module](crate::id), and
/// implements [`TypedId`](crate::TypedId).
///
/// ```
/// concat_doc::typed_id! {
///     /// A shelf.
///     pub struct ShelfId = "s";
/// }
/// ```
#[macro_export]
macro_rules! typed_id {
    ($( $(#[$meta:meta])* $vis:vis struct $name:ident = $tag:literal; )+) => {$(
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis struct $name($crate::RawId);

        const _: () = assert!(
            $crate::id::tag_is_valid($tag),
            "an id's letter is one to four lower-case letters"
        );

        impl $crate::TypedId for $name {
            const TAG: &'static str = $tag;

            fn from_raw(raw: $crate::RawId) -> Self {
                Self(raw)
            }

            fn raw(self) -> $crate::RawId {
                self.0
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                $crate::id::write_id($tag, self.0, f)
            }
        }

        impl ::core::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                $crate::id::write_id($tag, self.0, f)
            }
        }

        impl ::core::str::FromStr for $name {
            type Err = $crate::IdError;

            fn from_str(text: &str) -> ::core::result::Result<Self, $crate::IdError> {
                $crate::id::parse_id($tag, text).map(Self)
            }
        }

        impl ::core::convert::From<$name> for $crate::AnyId {
            fn from(id: $name) -> $crate::AnyId {
                $crate::TypedId::any(id)
            }
        }

        impl $crate::__private::serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error>
            where
                S: $crate::__private::serde::Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> $crate::__private::serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> ::core::result::Result<Self, D::Error>
            where
                D: $crate::__private::serde::Deserializer<'de>,
            {
                $crate::id::deserialize_id(deserializer)
            }
        }
    )+};
}

/// Why an id could not be made.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MintError {
    /// The writer has made all 2^48 - 1 of its ids. The count does not
    /// start again: an id used twice is worse than none.
    Exhausted {
        /// Whose count ran out.
        writer: Writer,
    },
    /// A saved count is past the last an id can hold.
    CountTooLarge {
        /// Whose count it is.
        writer: Writer,
    },
}

impl fmt::Display for MintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MintError::Exhausted { writer } => {
                write!(f, "writer {writer} has no ids left to give")
            }
            MintError::CountTooLarge { writer } => {
                write!(
                    f,
                    "writer {writer}'s saved count is past the last id there is"
                )
            }
        }
    }
}

impl std::error::Error for MintError {}

/// The counts that ids are made from: for each writer, the last count it
/// has given out.
///
/// It is part of the document and is saved with it. A count only goes up,
/// and only inside a transaction: there is no way to ask a mint for an id
/// from outside one ([`Draft::mint`](crate::Draft::mint) is the way). One
/// count serves every kind, so the number in `c12` is not also in an
/// `m12`, and the mint needs to know nothing about the kinds there are.
///
/// Undo does not turn a count back. A step back restores what the tables
/// held and keeps the counts where they are, so an id made by a step that
/// was undone is not given out again; the same id comes back only if that
/// step is redone.
///
/// In a file it is the writers and their counts: `{"0": 12}`.
#[derive(Clone, PartialEq, Eq, Default, Debug, Serialize, Deserialize)]
#[serde(try_from = "BTreeMap<Writer, u64>", into = "BTreeMap<Writer, u64>")]
pub struct Mint {
    /// The last count each writer has given out. A writer that has given
    /// none is not here.
    issued: BTreeMap<Writer, u64>,
}

impl Mint {
    /// A mint that has made nothing.
    pub fn new() -> Mint {
        Mint::default()
    }

    /// A mint that goes on from saved counts: for each writer, the last
    /// count it had given out. Refused when a count is past
    /// [`MAX_COUNTER`].
    pub fn resume(counts: impl IntoIterator<Item = (Writer, u64)>) -> Result<Mint, MintError> {
        let mut issued = BTreeMap::new();
        for (writer, count) in counts {
            if count > MAX_COUNTER {
                return Err(MintError::CountTooLarge { writer });
            }
            if count > 0 {
                let known = issued.entry(writer).or_insert(0);
                *known = count.max(*known);
            }
        }
        Ok(Mint { issued })
    }

    /// The last count a writer has given out; zero if it has given none.
    pub fn issued(&self, writer: Writer) -> u64 {
        self.issued.get(&writer).copied().unwrap_or(0)
    }

    /// Every writer that has given out an id, with its last count.
    pub fn counts(&self) -> impl Iterator<Item = (Writer, u64)> + '_ {
        self.issued.iter().map(|(writer, count)| (*writer, *count))
    }

    /// Whether this mint has given out the id, or one after it from the
    /// same writer: whether the id is one this mint will never make
    /// again.
    pub fn covers(&self, id: RawId) -> bool {
        id.counter() <= self.issued(id.writer())
    }

    /// The next id of a writer. Only a transaction may ask.
    pub(crate) fn mint<I: TypedId>(&mut self, writer: Writer) -> Result<I, MintError> {
        let next = self.issued(writer) + 1;
        let raw = RawId::from_parts(writer, next).ok_or(MintError::Exhausted { writer })?;
        self.issued.insert(writer, next);
        Ok(I::from_raw(raw))
    }
}

impl TryFrom<BTreeMap<Writer, u64>> for Mint {
    type Error = MintError;

    fn try_from(counts: BTreeMap<Writer, u64>) -> Result<Mint, MintError> {
        Mint::resume(counts)
    }
}

impl From<Mint> for BTreeMap<Writer, u64> {
    fn from(mint: Mint) -> BTreeMap<Writer, u64> {
        mint.issued
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use super::*;

    crate::typed_id! {
        /// A clip.
        struct ClipId = "c";
        /// A mask, with a letter of two.
        struct MaskId = "mk";
    }

    fn clip(writer: u16, counter: u64) -> ClipId {
        ClipId::from_raw(RawId::from_parts(Writer(writer), counter).unwrap())
    }

    #[test]
    fn an_id_is_its_letter_and_its_count_in_a_file() {
        assert_eq!(clip(0, 12).to_string(), "c12");
        assert_eq!(clip(3, 12).to_string(), "c3.12");
        assert_eq!(
            clip(u16::MAX, MAX_COUNTER).to_string(),
            "c65535.281474976710655"
        );
        assert_eq!(format!("{:?}", clip(0, 12)), "c12");
        assert_eq!(serde_json::to_string(&clip(0, 12)).unwrap(), "\"c12\"");
        assert_eq!(serde_json::to_string(&clip(3, 12)).unwrap(), "\"c3.12\"");
        assert_eq!(clip(3, 12).any().to_string(), "c3.12");
    }

    #[test]
    fn an_id_reads_back_as_the_id_it_was() {
        for id in [
            clip(0, 1),
            clip(0, 12),
            clip(0, MAX_COUNTER),
            clip(1, 1),
            clip(65_535, 9),
        ] {
            assert_eq!(id.to_string().parse::<ClipId>(), Ok(id));
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(serde_json::from_str::<ClipId>(&json).unwrap(), id);
            assert_eq!(RawId::from_bits(id.raw().bits()), Some(id.raw()));
        }
        let mask: MaskId = "mk7".parse().unwrap();
        assert_eq!((mask.writer(), mask.counter()), (Writer::LOCAL, 7));
        assert_eq!(mask.to_string(), "mk7");
    }

    #[test]
    fn the_writer_is_the_top_sixteen_bits_and_the_count_the_rest() {
        let id = clip(3, 12);
        assert_eq!(id.raw().bits(), (3 << 48) | 12);
        assert_eq!((id.writer(), id.counter()), (Writer(3), 12));
        assert_eq!(std::mem::size_of::<ClipId>(), 8);
        assert_eq!(std::mem::size_of::<Option<ClipId>>(), 8);
        // In order by writer, then by count.
        assert!(clip(0, 2) < clip(0, 10));
        assert!(clip(0, MAX_COUNTER) < clip(1, 1));
    }

    #[test]
    fn another_kinds_letter_is_refused() {
        let wrong = Err(IdError::WrongKind { expected: "c" });
        assert_eq!("m12".parse::<ClipId>(), wrong);
        assert_eq!("mk12".parse::<ClipId>(), wrong);
        assert_eq!("cx12".parse::<ClipId>(), wrong);
        assert_eq!("12".parse::<ClipId>(), wrong);
        assert_eq!("".parse::<ClipId>(), wrong);
        assert_eq!("C12".parse::<ClipId>(), wrong);
        assert_eq!(
            "c12".parse::<MaskId>(),
            Err(IdError::WrongKind { expected: "mk" })
        );
        assert_eq!(
            "m12".parse::<MaskId>(),
            Err(IdError::WrongKind { expected: "mk" })
        );
    }

    #[test]
    fn zero_is_refused() {
        assert_eq!("c0".parse::<ClipId>(), Err(IdError::Zero));
        assert_eq!("c3.0".parse::<ClipId>(), Err(IdError::Zero));
        // Writer zero is spelled without the writer, and only so.
        assert_eq!("c0.12".parse::<ClipId>(), Err(IdError::Zero));
        assert_eq!(RawId::from_bits(0), None);
        assert_eq!(RawId::from_bits(3 << 48), None);
        assert_eq!(RawId::from_parts(Writer(3), 0), None);
    }

    #[test]
    fn a_number_too_large_is_refused() {
        assert_eq!(
            "c281474976710655".parse::<ClipId>(),
            Ok(clip(0, MAX_COUNTER))
        );
        assert_eq!("c281474976710656".parse::<ClipId>(), Err(IdError::TooLarge));
        assert_eq!(
            "c18446744073709551616".parse::<ClipId>(),
            Err(IdError::TooLarge)
        );
        assert_eq!(
            "c99999999999999999999999999999".parse::<ClipId>(),
            Err(IdError::TooLarge)
        );
        assert_eq!("c65535.1".parse::<ClipId>(), Ok(clip(65_535, 1)));
        assert_eq!("c65536.1".parse::<ClipId>(), Err(IdError::TooLarge));
        assert_eq!(
            "c1.281474976710656".parse::<ClipId>(),
            Err(IdError::TooLarge)
        );
        assert_eq!(RawId::from_parts(Writer(0), MAX_COUNTER + 1), None);
    }

    #[test]
    fn a_zero_in_front_is_refused() {
        assert_eq!("c012".parse::<ClipId>(), Err(IdError::LeadingZero));
        assert_eq!("c00".parse::<ClipId>(), Err(IdError::LeadingZero));
        assert_eq!("c03.12".parse::<ClipId>(), Err(IdError::LeadingZero));
        assert_eq!("c3.012".parse::<ClipId>(), Err(IdError::LeadingZero));
    }

    #[test]
    fn signs_spaces_and_anything_that_is_not_a_digit_are_refused() {
        for text in [
            "c", "c+12", "c-12", "c 12", " c12", "c12 ", "c12\n", "c1 2", "c1_2", "c1.2.3", "c.12",
            "c12.", "c.", "c1e3", "c0x10", "c١٢", "c12c", "c1,2", "c3.+2", "c-3.2",
        ] {
            let read = text.parse::<ClipId>();
            assert!(
                matches!(read, Err(IdError::Malformed | IdError::WrongKind { .. })),
                "{text:?} read as {read:?}"
            );
        }
        assert_eq!("c+12".parse::<ClipId>(), Err(IdError::Malformed));
        assert_eq!("c 12".parse::<ClipId>(), Err(IdError::Malformed));
        assert_eq!("c12 ".parse::<ClipId>(), Err(IdError::Malformed));
        assert_eq!("c".parse::<ClipId>(), Err(IdError::Malformed));
    }

    #[test]
    fn a_file_must_spell_an_id_as_a_string_of_its_own_kind() {
        assert!(serde_json::from_str::<ClipId>("12").is_err());
        assert!(serde_json::from_str::<ClipId>("\"m12\"").is_err());
        assert!(serde_json::from_str::<ClipId>("\"c012\"").is_err());
        assert!(serde_json::from_str::<ClipId>("null").is_err());
        let message = serde_json::from_str::<ClipId>("\"c0\"")
            .unwrap_err()
            .to_string();
        assert!(message.contains("never zero"), "{message}");
    }

    #[test]
    fn ids_are_keys_in_a_file_and_in_a_map() {
        let mut names = BTreeMap::new();
        names.insert(clip(0, 2), "two");
        names.insert(clip(1, 1), "other");
        let json = serde_json::to_string(&names).unwrap();
        assert_eq!(json, r#"{"c2":"two","c1.1":"other"}"#);
        let back: BTreeMap<ClipId, &str> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, names);
        let hashed: HashSet<ClipId> = [clip(0, 2), clip(0, 2), clip(0, 3)].into();
        assert_eq!(hashed.len(), 2);
    }

    #[test]
    fn an_id_of_any_kind_knows_its_kind() {
        let any: AnyId = clip(0, 5).into();
        assert_eq!(any.tag(), "c");
        assert_eq!(any.typed::<ClipId>(), Some(clip(0, 5)));
        assert_eq!(any.typed::<MaskId>(), None);
        let mask = MaskId::from_raw(clip(0, 5).raw());
        assert_ne!(any, mask.any());
        assert_eq!(format!("{any:?} {:?}", mask.any()), "c5 mk5");
    }

    #[test]
    fn a_letter_is_one_to_four_lower_case_letters() {
        assert!(tag_is_valid("c"));
        assert!(tag_is_valid("mask"));
        assert!(!tag_is_valid(""));
        assert!(!tag_is_valid("masks"));
        assert!(!tag_is_valid("C"));
        assert!(!tag_is_valid("c1"));
        assert!(!tag_is_valid("c."));
    }

    #[test]
    fn a_mint_counts_up_per_writer_and_never_gives_an_id_twice() {
        let mut mint = Mint::new();
        let mut seen = BTreeSet::new();
        for _ in 0..100 {
            assert!(seen.insert(mint.mint::<ClipId>(Writer::LOCAL).unwrap().raw()));
            assert!(seen.insert(mint.mint::<MaskId>(Writer::LOCAL).unwrap().raw()));
            assert!(seen.insert(mint.mint::<ClipId>(Writer(7)).unwrap().raw()));
        }
        assert_eq!(mint.issued(Writer::LOCAL), 200);
        assert_eq!(mint.issued(Writer(7)), 100);
        assert_eq!(mint.issued(Writer(8)), 0);
        assert_eq!(
            mint.counts().collect::<Vec<_>>(),
            [(Writer(0), 200), (Writer(7), 100)]
        );
        assert!(mint.covers(clip(0, 200).raw()));
        assert!(!mint.covers(clip(0, 201).raw()));
        assert!(!mint.covers(clip(8, 1).raw()));
    }

    #[test]
    fn minting_past_the_last_count_is_an_error_and_not_a_wrap() {
        let mut mint = Mint::resume([(Writer(2), MAX_COUNTER - 1)]).unwrap();
        assert_eq!(mint.mint::<ClipId>(Writer(2)), Ok(clip(2, MAX_COUNTER)));
        let full = Err(MintError::Exhausted { writer: Writer(2) });
        assert_eq!(mint.mint::<ClipId>(Writer(2)), full);
        assert_eq!(mint.mint::<ClipId>(Writer(2)), full);
        assert_eq!(mint.issued(Writer(2)), MAX_COUNTER);
        // Another writer's count is its own.
        assert_eq!(mint.mint::<ClipId>(Writer(3)), Ok(clip(3, 1)));
    }

    #[test]
    fn a_mint_is_saved_as_its_counts_and_refuses_one_no_id_could_have() {
        let mut mint = Mint::new();
        for _ in 0..12 {
            mint.mint::<ClipId>(Writer::LOCAL).unwrap();
        }
        mint.mint::<ClipId>(Writer(3)).unwrap();
        let json = serde_json::to_string(&mint).unwrap();
        assert_eq!(json, r#"{"0":12,"3":1}"#);
        assert_eq!(serde_json::from_str::<Mint>(&json).unwrap(), mint);
        assert_eq!(serde_json::from_str::<Mint>("{}").unwrap(), Mint::new());
        assert_eq!(
            serde_json::from_str::<Mint>(r#"{"5":0}"#).unwrap(),
            Mint::new()
        );
        assert!(serde_json::from_str::<Mint>(r#"{"0":281474976710656}"#).is_err());
        assert!(serde_json::from_str::<Mint>(r#"{"65536":1}"#).is_err());
        assert!(serde_json::from_str::<Mint>(r#"{"0":-1}"#).is_err());
        assert_eq!(
            Mint::resume([(Writer(1), MAX_COUNTER + 1)]),
            Err(MintError::CountTooLarge { writer: Writer(1) })
        );
        // Told a writer's count twice, the mint goes on from the later.
        let twice = Mint::resume([(Writer(1), 9), (Writer(1), 4)]).unwrap();
        assert_eq!(twice.issued(Writer(1)), 9);
    }
}
