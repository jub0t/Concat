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
use std::num::NonZeroU32;

use concat_doc::model::{Color, ColorSpace, Frame, ItemId, Model, ModelChanges, SequenceId};
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
        let root = Path::new(path);
        let file = document_path(root);
        let (document, notes) = if file.is_file() || concat_doc::save::pending(&file).is_some() {
            // The file as this open found it, kept before any save of this
            // run can replace it.
            if file.is_file()
                && let Err(error) = std::fs::copy(&file, opened_copy_path(root))
            {
                log::warn!(
                    "project: keeping a copy of the document as opened failed: {error}; it opens all the same"
                );
            }
            let opened = concat_doc::save::open(&file).map_err(|error| error.to_string())?;
            (opened.document, opened.notes)
        } else {
            match projects::read_document(path)? {
                Some(manifest) if !projects::is_settings_only(&manifest) => {
                    return Err(format!(
                        "{path} was saved by an earlier Concat, whose projects this one does not open"
                    ));
                }
                // A manifest and no document: a project made moments ago.
                Some(_) | None => (fresh(&settings.frame)?, Vec::new()),
            }
        };
        if !notes.is_empty() {
            log::warn!(
                "project: opened with {} parts set aside or put right, kept in the file",
                notes.len()
            );
            for note in &notes {
                log::warn!("project: {note}");
            }
        }
        let (document, upgraded) = upgrade_links(document);
        Ok(Session {
            path: path.to_owned(),
            settings,
            history: History::new(document, Budget::default()),
            notes,
            upgraded,
            selection: Vec::new(),
            gesture: 0,
        })
    }

    /// Opens the project a [`projects::ProjectInfo`] describes.
    pub fn open_info(info: &projects::ProjectInfo) -> Result<Session, String> {
        Session::open(&info.path, settings_of(info)?)
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
        let document = self.history.current();
        document
            .view
            .showing
            .and_then(|showing| document.sequences.get(showing))
            .map_or_else(|| self.settings.frame.clone(), |sequence| sequence.frame.clone())
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
        let outcome = commands::run(self.history.current(), command, ORIGIN)
            .map_err(|error| error.to_string())?;
        let Some(commit) = outcome.commit else {
            return Ok(self.change(outcome.applied, None));
        };
        let changes = commit.changes().clone();
        let after = outcome
            .applied
            .selection
            .clone()
            .unwrap_or_else(|| self.selection.clone());
        let mut entry = Entry::new(vec![command.clone()], self.selection.clone(), after.clone());
        if let Some(name) = gesture {
            entry = entry.coalescing((name.to_owned(), self.gesture), GESTURE_SPAN);
        }
        self.history
            .record(commit, entry)
            .map_err(|error| error.to_string())?;
        self.selection = after;
        Ok(self.change(outcome.applied, Some(changes)))
    }

    fn change(&self, applied: Applied, changes: Option<ModelChanges>) -> Change {
        Change {
            applied,
            changes,
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
        }
    }

    fn restored(&mut self, restored: concat_doc::Restored<Model, Selection>) -> Change {
        self.selection.clone_from(&restored.selection);
        let applied = Applied {
            selection: Some(restored.selection),
            ..Applied::default()
        };
        self.change(applied, Some(restored.changes))
    }

    /// Ends the gesture in progress: the next command naming it starts a
    /// new undo step.
    pub fn end_gesture(&mut self) {
        self.gesture = self.gesture.wrapping_add(1);
    }

    /// Steps the history back one step. `None` when there is nothing to
    /// undo.
    pub fn undo(&mut self) -> Option<Change> {
        let restored = self.history.undo()?;
        Some(self.restored(restored))
    }

    /// Steps the history forward one step. `None` when there is nothing to
    /// redo.
    pub fn redo(&mut self) -> Option<Change> {
        let restored = self.history.redo()?;
        Some(self.restored(restored))
    }

    /// Takes a new name, if there is one, and captures what a save must
    /// write. The disk write is [`write_save`]'s, so it can happen off the
    /// thread that owns the session; [`Session::save`] does both.
    pub fn prepare_save(&mut self, name: Option<&str>) -> PendingSave {
        if let Some(name) = name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                self.settings.name = trimmed.to_owned();
            }
        }
        PendingSave {
            folder: PathBuf::from(&self.path),
            name: self.settings.name.clone(),
            frame: self.frame(),
            document: self.document(),
            ticket: projects::ticket(),
        }
    }

    /// Writes the document, and the manifest with the name, to the project
    /// folder.
    pub fn save(&mut self, name: Option<&str>) -> Result<(), String> {
        let pending = self.prepare_save(name);
        write_save(&pending)
    }

    /// The sequence showing, flattened into what the renderer draws: what
    /// export and the monitor consume. See [`concat_export::flatten`].
    pub fn flattened_clips(&self) -> Vec<concat_export::ExportClip> {
        let document = self.history.current();
        match document.view.showing {
            Some(sequence) => {
                concat_export::flatten::clips(document, sequence, Some(Path::new(&self.path)))
            }
            None => Vec::new(),
        }
    }
}

/// Writes a save captured by [`Session::prepare_save`]: the document
/// through [`concat_doc::save::save_document`], then the manifest. A
/// snapshot older than one already written is skipped.
pub fn write_save(pending: &PendingSave) -> Result<(), String> {
    projects::save_document_ordered(
        &pending.folder,
        &document_path(&pending.folder),
        &pending.name,
        &pending.frame,
        &pending.document,
        pending.ticket,
    )
    .map(|_| ())
}

/// Where the copy of the document as it was last opened is kept, beside
/// it: what the person can go back to whatever this run's saves did.
pub fn opened_copy_path(root: &Path) -> PathBuf {
    root.join("project.opened.concat")
}

/// The settings a recents entry or a manifest gives.
pub fn settings_of(info: &projects::ProjectInfo) -> Result<ProjectSettings, String> {
    let rate = concat_doc::Rate::new(
        u32::try_from(info.rate_num).unwrap_or(0),
        u32::try_from(info.rate_den).unwrap_or(0),
    )
    .map_err(|error| format!("{} has no frame rate it can play at: {error}", info.path))?;
    let (Some(width), Some(height)) = (NonZeroU32::new(info.width), NonZeroU32::new(info.height))
    else {
        return Err(format!("{} has a frame with no size", info.path));
    };
    let [r, g, b, a] = info.canvas;
    Ok(ProjectSettings {
        name: info.name.clone(),
        frame: Frame {
            width,
            height,
            rate,
            canvas: Color { r, g, b, a },
            color: ColorSpace::Sdr,
        },
    })
}

/// The document of a project with none yet: one timeline in `frame`,
/// with its lanes, showing.
fn fresh(frame: &Frame) -> Result<Document<Model>, String> {
    let empty = Document::<Model>::new();
    let made = commands::run(&empty, &Command::AddTimeline, ORIGIN)
        .map_err(|error| error.to_string())?;
    let mut document = made.commit.map_or(empty, |commit| commit.into_document());
    if let Some(sequence) = document.view.showing {
        let framed = commands::run(
            &document,
            &Command::SetTimelineVideo {
                sequence,
                frame: frame.clone(),
            },
            ORIGIN,
        )
        .map_err(|error| error.to_string())?;
        if let Some(commit) = framed.commit {
            document = commit.into_document();
        }
    }
    Ok(document)
}

/// The document with every link to a retired effect read as a link to the
/// effect that stands in for it (its `[[replaces]]`), knobs carried over,
/// and the pairs of package ids that changed, each once. Made as the
/// document's starting point, not as a step of undo; a rewrite the
/// model's rules refuse is left out and the document opens as it was.
pub(crate) fn upgrade_links(document: Document<Model>) -> (Document<Model>, Vec<(String, String)>) {
    use concat_doc::model::{Content, Effect};
    let catalogue = concat_effects::Catalogue::builtin();
    let mut upgraded: Vec<(String, String)> = Vec::new();
    let mut upgrade = |effect: &mut Effect| -> bool {
        // Keys travel at their ticks, written as numbers and read back.
        let mut link = concat_export::flatten::link_of(effect, |at| at.get() as f64);
        let old = link.id.clone();
        if !catalogue.upgrade(&mut link) {
            return false;
        }
        if old != link.id && !upgraded.contains(&(old.clone(), link.id.clone())) {
            upgraded.push((old, link.id.clone()));
        }
        effect.package = link.id;
        effect.enabled = link.enabled;
        effect.params = link
            .params
            .iter()
            .map(|(name, value)| (name.clone(), concat_doc::model::Param::fixed(Some(*value))))
            .collect();
        for (name, keys) in link.keys {
            let param = effect
                .params
                .entry(name)
                .or_insert_with(|| concat_doc::model::Param::fixed(None));
            param.keys = keys
                .into_iter()
                .map(|key| concat_doc::model::Key {
                    at: concat_doc::Tick::new(key.at.round() as i64),
                    value: key.value,
                    ease: concat_doc::model::Ease::Bezier {
                        x1: key.ease.x1,
                        y1: key.ease.y1,
                        x2: key.ease.x2,
                        y2: key.ease.y2,
                    },
                })
                .collect();
        }
        true
    };
    let mut changed = Vec::new();
    for (id, item) in document.items.iter() {
        let mut item = item.as_ref().clone();
        let mut any = false;
        let chains = item.effects.iter_mut().chain(item.filters.iter_mut());
        for effect in chains.filter_map(concat_doc::model::Entry::known_mut) {
            any |= upgrade(effect);
        }
        if let Content::Transition(effect) = &mut item.content {
            any |= upgrade(effect);
        }
        if let Some(cut_in) = &mut item.cut_in {
            any |= upgrade(&mut cut_in.effect);
        }
        if any {
            changed.push((id, item));
        }
    }
    if changed.is_empty() {
        return (document, upgraded);
    }
    let mut draft = document.begin("Upgrade effects", ORIGIN);
    for (id, item) in changed {
        if draft.items.replace(id, item).is_err() {
            return (document, Vec::new());
        }
    }
    match draft.commit() {
        Ok(commit) => (commit.into_document(), upgraded),
        Err(error) => {
            log::warn!("project: the retired effects could not be read as their stand-ins: {error}");
            (document, Vec::new())
        }
    }
}

/// The document's file in the project folder `root`.
pub fn document_path(root: &Path) -> PathBuf {
    root.join(DOCUMENT)
}

/// The document of the project folder `root`, read for a look and not
/// for editing (the launch screen's poster and length): `None` when the
/// folder has no document yet. Nothing is written, a save left waiting
/// included.
pub fn read_document(root: &Path) -> Result<Option<Document<Model>>, String> {
    let file = document_path(root);
    let bytes = match std::fs::read(&file) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read {}: {error}", file.display())),
    };
    concat_doc::file::read(&bytes)
        .map(|opened| Some(opened.document))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_doc::commands::LaneFlag;

    fn made(scratch: &Path) -> projects::ProjectInfo {
        projects::create(
            &scratch.to_string_lossy(),
            "Fresh",
            1280,
            720,
            25,
            1,
            [0, 0, 0, 255],
        )
        .expect("creates")
    }

    fn a_lane(session: &Session) -> concat_doc::model::LaneId {
        let document = session.document();
        let showing = session.showing().expect("a timeline shows");
        let lane = document.lanes.index().of(showing).next();
        lane.expect("a lane").0
    }

    #[test]
    fn a_fresh_project_opens_as_one_timeline_in_its_frame_and_saves_and_reopens() {
        let scratch = tempfile::tempdir().expect("scratch");
        let info = made(scratch.path());
        let mut session = Session::open_info(&info).expect("opens the manifest alone");
        assert!(!session.can_undo());
        assert_eq!(session.frame().width.get(), 1280);
        assert_eq!(session.frame().rate.num(), 25);
        let change = session
            .apply(&Command::AddTrack { bottom: false })
            .expect("adds a lane");
        assert!(change.can_undo && change.changes.is_some());
        assert!(change.applied.created.is_some());
        session.save(Some("Renamed")).expect("saves");
        assert!(document_path(Path::new(&info.path)).is_file());

        let reopened = Session::open_info(&projects::open(&info.path).expect("reads the manifest"))
            .expect("reopens");
        assert_eq!(reopened.name(), "Renamed", "the manifest took the new name");
        assert!(reopened.notes().is_empty());
        assert!(reopened.document().same_content(&session.document()));
        assert!(!reopened.can_undo(), "history is the session's, not the file's");
    }

    #[test]
    fn undo_and_redo_step_through_edits_and_say_what_changed() {
        let scratch = tempfile::tempdir().expect("scratch");
        let mut session = Session::open_info(&made(scratch.path())).expect("opens");
        let before = session.document();
        session
            .apply(&Command::AddTrack { bottom: true })
            .expect("adds");
        let undone = session.undo().expect("undoes");
        assert!(!undone.can_undo && undone.can_redo);
        assert!(session.document().same_content(&before));
        let redone = session.redo().expect("redoes");
        assert!(redone.can_undo && !redone.can_redo);
        assert!(session.undo().is_some() && session.undo().is_none());
    }

    #[test]
    fn a_gesture_is_one_step_until_it_ends() {
        let scratch = tempfile::tempdir().expect("scratch");
        let mut session = Session::open_info(&made(scratch.path())).expect("opens");
        let lane = a_lane(&session);
        for value in [true, false, true] {
            session
                .apply_within(
                    Some("mute"),
                    &Command::SetTrackFlag {
                        lane,
                        flag: LaneFlag::Muted,
                        value,
                    },
                )
                .expect("mutes");
        }
        session.end_gesture();
        session
            .apply_within(
                Some("mute"),
                &Command::SetTrackFlag {
                    lane,
                    flag: LaneFlag::Muted,
                    value: false,
                },
            )
            .expect("unmutes");
        assert!(session.undo().is_some(), "the second gesture");
        assert!(session.undo().is_some(), "the first, whole");
        assert!(session.undo().is_none());
    }

    #[test]
    fn a_command_that_changes_nothing_is_no_step() {
        let scratch = tempfile::tempdir().expect("scratch");
        let mut session = Session::open_info(&made(scratch.path())).expect("opens");
        let lane = a_lane(&session);
        let change = session
            .apply(&Command::SetTrackFlag {
                lane,
                flag: LaneFlag::Visible,
                value: true,
            })
            .expect("applies");
        assert!(change.changes.is_none() && !change.can_undo);
    }

    #[test]
    fn a_project_of_an_earlier_concat_is_refused_and_left_alone() {
        let scratch = tempfile::tempdir().expect("scratch");
        let info = made(scratch.path());
        let manifest = projects::manifest_path(Path::new(&info.path));
        let old = br#"{"concat":"0.2.12","name":"Old","video":{"width":1920,"height":1080,"rateNum":30,"rateDen":1},"timelines":[]}"#;
        std::fs::write(&manifest, old).expect("writes");
        let error = Session::open_info(&info).err().expect("refused");
        assert!(error.contains("earlier Concat"), "{error}");
        assert_eq!(std::fs::read(&manifest).expect("reads"), old);
        assert!(!document_path(Path::new(&info.path)).exists());
    }

    #[test]
    fn a_damaged_document_is_refused_rather_than_replaced() {
        let scratch = tempfile::tempdir().expect("scratch");
        let info = made(scratch.path());
        let file = document_path(Path::new(&info.path));
        std::fs::write(&file, br#"{"concat-project":1,"items":{"#).expect("writes");
        assert!(Session::open_info(&info).is_err());
        assert_eq!(
            std::fs::read(&file).expect("reads"),
            br#"{"concat-project":1,"items":{"#
        );
    }
}
