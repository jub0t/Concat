//! The engine-owned editing session, exposed to the UI.
//!
//! One session at a time, held in managed state: open a project and the
//! engine holds the edit; every mutation arrives as a `wolfcut_project`
//! [`Command`], is applied with undo recorded, and the new state goes back
//! over the wire. This is the API `lib/editor.ts` mirrors; the provisional
//! TypeScript model it replaced is gone - see
//! `engine/docs/decisions/0007-engine-owns-the-project.md`.
//!
//! Saving reuses `projects::save`'s temp-file-and-rename, so the document on
//! disk is written by exactly one code path whichever side owns the model.

use std::sync::Mutex;

use wolfcut_project::{Command, DocumentSettings, Editor};
use serde::Serialize;

use crate::projects;

/// The one editing session, or None before a project is opened.
pub struct EditorState(pub Mutex<Option<Session>>);

pub struct Session {
    /// The project folder, for saving.
    path: String,
    settings: DocumentSettings,
    editor: Editor,
}

/// What every mutating call returns: the authoritative state plus history
/// availability, so the UI's undo/redo affordances are never guessing.
#[derive(Serialize)]
#[cfg_attr(feature = "types", derive(ts_rs::TS))]
#[cfg_attr(feature = "types", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct EditorView {
    project: wolfcut_project::Project,
    can_undo: bool,
    can_redo: bool,
    /// The settings as the session holds them - the document's own output
    /// size wins over the manifest's on open, exactly as the old loader
    /// preferred it.
    settings: SettingsView,
    /// The id a creating command minted, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "types", ts(optional))]
    created_id: Option<String>,
}

#[derive(Serialize)]
#[cfg_attr(feature = "types", derive(ts_rs::TS))]
#[cfg_attr(feature = "types", ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    name: String,
    width: u32,
    height: u32,
    // i64 over the wire is a plain JSON number, not a bigint - serde_json
    // writes it bare and the UI reads it with JSON.parse.
    #[cfg_attr(feature = "types", ts(type = "number"))]
    rate_num: i64,
    #[cfg_attr(feature = "types", ts(type = "number"))]
    rate_den: i64,
}

fn view(session: &Session, created_id: Option<String>) -> EditorView {
    EditorView {
        project: session.editor.project().clone(),
        can_undo: session.editor.can_undo(),
        can_redo: session.editor.can_redo(),
        settings: SettingsView {
            name: session.settings.name.clone(),
            width: session.settings.width,
            height: session.settings.height,
            rate_num: session.settings.rate_num,
            rate_den: session.settings.rate_den,
        },
        created_id,
    }
}

/// The active timeline flattened for rendering, plus the session settings.
///
/// This is what export and preview consume. It used to be a clip list the
/// UI flattened and sent over the wire, which made the frontend's copy of
/// the model - not the model - the thing that rendered; see engine decision
/// 0009. Now the engine flattens its own session and the wire carries only
/// what the UI genuinely owns (the destination, the quality, its rasterised
/// titles).
pub fn flattened_clips(
    state: &EditorState,
) -> Result<(Vec<wolfcut_export::ExportClip>, DocumentSettings), String> {
    with_session(state, |session| {
        Ok((
            wolfcut_export::flatten::flatten_timeline(session.editor.project(), None),
            session.settings.clone(),
        ))
    })
}

/// The open session's document, project folder and settings, for features
/// that package the current edit (saving it as a template) rather than
/// editing it.
pub fn session_snapshot(
    state: &EditorState,
) -> Result<(serde_json::Value, String, DocumentSettings), String> {
    with_session(state, |session| {
        Ok((
            session.editor.to_document(&session.settings),
            session.path.clone(),
            session.settings.clone(),
        ))
    })
}

fn with_session<T>(
    state: &EditorState,
    operation: impl FnOnce(&mut Session) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.0.lock().map_err(|_| "editor state poisoned".to_owned())?;
    let session = guard.as_mut().ok_or("no project is open")?;
    operation(session)
}

/// The editor a parsed document opens as.
///
/// `from_document` answers with an `Option`, which collapses two different
/// situations into `None`: a document whose edit will not load, and one that
/// never had an edit in it. `projects::create` writes the second kind - a
/// settings-only manifest, the full document arriving on the first save - so
/// treating every `None` as corrupt made every new project fail to open.
///
/// The keys are the signal. A file that names neither `timelines` nor
/// `tracks` is claiming no edit and has none to lose; anything that does
/// claim one and still will not load is corrupt, and must stay an error
/// rather than open as emptiness that the next save writes over the top of.
fn editor_for(document: &serde_json::Value, path: &str) -> Result<Editor, String> {
    if let Some(editor) = Editor::from_document(document) {
        return Ok(editor);
    }
    if document.get("timelines").is_none() && document.get("tracks").is_none() {
        return Ok(Editor::new());
    }
    Err(format!("{path} holds a document this build cannot read"))
}

/// Opens a project folder as the editing session and returns its state.
///
/// A folder whose document is missing or unreadable opens as an empty
/// project rather than failing - the same grace the TS loader extends -
/// but a *corrupt* document is an error, because silently replacing an
/// edit with emptiness is how projects get lost.
#[tauri::command]
pub async fn editor_open(
    app: tauri::AppHandle,
    state: tauri::State<'_, EditorState>,
    path: String,
    name: String,
    width: u32,
    height: u32,
    rate_num: i64,
    rate_den: i64,
) -> Result<EditorView, String> {
    // Reading and parsing the document is the slow part and needs no state,
    // so it runs on a blocking thread; the session lock is taken only for the
    // in-memory install below.
    let (path, settings, editor) = tauri::async_runtime::spawn_blocking(move || {
        let mut settings = DocumentSettings { name, width, height, rate_num, rate_den };
        let editor = match projects::read_document(&path) {
            Ok(document) => {
                // The document's frame wins over the manifest: it is where an
                // edited output size was saved.
                if let Some(video) = document.get("video") {
                    if let (Some(width), Some(height)) = (
                        video.get("width").and_then(|value| value.as_u64()),
                        video.get("height").and_then(|value| value.as_u64()),
                    ) {
                        if width > 0 && height > 0 {
                            settings.width = width as u32;
                            settings.height = height as u32;
                        }
                    }
                }
                editor_for(&document, &path)?
            }
            // No document yet - a project created moments ago.
            Err(_) => Editor::new(),
        };
        Ok::<_, String>((path, settings, editor))
    })
    .await
    .map_err(|error| format!("open task failed: {error}"))??;

    // Everything the document lists is media the user already imported, so
    // reopening a project restores exactly the asset scope importing built -
    // no re-probe, and nothing beyond the document's own list.
    for item in &editor.project().media {
        crate::grant_asset(&app, &item.path);
    }

    let mut guard = state.0.lock().map_err(|_| "editor state poisoned".to_owned())?;
    *guard = Some(Session { path, settings, editor });
    let session = guard.as_ref().expect("just set");
    Ok(view(session, None))
}

/// Applies one edit command and returns the new state.
#[tauri::command]
pub fn editor_apply(
    state: tauri::State<'_, EditorState>,
    command: Command,
) -> Result<EditorView, String> {
    with_session(&state, |session| {
        let outcome = session.editor.apply(command).map_err(|error| error.to_string())?;
        Ok(view(session, outcome.created_id))
    })
}

#[tauri::command]
pub fn editor_undo(state: tauri::State<'_, EditorState>) -> Result<EditorView, String> {
    with_session(&state, |session| {
        session.editor.undo();
        Ok(view(session, None))
    })
}

#[tauri::command]
pub fn editor_redo(state: tauri::State<'_, EditorState>) -> Result<EditorView, String> {
    with_session(&state, |session| {
        session.editor.redo();
        Ok(view(session, None))
    })
}

/// The current state without changing anything.
#[tauri::command]
pub fn editor_state(state: tauri::State<'_, EditorState>) -> Result<EditorView, String> {
    with_session(&state, |session| Ok(view(session, None)))
}

/// Writes the session's document to its project folder. The output size can
/// have been edited in the preview footer and the name in the project
/// details dialog, so both ride along here.
#[tauri::command]
pub async fn editor_save(
    state: tauri::State<'_, EditorState>,
    name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<(), String> {
    // Settings mutation and serialisation happen under the lock; the disk
    // write - the slow, blockable part - happens off the main thread with the
    // lock released.
    let (path, document) = with_session(&state, |session| {
        if let Some(name) = name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                session.settings.name = trimmed.to_owned();
            }
        }
        // A zero dimension is never a real output size, only a caller bug -
        // saving it would poison the document until the next open.
        if let Some(width) = width.filter(|width| *width > 0) {
            session.settings.width = width;
        }
        if let Some(height) = height.filter(|height| *height > 0) {
            session.settings.height = height;
        }
        Ok((session.path.clone(), session.editor.to_document(&session.settings)))
    })?;
    tauri::async_runtime::spawn_blocking(move || projects::save(&path, &document))
        .await
        .map_err(|error| format!("save task failed: {error}"))?
}

/// Closes the session, dropping its undo history.
#[tauri::command]
pub fn editor_close(state: tauri::State<'_, EditorState>) {
    if let Ok(mut guard) = state.0.lock() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug a user hit on day one: create a project, and the editor it
    /// switches to refuses to open it. `projects::create` writes a
    /// settings-only manifest on purpose - the edit itself only appears on
    /// the first save - so the freshly written file must load as an empty
    /// edit, not as a failure.
    #[test]
    fn the_manifest_create_writes_opens_as_an_empty_edit() {
        let scratch = std::env::temp_dir().join("wolfcut-fresh-open-test");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch dir");

        let project = projects::create(&scratch.to_string_lossy(), "Test", 3840, 2160, 24, 1)
            .expect("create writes a project");
        let document = projects::read_document(&project.path).expect("and it parses");

        let editor = editor_for(&document, &project.path)
            .expect("a project created moments ago has to open");
        assert!(
            editor.project().media.is_empty(),
            "and it opens empty, because there is no edit in it yet"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The guard that must not regress in the process. A document that
    /// claims an edit and will not load is corrupt, and opening it as
    /// emptiness is how someone's project gets overwritten with nothing.
    #[test]
    fn a_document_that_claims_an_edit_and_will_not_load_is_still_an_error() {
        assert!(editor_for(&serde_json::json!({ "tracks": [] }), "p").is_err());
        assert!(editor_for(&serde_json::json!({ "timelines": [] }), "p").is_err());
    }
}
