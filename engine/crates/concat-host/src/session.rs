// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The engine-owned editing session.
//!
//! Open a project folder and the engine holds the edit: every mutation is a
//! `concat_project` [`Command`], applied with undo recorded, and the new
//! state is what the window draws. The window never keeps a model of its
//! own; it renders the [`Project`] this session hands back.
//!
//! Saving reuses `projects::save`'s temp-file-and-rename, so the document on
//! disk is written by exactly one code path.

use concat_project::{Command, DocumentSettings, Editor, Project};
use serde::Serialize;

use crate::projects;

/// One open project: its folder, its settings and its undo history.
pub struct Session {
    /// The project folder, for saving.
    path: String,
    settings: DocumentSettings,
    editor: Editor,
}

/// What every mutating call returns: the authoritative state plus history
/// availability, so undo/redo affordances are never guessing.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EditorView {
    /// The whole project, as the engine holds it.
    pub project: Project,
    /// Whether there is something to undo.
    pub can_undo: bool,
    /// Whether there is something to redo.
    pub can_redo: bool,
    /// The settings as the session holds them - the document's own output
    /// size wins over the manifest's on open.
    pub settings: SettingsView,
    /// The id a creating command minted, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_id: Option<String>,
}

/// The session's settings, as the window shows them.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    /// Project name.
    pub name: String,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub rate_num: i64,
    /// Frame rate denominator.
    pub rate_den: i64,
}

impl Session {
    /// Opens a project folder as the editing session.
    ///
    /// A folder whose document is missing or unreadable opens as an empty
    /// project rather than failing, but a *corrupt* document is an error,
    /// because silently replacing an edit with emptiness is how projects get
    /// lost. `settings` come from the manifest; the document's own output
    /// size wins over them, because that is where an edited size was saved.
    pub fn open(path: &str, mut settings: DocumentSettings) -> Result<Session, String> {
        let editor = match projects::read_document(path) {
            Ok(document) => {
                if let Some(video) = document.get("video")
                    && let (Some(width), Some(height)) = (
                        video.get("width").and_then(|value| value.as_u64()),
                        video.get("height").and_then(|value| value.as_u64()),
                    )
                    && width > 0
                    && height > 0
                {
                    settings.width = width as u32;
                    settings.height = height as u32;
                }
                match Editor::from_document(&document) {
                    Some(editor) => editor,
                    // The settings-only manifest `create` writes: a project
                    // closed before its first edit reopens empty, it is not
                    // corrupt.
                    None if projects::is_settings_only(&document) => Editor::new(),
                    None => {
                        return Err(format!("{path} holds a document this build cannot read"));
                    }
                }
            }
            // No document yet - a project created moments ago.
            Err(_) => Editor::new(),
        };
        Ok(Session {
            path: path.to_owned(),
            settings,
            editor,
        })
    }

    /// Opens the project a [`projects::ProjectInfo`] describes.
    pub fn open_info(info: &projects::ProjectInfo) -> Result<Session, String> {
        Session::open(
            &info.path,
            DocumentSettings {
                name: info.name.clone(),
                width: info.width,
                height: info.height,
                rate_num: info.rate_num,
                rate_den: info.rate_den,
            },
        )
    }

    /// The project folder.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The session's settings.
    pub fn settings(&self) -> &DocumentSettings {
        &self.settings
    }

    /// The edit as it stands.
    pub fn project(&self) -> &Project {
        self.editor.project()
    }

    /// Whether there is something to undo.
    pub fn can_undo(&self) -> bool {
        self.editor.can_undo()
    }

    /// Whether there is something to redo.
    pub fn can_redo(&self) -> bool {
        self.editor.can_redo()
    }

    /// The current state without changing anything.
    pub fn view(&self) -> EditorView {
        self.view_with(None)
    }

    fn view_with(&self, created_id: Option<String>) -> EditorView {
        EditorView {
            project: self.editor.project().clone(),
            can_undo: self.editor.can_undo(),
            can_redo: self.editor.can_redo(),
            settings: SettingsView {
                name: self.settings.name.clone(),
                width: self.settings.width,
                height: self.settings.height,
                rate_num: self.settings.rate_num,
                rate_den: self.settings.rate_den,
            },
            created_id,
        }
    }

    /// Applies one edit command and returns the new state.
    pub fn apply(&mut self, command: Command) -> Result<EditorView, String> {
        let outcome = self
            .editor
            .apply(command)
            .map_err(|error| error.to_string())?;
        Ok(self.view_with(outcome.created_id))
    }

    /// Steps the history back one edit.
    pub fn undo(&mut self) -> EditorView {
        self.editor.undo();
        self.view()
    }

    /// Steps the history forward one edit.
    pub fn redo(&mut self) -> EditorView {
        self.editor.redo();
        self.view()
    }

    /// Takes new settings and hands back what a save must write: the folder
    /// and the document. The output size can have been edited in the
    /// preview footer and the name in the project details dialog, so both
    /// ride along here. The disk write is the caller's, so it can happen off
    /// the thread that owns the session; [`Session::save`] does both.
    pub fn prepare_save(
        &mut self,
        name: Option<&str>,
        width: Option<u32>,
        height: Option<u32>,
    ) -> (String, serde_json::Value) {
        if let Some(name) = name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                self.settings.name = trimmed.to_owned();
            }
        }
        // A zero dimension is never a real output size, only a caller bug -
        // saving it would poison the document until the next open.
        if let Some(width) = width.filter(|width| *width > 0) {
            self.settings.width = width;
        }
        if let Some(height) = height.filter(|height| *height > 0) {
            self.settings.height = height;
        }
        (self.path.clone(), self.editor.to_document(&self.settings))
    }

    /// Writes the session's document to its project folder.
    pub fn save(
        &mut self,
        name: Option<&str>,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Result<(), String> {
        let (path, document) = self.prepare_save(name, width, height);
        projects::save(&path, &document)
    }

    /// The document as it would be saved.
    pub fn document(&self) -> serde_json::Value {
        self.editor.to_document(&self.settings)
    }

    /// The active timeline flattened for rendering. This is what export and
    /// preview consume: the engine flattens its own session, so the pixels
    /// rendered are the model's, never a copy of it.
    pub fn flattened_clips(&self) -> Vec<concat_export::ExportClip> {
        concat_export::flatten::flatten_timeline(self.editor.project(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> DocumentSettings {
        DocumentSettings {
            name: "Test".to_owned(),
            width: 1920,
            height: 1080,
            rate_num: 30,
            rate_den: 1,
        }
    }

    #[test]
    fn a_fresh_project_opens_empty_and_round_trips_a_save() {
        let scratch =
            std::env::temp_dir().join(format!("concat-session-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        let info = projects::create(&scratch.to_string_lossy(), "Fresh", 1920, 1080, 30, 1)
            .expect("creates");

        let mut session = Session::open_info(&info).expect("opens the settings-only manifest");
        assert!(!session.can_undo());
        let view = session.apply(Command::AddTrack).expect("adds a track");
        assert!(view.can_undo);
        session
            .save(Some("Renamed"), Some(1280), Some(720))
            .expect("saves");

        let reopened = Session::open(&info.path, settings()).expect("reopens");
        assert_eq!(
            reopened.settings().name,
            "Test",
            "the manifest's name is what open gets"
        );
        assert_eq!(
            (reopened.settings().width, reopened.settings().height),
            (1280, 720)
        );
        assert_eq!(
            reopened.project().active().tracks.len(),
            session.project().active().tracks.len()
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_corrupt_document_is_refused() {
        let scratch =
            std::env::temp_dir().join(format!("concat-corrupt-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");
        std::fs::write(scratch.join("concat.json"), br#"{"timelines": "garbage"}"#)
            .expect("writes");
        assert!(Session::open(&scratch.to_string_lossy(), settings()).is_err());
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
