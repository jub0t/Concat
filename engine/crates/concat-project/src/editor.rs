// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The editing session: a project, its id mint, and undo.
//!
//! Undo is whole-state snapshots. The document is small - a heavy project is
//! a few hundred kilobytes - so cloning per edit is cheaper than maintaining
//! inverse operations and impossible to get wrong. The depth cap bounds the
//! memory; two hundred undos of a large project is tens of megabytes, which
//! an editor holding gigabytes of frame data does not notice.

use std::collections::VecDeque;

use serde_json::Value;

use crate::commands::{Command, CommandError, IdMint, Outcome, apply};
use crate::doc::{DocumentSettings, from_document, to_document};
use crate::model::Project;

const UNDO_DEPTH: usize = 200;

/// One editing session: the project, the id mint that keeps its ids unique,
/// and the undo/redo stacks. This is the object a host holds per open
/// project; everything else in the crate is reachable through it.
pub struct Editor {
    project: Project,
    mint: IdMint,
    /// Oldest snapshot at the front: overflowing the depth cap evicts from
    /// the front in constant time, where a Vec would shuffle the whole stack.
    undo: VecDeque<Project>,
    redo: Vec<Project>,
}

impl Editor {
    /// A fresh, empty project.
    pub fn new() -> Self {
        Self {
            project: Project::new(),
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
        let before = self.project.clone();
        let outcome = apply(&mut self.project, &mut self.mint, command)?;
        if outcome.applied {
            self.undo.push_back(before);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.pop_front();
            }
            self.redo.clear();
        }
        Ok(outcome)
    }

    /// Steps back one edit. Returns whether anything changed.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop_back() {
            Some(previous) => {
                self.redo
                    .push(std::mem::replace(&mut self.project, previous));
                true
            }
            None => false,
        }
    }

    /// Steps forward again. Returns whether anything changed.
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo
                    .push_back(std::mem::replace(&mut self.project, next));
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
