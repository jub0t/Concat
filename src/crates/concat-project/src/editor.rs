// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The editing session: a project, its id mint, and undo.
//!
//! Undo is whole-state snapshots, and they are cheap: the project's
//! timelines and clips sit behind `Arc`, so a snapshot copies pointers and
//! a command copies only the clip it writes to. Impossible to get wrong in
//! the way inverse operations are, and a heavy project's two hundred undos
//! are a few megabytes, not a few hundred.
//!
//! Two refinements on "one snapshot per command":
//!
//! - A **gesture** is one undo step. A knob being dragged commits on every
//!   pause of the pointer; the caller names the gesture, and commits that
//!   name the same gesture as the last recorded step fold into it. See
//!   [`Editor::apply_within`].
//! - **View state is not an edit.** Which timeline tab is showing, and the
//!   order of the tabs, is saved with the document but never recorded in
//!   history: undo steps back through what changed the picture, not
//!   through what the person was looking at.

use std::collections::VecDeque;

use serde_json::Value;

use crate::commands::{Command, CommandError, IdMint, Outcome, apply};
use crate::doc::{DocumentSettings, from_document, to_document};
use crate::model::{Clip, Project};

const UNDO_DEPTH: usize = 200;

/// The state before one recorded step, and the gesture it belonged to.
struct Snapshot {
    before: Project,
    /// Set while the gesture that produced this step may still continue;
    /// cleared by [`Editor::end_gesture`] or by any step of another kind.
    gesture: Option<String>,
}

/// One editing session: the project, the id mint that keeps its ids unique,
/// and the undo/redo stacks. This is the object a host holds per open
/// project; everything else in the crate is reachable through it.
pub struct Editor {
    project: Project,
    mint: IdMint,
    /// Oldest snapshot at the front: overflowing the depth cap evicts from
    /// the front in constant time, where a Vec would shuffle the whole stack.
    undo: VecDeque<Snapshot>,
    redo: Vec<Project>,
}

impl Editor {
    /// A fresh, empty project.
    pub fn new() -> Self {
        Self::with_video(crate::model::VideoSettings::default())
    }

    /// A fresh editor whose first timeline renders to `video`.
    pub fn with_video(video: crate::model::VideoSettings) -> Self {
        Self {
            project: Project::with_video(video),
            mint: IdMint::default(),
            undo: VecDeque::new(),
            redo: Vec::new(),
        }
    }

    /// Restores a project from a document, adopting every id it uses so the
    /// mint can never re-issue one. Returns None when the document holds
    /// nothing recognisable.
    pub fn from_document(document: &Value) -> Option<Self> {
        let project = from_document(document)?;
        let mut mint = IdMint::default();
        mint.adopt_project(&project);
        Some(Self {
            project,
            mint,
            undo: VecDeque::new(),
            redo: Vec::new(),
        })
    }

    /// The current state, read-only: all mutation goes through
    /// [`Editor::apply`] so nothing can change without being undoable.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Applies one command, recording the state before it for undo.
    ///
    /// A command that fails leaves the project and history untouched, and
    /// the snapshot is only kept when the command reports it actually
    /// changed something ([`Outcome::applied`]), so undo never replays a
    /// no-op - a missing id, a value already in place.
    pub fn apply(&mut self, command: Command) -> Result<Outcome, CommandError> {
        self.apply_within(None, command)
    }

    /// [`Editor::apply`] as one move of a gesture.
    ///
    /// The first command naming a gesture records a step as usual; every
    /// later command naming the same gesture, with no other step recorded
    /// in between, folds into that step, so undo lands where the gesture
    /// began. Commands that set absolute values (every inspector knob) are
    /// the ones this is for. `None` is a step of its own, and ends any
    /// gesture in progress.
    pub fn apply_within(
        &mut self,
        gesture: Option<&str>,
        command: Command,
    ) -> Result<Outcome, CommandError> {
        if command.is_view_state() {
            return apply(&mut self.project, &mut self.mint, command);
        }
        let before = self.project.clone();
        let outcome = apply(&mut self.project, &mut self.mint, command)?;
        if !outcome.applied {
            return Ok(outcome);
        }
        self.tidy_touched(&before);
        let continues = match (gesture, self.undo.back()) {
            (Some(gesture), Some(last)) => last.gesture.as_deref() == Some(gesture),
            _ => false,
        };
        if !continues {
            self.undo.push_back(Snapshot {
                before,
                gesture: gesture.map(str::to_owned),
            });
            if self.undo.len() > UNDO_DEPTH {
                self.undo.pop_front();
            }
        }
        self.redo.clear();
        Ok(outcome)
    }

    /// Runs [`Clip::tidy`] over every clip the command wrote, so the clamps
    /// live in one place and a document is the same on screen as it is
    /// after a save and a reopen (audit 2026-09-23, #13). Only a clip
    /// whose `Arc` the command replaced is looked at: the rest are still
    /// the snapshot's, and were tidy already.
    fn tidy_touched(&mut self, before: &Project) {
        use std::collections::HashSet;
        use std::sync::Arc;
        let previous: std::collections::HashMap<&str, &Arc<crate::model::Timeline>> = before
            .timelines
            .iter()
            .map(|timeline| (timeline.id.as_str(), timeline))
            .collect();
        for timeline in &mut self.project.timelines {
            let old = previous.get(timeline.id.as_str()).copied();
            if old.is_some_and(|old| Arc::ptr_eq(old, timeline)) {
                continue;
            }
            let kept: HashSet<*const Clip> = old
                .map(|old| old.clips.iter().map(Arc::as_ptr).collect())
                .unwrap_or_default();
            let timeline = Arc::make_mut(timeline);
            for clip in &mut timeline.clips {
                if kept.contains(&Arc::as_ptr(clip)) {
                    continue;
                }
                let tidied = (**clip).clone().tidy();
                if tidied != **clip {
                    *clip = Arc::new(tidied);
                }
            }
        }
    }

    /// Ends the gesture in progress, if any: the next command naming it
    /// starts a new step rather than folding into the last one. Called when
    /// the pointer goes up, or when enough time has passed that a second
    /// drag of the same knob is a second edit.
    pub fn end_gesture(&mut self) {
        if let Some(last) = self.undo.back_mut() {
            last.gesture = None;
        }
    }

    /// Steps back one edit. Returns whether anything changed.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop_back() {
            Some(previous) => {
                self.redo
                    .push(std::mem::replace(&mut self.project, previous.before));
                true
            }
            None => false,
        }
    }

    /// Steps forward again. Returns whether anything changed.
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                let before = std::mem::replace(&mut self.project, next);
                self.undo.push_back(Snapshot {
                    before,
                    gesture: None,
                });
                true
            }
            None => false,
        }
    }

    /// Whether [`Editor::undo`] would do anything - what the UI's undo
    /// button greys out on.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether [`Editor::redo`] would do anything; the redo button's twin
    /// of [`Editor::can_undo`].
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The full document for saving.
    pub fn to_document(&self, settings: &DocumentSettings) -> Value {
        to_document(settings, &self.project)
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
