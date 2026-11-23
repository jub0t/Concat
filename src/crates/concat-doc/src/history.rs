// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Undo, as a step back to a document that was kept.
//!
//! An undo written as the opposite of each command is a second program
//! that has to be right wherever the first one is, and each opposite is
//! wrong on its own. Here nothing is run backwards. A [`Step`] is the
//! document before a change and the document after it; undo makes the
//! one before current again. It cannot be wrong in the way an opposite
//! can, and because documents share everything they have in common, a
//! step costs what the change replaced and no more.
//!
//! Beside the two documents a step keeps what was done, for people: its
//! label, who made it, the commands, when, and what was selected before
//! and after, which comes back with it.
//!
//! # The rules
//!
//! - **One history, whoever is writing.** Every step says who made it
//!   and [`undo`](History::undo) takes back the latest, theirs or not. A
//!   turn of many commands made in one draft is one step.
//! - **A new change ends redo.** Recording a step drops every step that
//!   had been undone.
//! - **Counts do not go back.** A step back restores the tables and keeps
//!   the document's [`Mint`](crate::Mint) where it is, at a new
//!   [`Revision`]: the ids an undone step made are not
//!   made again, and a plan against the old revision can be told from a
//!   plan against the new.
//! - **Small repeats are one step.** A commit that gives a
//!   [`Coalesce`] key is joined to the step before it when that step has
//!   the same key and the same origin, was recorded within the key's
//!   time, and nothing has been undone or redone in between. The joined
//!   step keeps the older "before", so it undoes all the way in one go.
//!   If joining leaves nothing changed - a nudge right and a nudge back -
//!   the step is gone.
//! - **View state is no step.** What the schema's
//!   [`carry_view`](Schema::carry_view) carries is view state: saved
//!   with the document, and no business of undo. A commit that changed
//!   nothing else is made current and is not a step; it leaves both what
//!   can be undone and what can be redone as they were. A step back or
//!   forward restores everything else and keeps the view as it is now,
//!   except what the restored document cannot hold: undoing the step
//!   that made a timeline cannot leave that timeline showing, and there
//!   the restored document's own view stands. So a change of view is
//!   never undone by itself, and undoing an earlier step does not lose
//!   it unless that step made the thing it belongs to.
//! - **The budget lets go of the oldest first.** See [`Budget`].

use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use crate::document::{ChangeSet, Commit, Document, Origin, Revision, Schema};

/// How much the history may keep.
///
/// It is checked when a step is recorded and when the budget is set,
/// never when stepping back or forward: undo and then redo always returns
/// to where it started. Over budget, the history lets go of the oldest
/// step that could still be undone, and goes on doing so. A step that
/// could still be redone goes only when there is nothing older left to
/// let go, the farthest from the present first. The last step left is
/// kept whatever it weighs: one change too big for the budget can still
/// be undone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Budget {
    max_steps: usize,
    max_bytes: usize,
}

impl Budget {
    /// At most this many steps, and about this many bytes. Fewer than
    /// one step is taken as one.
    pub const fn new(max_steps: usize, max_bytes: usize) -> Budget {
        Budget {
            max_steps: if max_steps == 0 { 1 } else { max_steps },
            max_bytes,
        }
    }

    /// How many steps may be kept.
    pub const fn max_steps(self) -> usize {
        self.max_steps
    }

    /// About how many bytes the steps may keep alive between them.
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

impl Default for Budget {
    /// A thousand steps or 256 MB, whichever is reached first.
    fn default() -> Budget {
        Budget::new(1000, 256 << 20)
    }
}

/// What lets a commit be joined to the step before it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Coalesce<K> {
    /// What kind of small repeat this is: the same command on the same
    /// thing. Equal keys join.
    pub key: K,
    /// How long after the step before this may still join it. Half a
    /// second for a nudge; as long as a turn lasts for an assistant's.
    pub within: Duration,
}

/// What is kept with a step besides its two documents.
#[derive(Clone, Debug)]
pub struct Entry<C, Sel, K = ()> {
    /// The commands that made the change, for the record.
    pub commands: Vec<C>,
    /// What was selected before the change. Comes back on undo.
    pub selection_before: Sel,
    /// What was selected after it. Comes back on redo.
    pub selection_after: Sel,
    /// When the change was made.
    pub when: Instant,
    /// Whether, and as what, it may join the step before.
    pub coalesce: Option<Coalesce<K>>,
}

impl<C, Sel, K> Entry<C, Sel, K> {
    /// An entry for one change that stands as a step of its own.
    pub fn new(commands: Vec<C>, selection_before: Sel, selection_after: Sel) -> Self {
        Entry {
            commands,
            selection_before,
            selection_after,
            when: Instant::now(),
            coalesce: None,
        }
    }

    /// The same, made at a given time.
    pub fn at(mut self, when: Instant) -> Self {
        self.when = when;
        self
    }

    /// The same, able to join the step before it under a key.
    pub fn coalescing(mut self, key: K, within: Duration) -> Self {
        self.coalesce = Some(Coalesce { key, within });
        self
    }
}

/// One change that can be taken back and made again.
pub struct Step<S, C, Sel, K = ()> {
    before: Document<S>,
    after: Document<S>,
    label: String,
    origin: Origin,
    planned_at: Revision,
    commands: Vec<C>,
    selection_before: Sel,
    selection_after: Sel,
    when: Instant,
    key: Option<K>,
    cost: usize,
}

impl<S, C, Sel, K> Step<S, C, Sel, K> {
    /// The document before the change.
    pub fn before(&self) -> &Document<S> {
        &self.before
    }

    /// The document after it.
    pub fn after(&self) -> &Document<S> {
        &self.after
    }

    /// What the change is called.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Who made it.
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// The revision it was planned against.
    pub fn planned_at(&self) -> Revision {
        self.planned_at
    }

    /// The commands that made it; those of every commit joined into it,
    /// in order.
    pub fn commands(&self) -> &[C] {
        &self.commands
    }

    /// What was selected before it.
    pub fn selection_before(&self) -> &Sel {
        &self.selection_before
    }

    /// What was selected after it.
    pub fn selection_after(&self) -> &Sel {
        &self.selection_after
    }

    /// When it was made; for a joined step, when it was last added to.
    pub fn when(&self) -> Instant {
        self.when
    }

    /// About how many bytes keeping it keeps alive.
    pub fn cost(&self) -> usize {
        self.cost
    }
}

/// What recording a commit did to the history.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Recorded {
    /// It is a new step.
    Step,
    /// It was joined to the step before it.
    Joined,
    /// It was joined to the step before it and the two together change
    /// nothing, so that step is gone.
    Cancelled,
    /// It changed only view state: made current, and not a step.
    View,
    /// It changed nothing in the tables: made current, and not a step.
    Nothing,
}

/// A commit was offered that was not made from the history's current
/// document. Nothing was recorded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotCurrent {
    /// The revision the history is at.
    pub current: Revision,
    /// The revision the commit was made from.
    pub commit_from: Revision,
}

impl fmt::Display for NotCurrent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the change was made to revision {} and the document is at revision {}",
            self.commit_from, self.current
        )
    }
}

impl std::error::Error for NotCurrent {}

/// What a step back or forward gives: the document to show now, and what
/// goes with it.
pub struct Restored<S: Schema, Sel> {
    /// The document that is now current.
    pub document: Document<S>,
    /// What to select again.
    pub selection: Sel,
    /// What is different from the document that was current a moment
    /// ago: what to draw and compile again.
    pub changes: S::Changes,
    /// The label of the step that was taken back or made again.
    pub label: String,
    /// Who made that step.
    pub origin: Origin,
}

/// The current document and the steps that led to it.
///
/// `C` is the caller's record of a command, `Sel` its selection, and `K`
/// the key by which small repeats are joined.
///
/// The history holds the current document itself, so that the document a
/// step is recorded from and the one it is stepped back to are never two
/// different ideas of "now". Make a change with
/// `history.current().begin(..)`, commit it, and hand the commit to
/// [`record`](History::record).
pub struct History<S: Schema, C, Sel, K = ()> {
    current: Document<S>,
    /// Oldest first. Those before `cursor` can be undone, the newest
    /// first; those from `cursor` on have been undone and can be redone.
    steps: VecDeque<Step<S, C, Sel, K>>,
    cursor: usize,
    budget: Budget,
    /// The sum of every step's cost.
    bytes: usize,
    /// Whether the newest step may still be joined: nothing has been
    /// undone or redone since it was recorded.
    open: bool,
}

impl<S: Schema, C, Sel: Clone, K: PartialEq> History<S, C, Sel, K> {
    /// A history that starts at a document, with nothing to undo.
    pub fn new(document: Document<S>, budget: Budget) -> Self {
        History {
            current: document,
            steps: VecDeque::new(),
            cursor: 0,
            budget,
            bytes: 0,
            open: false,
        }
    }

    /// The document as it is now.
    pub fn current(&self) -> &Document<S> {
        &self.current
    }

    /// Whether there is a step to take back.
    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    /// Whether there is a step to make again.
    pub fn can_redo(&self) -> bool {
        self.cursor < self.steps.len()
    }

    /// How many steps can be taken back.
    pub fn undo_len(&self) -> usize {
        self.cursor
    }

    /// How many steps can be made again.
    pub fn redo_len(&self) -> usize {
        self.steps.len() - self.cursor
    }

    /// The step that [`undo`](History::undo) would take back.
    pub fn next_undo(&self) -> Option<&Step<S, C, Sel, K>> {
        self.steps.get(self.cursor.checked_sub(1)?)
    }

    /// The step that [`redo`](History::redo) would make again.
    pub fn next_redo(&self) -> Option<&Step<S, C, Sel, K>> {
        self.steps.get(self.cursor)
    }

    /// The steps that can be taken back, the latest first.
    pub fn undo_steps(&self) -> impl Iterator<Item = &Step<S, C, Sel, K>> {
        self.steps.range(..self.cursor).rev()
    }

    /// The steps that can be made again, the next first.
    pub fn redo_steps(&self) -> impl Iterator<Item = &Step<S, C, Sel, K>> {
        self.steps.range(self.cursor..)
    }

    /// About how many bytes the steps keep alive between them.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// How much may be kept.
    pub fn budget(&self) -> Budget {
        self.budget
    }

    /// Changes how much may be kept, and lets go of what is now over.
    pub fn set_budget(&mut self, budget: Budget) {
        self.budget = budget;
        self.enforce();
    }

    /// Forgets every step. The current document stays.
    pub fn clear(&mut self) {
        self.steps.clear();
        self.cursor = 0;
        self.bytes = 0;
        self.open = false;
    }

    /// Makes a commit current, and a step if it is one.
    ///
    /// The commit must have been made from [`current`](History::current);
    /// one made from any other document is refused and nothing changes.
    /// What it becomes is in the answer: see [`Recorded`], and the rules
    /// at [the top of the module](self).
    pub fn record(
        &mut self,
        commit: Commit<S>,
        entry: Entry<C, Sel, K>,
    ) -> Result<Recorded, NotCurrent> {
        if !Document::ptr_eq(&commit.before, &self.current) {
            return Err(NotCurrent {
                current: self.current.revision(),
                commit_from: commit.before.revision(),
            });
        }
        let Commit {
            before,
            after,
            changes,
            label,
            origin,
            planned_at,
        } = commit;
        self.current = after.clone();
        if changes.is_empty() {
            return Ok(Recorded::Nothing);
        }
        if only_view(&before, &after) {
            return Ok(Recorded::View);
        }

        // A real change: what had been undone can no longer be redone.
        while self.steps.len() > self.cursor {
            if let Some(dropped) = self.steps.pop_back() {
                self.bytes = self.bytes.saturating_sub(dropped.cost);
            }
        }

        let Entry {
            commands,
            selection_before,
            selection_after,
            when,
            coalesce,
        } = entry;
        let joins = self.open
            && match (self.steps.back(), &coalesce) {
                (Some(last), Some(coalesce)) => {
                    last.key.as_ref() == Some(&coalesce.key)
                        && last.origin == origin
                        && when.saturating_duration_since(last.when) <= coalesce.within
                }
                _ => false,
            };
        if joins && let Some(mut last) = self.steps.pop_back() {
            self.bytes = self.bytes.saturating_sub(last.cost);
            self.cursor -= 1;
            let whole = S::diff(last.before.tables(), after.tables());
            if whole.is_empty() || only_view(&last.before, &after) {
                self.open = false;
                return Ok(Recorded::Cancelled);
            }
            last.after = after;
            last.commands.extend(commands);
            last.selection_after = selection_after;
            last.when = when;
            last.cost = whole.cost();
            self.push(last);
            return Ok(Recorded::Joined);
        }

        self.push(Step {
            before,
            after,
            label,
            origin,
            planned_at,
            commands,
            selection_before,
            selection_after,
            when,
            key: coalesce.map(|coalesce| coalesce.key),
            cost: changes.cost(),
        });
        Ok(Recorded::Step)
    }

    /// Takes back the latest step, whoever made it.
    ///
    /// The answer is the document to show and the selection to restore;
    /// `None` when there is nothing to take back. The document has the
    /// tables the step started from, the view as it is now (see
    /// [the rules](self)), the mint as it is now, and a new revision.
    pub fn undo(&mut self) -> Option<Restored<S, Sel>> {
        let step = self.steps.get(self.cursor.checked_sub(1)?)?;
        let restored = self.restore(&step.before, &step.selection_before, step);
        self.cursor -= 1;
        Some(self.make_current(restored))
    }

    /// Makes the last step taken back happen again.
    ///
    /// `None` when nothing has been taken back since the last change.
    pub fn redo(&mut self) -> Option<Restored<S, Sel>> {
        let step = self.steps.get(self.cursor)?;
        let restored = self.restore(&step.after, &step.selection_after, step);
        self.cursor += 1;
        Some(self.make_current(restored))
    }

    /// One of a step's two documents as it should be shown now.
    fn restore(
        &self,
        root: &Document<S>,
        selection: &Sel,
        step: &Step<S, C, Sel, K>,
    ) -> Restored<S, Sel> {
        let tables = with_present_view(self.current.tables(), root.tables());
        let document = Document::assemble(
            tables,
            self.current.mint().clone(),
            self.current.revision().next(),
        );
        Restored {
            changes: document.changes_since(&self.current),
            document,
            selection: selection.clone(),
            label: step.label.clone(),
            origin: step.origin,
        }
    }

    fn make_current(&mut self, restored: Restored<S, Sel>) -> Restored<S, Sel> {
        self.current = restored.document.clone();
        self.open = false;
        restored
    }

    fn push(&mut self, step: Step<S, C, Sel, K>) {
        self.bytes = self.bytes.saturating_add(step.cost);
        self.steps.push_back(step);
        self.cursor = self.steps.len();
        self.open = true;
        self.enforce();
    }

    fn over(&self) -> bool {
        self.steps.len() > self.budget.max_steps || self.bytes > self.budget.max_bytes
    }

    /// Lets go of steps until the history is within its budget or has
    /// one step left: the oldest that could be undone first, and one
    /// that could be redone only when none of those is left.
    fn enforce(&mut self) {
        while self.over() && self.steps.len() > 1 {
            let dropped = if self.cursor > 0 {
                self.cursor -= 1;
                self.steps.pop_front()
            } else {
                self.steps.pop_back()
            };
            match dropped {
                Some(step) => self.bytes = self.bytes.saturating_sub(step.cost),
                None => break,
            }
        }
    }
}

/// Whether the only difference between two documents is view state: the
/// newer one's view, put over the older, is the newer one.
fn only_view<S: Schema>(older: &Document<S>, newer: &Document<S>) -> bool {
    let carried = S::carry_view(newer.tables(), older.tables());
    S::diff(&carried, newer.tables()).is_empty()
}

/// `root` with the present view put over it by the schema, if what comes
/// of that keeps the schema's rules; `root` as it is if not. A document
/// is always valid, and a view that the restored tables cannot hold is
/// the one thing a step back is allowed to lose.
fn with_present_view<S: Schema>(present: &S, root: &S) -> S {
    let carried = S::carry_view(present, root);
    let moved = S::diff(root, &carried);
    if !moved.is_empty() && S::check(root, &carried, &moved).is_empty() {
        carried
    } else {
        root.clone()
    }
}
