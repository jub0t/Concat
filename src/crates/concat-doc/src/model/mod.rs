// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The model: what a project is made of, and the rules it keeps.
//!
//! A [`Model`] is the [`Schema`] of a Concat document: flat tables keyed
//! by id, a few plain values, and the rules every [`Document`] of it
//! keeps. A [`Draft`] of it may break them for as long as it is open;
//! its commit is refused with every [`Violation`], each naming the id at
//! fault and the [`Rule`], and the document is as it was.
//!
//! [`Document`]: crate::Document
//! [`Draft`]: crate::Draft
//!
//! # What a document holds
//!
//! | Table | Of | Each says |
//! |---|---|---|
//! | `media` | [`Media`] | a file in the bin and what its probe found |
//! | `sequences` | [`Sequence`] | a timeline: frame, rate, background, markers |
//! | `lanes` | [`Lane`] | its sequence, its place in the stack, its toggles |
//! | `items` | [`Item`] | its lane, start, length, window, content, look, sound, chains, masks, markers, tie |
//! | `held` | [`Held`] | an entry of a file that could not be taken in, as its text |
//!
//! and three plain values: the `fonts` the person added, the document's
//! own unknown fields (`residual`), and the [`View`]: which sequence is
//! showing and the order of the tabs, which is saved and is no step of
//! undo.
//!
//! An item's place is absolute: its lane and its start tick. Nothing is
//! a list index, so a move changes one item, and two writers naming ids
//! cannot point at the wrong thing. What is placed where is answered by
//! an index each snapshot keeps of its items, derived from them and never
//! saved: see [`ItemIndex`].
//!
//! Every sequence is an entry of its table; an item nests one by naming
//! it ([`Content::Nest`]). Effects, masks and markers belong to what
//! holds them and have ids of their own, from the same count as every
//! other id. Every enum a later build could add to has an `Unknown` arm
//! that keeps the file's text, and every record a [`Residual`] for
//! fields this build does not know; a list entry that could not be read
//! at all is an [`Entry::Unread`] in its place.
//!
//! # The tables are declared once
//!
//! The struct, what changed between two of it, the checks and the list
//! of tables every caller can ask about are all made from one list in
//! this module, so no table can be left out of one of them.
//!
//! # The rules
//!
//! Where a type can hold a rule it does: ids are of their kind, lengths
//! are more than nothing ([`TickLen`](crate::TickLen)), a rate or a speed has no zero, a
//! picture's size has no zero side, an order key is a key. The rest are
//! checked at commit, on what the change list names: a commit that moved
//! one item checks that item, its lane's neighbours around it, what is
//! tied to it, and the transitions above where it was. Each rule below
//! is refused at commit by the test named after it, in
//! `tests/model_rules.rs`.
//!
//! **Time**
//!
//! 1. Every tick and length the document holds, and every end worked out
//!    from them - an item's end, its window's end, the source its window
//!    shows - is within [`TICK_LIMIT`] of zero. *a_tick_past_the_range_is_refused*
//!
//! **What names what** (each reference resolves to a thing of its kind)
//!
//! 2. A lane's sequence exists. *a_lane_of_no_sequence_is_refused*
//! 3. An item's lane exists. *an_item_on_no_lane_is_refused*
//! 4. The file a cut is of exists. *a_cut_of_no_file_is_refused*
//! 5. The sequence an item nests exists. *a_nest_of_no_sequence_is_refused*
//! 6. An enhanced copy exists. *an_enhanced_copy_that_is_not_there_is_refused*
//! 7. A background still, of a sequence or of an item, exists and is not
//!    a video or a recording. *a_background_that_is_not_a_still_is_refused*
//! 8. What an item is tied to exists. *a_link_to_nothing_is_refused*
//! 9. The mask an effect works inside is one of the same item's.
//!    *an_effect_on_a_mask_of_another_item_is_refused*
//! 10. The sequence showing, and every tab, exist. *a_view_of_no_sequence_is_refused*
//! 11. A file is not taken out of the bin while an item or a sequence
//!     names it. *removing_a_file_in_use_is_refused*
//! 12. A sequence is not taken out while a lane is in it or an item
//!     nests it. *removing_a_sequence_in_use_is_refused*
//! 13. A lane is not taken out while an item is on it.
//!     *removing_a_lane_with_items_is_refused*
//! 14. An item is not taken out while another is tied to it.
//!     *removing_what_a_sound_is_tied_to_is_refused*
//!
//! **Files**
//!
//! 15. A file lists each sound stream index once. *a_stream_listed_twice_is_refused*
//! 16. A file without sound lists no sound streams.
//!     *streams_on_a_file_without_sound_are_refused*
//!
//! **Sequences and lanes**
//!
//! 17. A sequence's name is not blank. *a_blank_sequence_name_is_refused*
//! 18. No sequence holds itself, at any depth, through what it nests.
//!     *a_sequence_inside_itself_is_refused*
//! 19. A lane stays in the sequence it was made in.
//!     *moving_a_lane_to_another_sequence_is_refused*
//! 20. No two lanes of a sequence are at one place in its stack.
//!     *two_lanes_at_one_place_are_refused*
//!
//! **Items**
//!
//! 21. No two items overlap on a lane; touching is not overlapping. This
//!     holds for every kind, transitions and layers included.
//!     *overlapping_items_on_a_lane_are_refused*
//! 22. A cut takes from its file only what the file has: not the sound
//!     alone of a still. *the_sound_of_a_still_is_refused*
//! 23. A cut plays a sound stream its file lists, when the file lists
//!     any. *a_stream_the_file_does_not_list_is_refused*
//! 24. An enhanced copy is not the file it was cut from.
//!     *an_enhanced_copy_that_is_the_original_is_refused*
//! 25. A speed, and the pace between any two position keys and at either
//!     end of a smooth stretch, is from a sixteenth to sixteen times,
//!     either way. *a_speed_past_sixteen_is_refused*
//! 26. A keyed time map has at least two keys. *a_time_map_of_one_key_is_refused*
//! 27. Its keys are in order of content time, no two at one tick.
//!     *time_keys_out_of_order_are_refused*
//! 28. Its source runs one way all along: no stretch stands still or
//!     turns back, and no smooth stretch's speeds ease it past a key
//!     (Fritsch and Carlson's condition). *a_time_map_that_turns_back_is_refused*
//! 29. The window an item shows lies inside its keyed map's keys.
//!     *a_window_past_the_time_keys_is_refused*
//! 30. The window shows only source the file has: nothing before its
//!     start or past its length, and while an enhanced copy shows,
//!     nothing outside the copy. A still and a nested sequence have no
//!     length to pass. *a_window_past_the_file_is_refused*
//! 31. Every animated number's keys are in order of their ticks, no two
//!     at one. *keys_out_of_order_are_refused*
//! 32. Every easing is four numbers whose times are from 0 to 1.
//!     *an_ease_that_runs_back_in_time_is_refused*
//! 33. Every number is a finite number. *a_number_that_is_not_a_number_is_refused*
//! 34. Every number is inside what its field can mean, keys included:
//!     opacities, feathers and roundness from 0 to 1; a scale and a
//!     stretch above zero; a level and a gain not below zero; a crop
//!     leaving something of the picture; a weight from 100 to 900; a
//!     polygon of 3 to 12 sides and a star of 3 to 24 points; and the
//!     rest listed with the test. *numbers_outside_their_field_are_refused*
//! 35. A length kept as a distance - a fade, a marker's length, where an
//!     enhanced copy starts - is not below zero. *a_negative_length_is_refused*
//! 36. An effect names its package. *an_effect_of_no_package_is_refused*
//! 37. An effect's knob has a name and a value or keys.
//!     *an_empty_knob_is_refused*
//! 38. A transition placed as an item is over something: an item that is
//!     not a transition, on a lower lane of its sequence, for some of its
//!     span. Moving or taking away what was beneath it is checked too.
//!     *a_transition_over_nothing_is_refused*
//! 39. A title's runs are each non-empty, inside the words, and in order
//!     without overlapping. *a_bad_run_of_words_is_refused*
//! 40. No item is tied to itself. *a_link_to_itself_is_refused*
//! 41. A tie is between items of one sequence. *a_link_across_sequences_is_refused*
//! 42. A sound detached from an item is the sound alone of the file that
//!     item plays whole. *a_sound_tied_to_another_file_is_refused*
//! 43. A sound is not detached from a detached sound.
//!     *a_sound_tied_to_a_sound_is_refused*
//! 44. Every effect, mask and marker id is held once in the document.
//!     *an_effect_id_used_twice_is_refused*
//! 45. Every effect, mask and marker id is one the document's mint made.
//!     *a_nested_id_the_mint_never_made_is_refused*
//!
//! **View and fonts**
//!
//! 46. A sequence is open in one tab at most. *a_sequence_in_two_tabs_is_refused*
//! 47. A document with a sequence has one showing.
//!     *a_document_with_sequences_shows_one*
//! 48. No two fonts are one file. *a_font_added_twice_is_refused*
//!
//! The empty document keeps every rule (*an_empty_document_is_valid*).
//! A random run of edits is checked two ways, by what changed and by
//! reading the whole result as a file is read, and the two always agree
//! (*checking_what_changed_finds_what_checking_everything_finds*); the
//! index always equals one built afresh from the rows
//! (*the_index_is_its_rows_after_any_run_of_edits*).
//!
//! What is not a rule here, and why:
//!
//! - **At least one sequence, one lane, a floor on a clip's length.**
//!   The commands keep these (a timeline cannot be removed at the floor
//!   of one); the empty document must be valid, and a length of one
//!   tick is a length.
//! - **A file's path is used once.** Adding a path that is there is a
//!   command's no-op; a template bundle's slots all have the empty path.
//! - **What a kind of item reads.** A title keeps a level it never plays,
//!   as the published API does.
//! - **A transition on the cut into an item against its neighbours.**
//!   Kept as the API has it: a package and a length.

mod check;
mod index;
mod order;
mod timing;

pub mod animated;
pub mod effect;
pub mod ids;
pub mod item;
pub mod look;
pub mod media;
pub mod sequence;

use crate::document::{ChangePart, ChangeSet, Changed, Schema};
use crate::id::{AnyId, RawId, TypedId};
use crate::table::{AnyTable, Table, TableChanges};

pub use animated::{Animated, Ease, Key};
pub use check::{Rule, Violation};
pub use effect::{Effect, Mask, MaskMode, MaskShape, MaskSource, Param};
pub use ids::{
    EffectId, Entry, HeldId, ItemId, LaneId, MarkerId, MaskId, MediaId, Raw, Residual, SequenceId,
};
pub use index::{Derive, Indexed, ItemIndex, Items, LaneIndex, Lanes, SequenceIndex, Sequences};
pub use item::{
    Blend, BrushTool, Content, Crop, CutIn, Cutout, CutoutMode, Enhanced, Item, Link, LinkKind,
    Loop, MediaCut, Motion, Move, Nest, Picture, Sound, Stabilize, Stroke, Subject, Take,
    VoiceKind, VoiceMix,
};
pub use look::{
    Gradient, LineHead, ShapeKind, ShapeShadow, ShapeStyle, StrokeAlign, StrokeDash, TextAlign,
    TextCase, TextRun, Title,
};
pub use media::{
    BeatMarks, ColorRange, ColorSpace, Media, MediaKind, MediaOrigin, Size, SoundStream,
};
pub use order::{NotAnOrderKey, OrderKey};
pub use sequence::{Background, Color, Font, Frame, Held, Lane, Marker, Sequence, View};
pub use timing::{
    FASTEST, SLOWEST, Speed, TICK_LIMIT, TimeCurve, TimeMap, TimePoint, dur_in_range, len_in_range,
    tick_in_range,
};

/// A table of rows, as the comparison reads it: a plain one, or one that
/// keeps an index.
pub trait Rows<I, T> {
    /// The rows.
    fn rows(&self) -> &Table<I, T>;
}

impl<I, T> Rows<I, T> for Table<I, T> {
    fn rows(&self) -> &Table<I, T> {
        self
    }
}

impl<I: TypedId, T, X: Derive<I, T>> Rows<I, T> for Indexed<I, T, X> {
    fn rows(&self) -> &Table<I, T> {
        Indexed::rows(self)
    }
}

/// The ids a change made inside items and sequences, which no table
/// holds: shown to the commit so that the mint must have made them.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct NestedAdded(pub Vec<AnyId>);

impl ChangePart for NestedAdded {
    /// Always: what holds the ids changed, and says so itself.
    fn is_unchanged(&self) -> bool {
        true
    }

    fn added(&self, each: &mut dyn FnMut(AnyId)) {
        for id in &self.0 {
            each(*id);
        }
    }

    fn cost(&self) -> usize {
        0
    }
}

/// Declares the model's tables and values once, and from that one list
/// makes the struct, its changes, their comparison, the checks and the
/// list of tables.
macro_rules! model {
    (
        tables {
            $( $(#[$table_doc:meta])* $table:ident: $table_ty:ty => ($id:ty, $row:ty), $table_check:path; )*
        }
        values {
            $( $(#[$value_doc:meta])* $value:ident: $value_ty:ty, $value_check:path; )*
        }
    ) => {
        /// A Concat document's tables and plain values. See [the module](self).
        #[derive(Clone, Default, PartialEq, Debug)]
        pub struct Model {
            $( $(#[$table_doc])* pub $table: $table_ty, )*
            $( $(#[$value_doc])* pub $value: $value_ty, )*
        }

        /// What changed between two models: each table's changes, each
        /// value's, and the ids made inside items and sequences.
        #[derive(Clone, PartialEq, Debug)]
        pub struct ModelChanges {
            $(
                #[doc = concat!("What changed in `", stringify!($table), "`.")]
                pub $table: TableChanges<$id, $row>,
            )*
            $(
                #[doc = concat!("What `", stringify!($value), "` was and is, if it changed.")]
                pub $value: Option<Changed<$value_ty>>,
            )*
            /// The effect, mask and marker ids that came in.
            pub nested: NestedAdded,
        }

        impl ChangeSet for ModelChanges {
            fn parts(&self, each: &mut dyn FnMut(&dyn ChangePart)) {
                $( each(&self.$table); )*
                $( each(&self.$value); )*
                each(&self.nested);
            }
        }

        impl Model {
            /// The names of the tables, in the order they are declared,
            /// compared and checked.
            pub const TABLES: &'static [&'static str] = &[$( stringify!($table) ),*];

            /// The names of the plain values.
            pub const VALUES: &'static [&'static str] = &[$( stringify!($value) ),*];

            fn compare(before: &Model, after: &Model) -> ModelChanges {
                let mut changes = ModelChanges {
                    $( $table: TableChanges::between(
                        Rows::rows(&before.$table),
                        Rows::rows(&after.$table),
                    ), )*
                    $( $value: Changed::between(&before.$value, &after.$value), )*
                    nested: NestedAdded::default(),
                };
                changes.nested = NestedAdded(check::nested_added(&changes));
                changes
            }

            fn check_all(cx: &mut check::Check<'_>) {
                $( $table_check(cx); )*
                $( $value_check(cx); )*
            }

            fn each_table(&self, each: &mut dyn FnMut(&dyn AnyTable)) {
                $( each(&self.$table); )*
            }
        }
    };
}

model! {
    tables {
        /// The bin.
        media: Table<MediaId, Media> => (MediaId, Media), check::media;
        /// Every sequence, shown as a tab or nested.
        sequences: Sequences => (SequenceId, Sequence), check::sequences;
        /// Every lane of every sequence.
        lanes: Lanes => (LaneId, Lane), check::lanes;
        /// Everything placed on a lane. Checked after the tables above,
        /// which name the items their changes put in question.
        items: Items => (ItemId, Item), check::items;
        /// Entries of a file that could not be taken in.
        held: Table<HeldId, Held> => (HeldId, Held), check::held;
    }
    values {
        /// The fonts the person added from disk.
        fonts: Vec<Font>, check::fonts;
        /// Which sequence is showing, and the order of the tabs: view
        /// state.
        view: View, check::view;
        /// The document's own fields that this build does not know.
        residual: Residual, check::residual;
    }
}

/// A table the document does not have, of the ids held inside its items
/// and sequences, so that a plan naming an effect, a mask or a marker
/// can be told whether it is still there.
struct Nested<'a> {
    tag: &'static str,
    model: &'a Model,
}

impl AnyTable for Nested<'_> {
    fn tag(&self) -> &'static str {
        self.tag
    }

    fn holds(&self, raw: RawId) -> bool {
        let id = match self.tag {
            tag if tag == EffectId::TAG => EffectId::from_raw(raw).any(),
            tag if tag == MaskId::TAG => MaskId::from_raw(raw).any(),
            _ => MarkerId::from_raw(raw).any(),
        };
        self.model.items.index().holding(id).next().is_some()
            || self.model.sequences.index().holding(id).next().is_some()
    }

    fn count(&self) -> usize {
        0
    }
}

impl Schema for Model {
    type Changes = ModelChanges;
    type Violation = Violation;

    fn diff(before: &Self, after: &Self) -> ModelChanges {
        Model::compare(before, after)
    }

    fn check(before: &Self, after: &Self, changes: &ModelChanges) -> Vec<Violation> {
        let mut cx = check::Check::new(before, after, changes);
        Model::check_all(&mut cx);
        cx.finish()
    }

    fn tables(&self, each: &mut dyn FnMut(&dyn AnyTable)) {
        self.each_table(each);
        for tag in [EffectId::TAG, MaskId::TAG, MarkerId::TAG] {
            each(&Nested { tag, model: self });
        }
    }

    /// The view as it is now, put over a document undo or redo brings
    /// back, wherever that document can hold it.
    ///
    /// - The sequence showing stays showing if the restored document has
    ///   it; otherwise the restored document's own shows.
    /// - The tabs keep their present order, less those the restored
    ///   document does not have. A tab of a sequence that only the
    ///   restored document has comes back after the tab it followed
    ///   there.
    fn carry_view(present: &Self, restored: &Self) -> Self {
        let has = |id: &SequenceId| restored.sequences.contains(*id);
        let mut shown = restored.clone();
        shown.view.showing = match present.view.showing {
            Some(showing) if has(&showing) => Some(showing),
            _ => restored.view.showing,
        };
        let mut tabs: Vec<SequenceId> = present.view.tabs.iter().copied().filter(has).collect();
        for (at, tab) in restored.view.tabs.iter().enumerate() {
            // A tab closed since stays closed; one whose sequence the
            // step brings back comes back with it.
            if tabs.contains(tab) || present.sequences.contains(*tab) {
                continue;
            }
            let after = at
                .checked_sub(1)
                .and_then(|before| tabs.iter().position(|t| *t == restored.view.tabs[before]));
            tabs.insert(after.map_or(0, |position| position + 1), *tab);
        }
        shown.view.tabs = tabs;
        shown
    }
}
