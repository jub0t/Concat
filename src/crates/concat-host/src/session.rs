// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The engine-owned editing session.
//!
//! Open a project folder and the engine holds the edit: a
//! [`Document`] of the [`Model`], and the [`History`] of how it got there.
//! Every edit is a [`Command`], made as one transaction
//! ([`concat_doc::commands::run`]) and recorded as one step of undo. The
//! window never keeps a model of its own: it draws the snapshot
//! [`Session::document`] hands it, which costs one pointer to copy.
//!
//! # The project folder
//!
//! A project is a folder holding two files:
//!
//! - **`concat.json`**, the manifest: the project's name and the frame its
//!   first timeline starts with. It is what the recents list and the
//!   launch screen read, and it is written when the project is made and
//!   again whenever the name changes.
//! - **`project.concat`**, the document, in concat-doc's file format
//!   ([`concat_doc::file`]), saved through [`concat_doc::save`]: written
//!   whole beside the old one, flushed, read back, the old save kept as
//!   `.prev`, then swapped in. A folder with a manifest and no document is
//!   a project made moments ago, and opens as one empty timeline in the
//!   manifest's frame.
//!
//! A folder whose manifest holds the edit itself is a project of an
//! earlier Concat; it is refused, said plainly, and nothing is written.
//!
//! # Gestures
//!
//! A knob dragged through a hundred values is one undo step. Each command
//! of the drag is applied with [`Session::apply_within`] naming the
//! gesture; the history joins each to the step before while the name and
//! its generation are the same, and [`Session::end_gesture`] starts a new
//! generation, so the next drag of the same knob is a step of its own.

use std::path::{Path, PathBuf};
use std::time::Duration;

use concat_doc::commands::{self, Applied, Command};
use concat_doc::file::Note;
use concat_doc::model::{Frame, ItemId, Model, ModelChanges, SequenceId};
use concat_doc::{Budget, Document, Entry, History, Origin};

use crate::projects;

/// The name of the document's file in a project folder.
pub const DOCUMENT: &str = "project.concat";

/// How long one gesture may go on and still be one step: a drag held for
/// longer than this is two steps. Long enough for any drag a person makes.
const GESTURE_SPAN: Duration = Duration::from_secs(600);

/// Who writes the edits of this process: the window and the API alike.
const ORIGIN: Origin = Origin(0);

/// What the person selected, as the history keeps it beside each step and
/// gives back on undo and redo.
pub type Selection = Vec<ItemId>;

/// What joins a command to the step before it: the gesture's name and its
/// generation.
type GestureKey = (String, u64);

/// The settings a project's manifest holds: what is not the document.
#[derive(Clone, PartialEq, Debug)]
pub struct ProjectSettings {
    /// The project's name, as the title bar and the recents list show it.
    pub name: String,
    /// The frame a project with no document yet starts its first timeline
    /// in. A document that opens brings each timeline's own frame, and
    /// those are what draws.
    pub frame: Frame,
}

/// What every edit, undo and redo returns: what the edit made, what
/// changed, and whether there is now something to undo or redo, so the
/// window's affordances are never guessing.
#[derive(Clone, Debug)]
pub struct Change {
    /// What the command made and what it would select. On undo and redo,
    /// `selection` is the selection that comes back with the step and
    /// `created` is none.
    pub applied: Applied,
    /// What changed between the document before and the one now current:
    /// what to draw and compile again. `None` when nothing changed.
    pub changes: Option<ModelChanges>,
    /// Whether there is something to undo.
    pub can_undo: bool,
    /// Whether there is something to redo.
    pub can_redo: bool,
}

/// A save captured on the thread that owns the session, to be written on
/// any thread with [`write_save`]: the folder, the name for the manifest,
/// the document as it stood, and its place in this process's write order.
#[derive(Clone, Debug)]
pub struct PendingSave {
    /// The project folder.
    pub folder: PathBuf,
    /// The project's name, for the manifest.
    pub name: String,
    /// The frame the manifest records, the showing timeline's.
    pub frame: Frame,
    /// The document to write.
    pub document: Document<Model>,
    /// Where this save stands in this process's write order: an older
    /// snapshot that reaches the disk after a newer one is skipped.
    pub ticket: projects::Ticket,
}

/// One open project: its folder, its settings, its document and its undo
/// history.
pub struct Session {
    /// The project folder.
    path: String,
    /// The manifest's settings; the name may change under a save.
    settings: ProjectSettings,
    /// The document and its steps.
    history: History<Model, Command, Selection, GestureKey>,
    /// What the open set aside or put right, for the window to say.
    notes: Vec<Note>,
    /// The links to retired effects the open rewrote, old package and new.
    upgraded: Vec<(String, String)>,
    /// What the window has selected, kept with the next step.
    selection: Selection,
    /// The generation of every gesture; [`Session::end_gesture`] moves it
    /// on.
    gesture: u64,
}

impl Session {
    /// Opens a project folder as the editing session.
    ///
    /// A folder with a manifest and no document opens as an empty
    /// project: one timeline in `settings.frame`, four lanes. A document
    /// that is there opens through [`concat_doc::save::open`] (a save left
    /// waiting beside it is taken up first); what it sets aside or puts
    /// right is in [`Session::notes`]. A document that cannot be read, or
    /// a project of an earlier Concat, is an error: answering with an
    /// empty project would let the next save replace the edit.
    pub fn open(path: &str, settings: ProjectSettings) -> Result<Session, String> {
        let _ = (path, settings);
        todo!()
    }

    /// Opens the project a [`projects::ProjectInfo`] describes.
    pub fn open_info(info: &projects::ProjectInfo) -> Result<Session, String> {
        let _ = info;
        todo!()
    }

    /// What this open set aside or put right, in plain words: parts a later
    /// build wrote, parts a damaged file broke, a save that had been left
    /// waiting. Empty for most documents.
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// The links to retired effects this open read as links to the
    /// effects that stand in for them, old package and new. Empty when the
    /// document named none; the change is the document's once it is saved.
    pub fn upgraded(&self) -> &[(String, String)] {
        &self.upgraded
    }

    /// The project folder.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The project's name.
    pub fn name(&self) -> &str {
        &self.settings.name
    }

    /// The sequence showing, if the document has one.
    pub fn showing(&self) -> Option<SequenceId> {
        self.history.current().view.showing
    }

    /// The frame of the sequence showing: what the monitor and an export
    /// draw. The manifest's when the document has no sequence.
    pub fn frame(&self) -> Frame {
        todo!()
    }

    /// The document as it stands: a snapshot, one pointer to copy, that
    /// later edits never change.
    pub fn document(&self) -> Document<Model> {
        self.history.current().clone()
    }

    /// Whether there is something to undo.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether there is something to redo.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// The label of the step undo would take back, for the menu.
    pub fn undo_label(&self) -> Option<&str> {
        self.history.next_undo().map(|step| step.label())
    }

    /// The label of the step redo would make again.
    pub fn redo_label(&self) -> Option<&str> {
        self.history.next_redo().map(|step| step.label())
    }

    /// Tells the session what is selected, so that the next step keeps it
    /// and undo gives it back.
    pub fn set_selection(&mut self, items: Selection) {
        self.selection = items;
    }

    /// Applies one command as a step of its own.
    ///
    /// A command that changes nothing makes no step and answers with
    /// `changes: None`. A command the model's rules refuse leaves the
    /// document as it was and answers with why, in plain words.
    pub fn apply(&mut self, command: &Command) -> Result<Change, String> {
        self.apply_within(None, command)
    }

    /// Applies one command as a move of `gesture`: while the name is the
    /// same and [`Session::end_gesture`] has not been called, each command
    /// is joined to the step before, so a drag is one undo step. `None` is
    /// a step of its own.
    pub fn apply_within(
        &mut self,
        gesture: Option<&str>,
        command: &Command,
    ) -> Result<Change, String> {
        let _ = (gesture, command, GESTURE_SPAN, ORIGIN);
        todo!()
    }

    /// Ends the gesture in progress: the next command naming it starts a
    /// new undo step.
    pub fn end_gesture(&mut self) {
        self.gesture = self.gesture.wrapping_add(1);
    }

    /// Steps the history back one step. `None` when there is nothing to
    /// undo.
    pub fn undo(&mut self) -> Option<Change> {
        todo!()
    }

    /// Steps the history forward one step. `None` when there is nothing to
    /// redo.
    pub fn redo(&mut self) -> Option<Change> {
        todo!()
    }

    /// Takes a new name, if there is one, and captures what a save must
    /// write. The disk write is [`write_save`]'s, so it can happen off the
    /// thread that owns the session; [`Session::save`] does both.
    pub fn prepare_save(&mut self, name: Option<&str>) -> PendingSave {
        let _ = name;
        todo!()
    }

    /// Writes the document, and the manifest with the name, to the project
    /// folder.
    pub fn save(&mut self, name: Option<&str>) -> Result<(), String> {
        let pending = self.prepare_save(name);
        write_save(&pending)
    }

    /// The sequence showing, flattened into what the renderer draws: what
    /// export and the monitor consume. See [`crate::render`].
    pub fn flattened_clips(&self) -> Vec<concat_export::ExportClip> {
        todo!()
    }
}

/// Writes a save captured by [`Session::prepare_save`]: the document
/// through [`concat_doc::save::save_document`], then the manifest. A
/// snapshot older than one already written is skipped.
pub fn write_save(pending: &PendingSave) -> Result<(), String> {
    let _ = pending;
    todo!()
}

/// The document's file in the project folder `root`.
pub fn document_path(root: &Path) -> PathBuf {
    root.join(DOCUMENT)
}
