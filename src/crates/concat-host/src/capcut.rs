// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Importing a CapCut draft as a new Concat project (#477).
//!
//! The file half of `concat_project::capcut`: where CapCut keeps its
//! drafts, what makes a folder one, reading a draft's timelines, and
//! writing the project they make into a folder of its own. The draft is
//! only ever read. A CapCut folder may be synced to CapCut's cloud and is
//! CapCut's to manage, so nothing of Concat's is written into it.

use std::path::{Path, PathBuf};

use concat_project::capcut::{DRAFT_FILES, Draft, DraftTimeline};
use serde_json::Value;

use crate::projects::{self, ProjectInfo};

/// CapCut's drafts on this machine, where it keeps them: the folder holding
/// one folder a draft. `None` where there is none.
pub fn drafts_folder() -> Option<PathBuf> {
    let under = |base: Option<PathBuf>, tail: &str| base.map(|base| base.join(tail));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let candidates = [
        // The download from capcut.com, then the App Store's, which is
        // sandboxed into a container of its own.
        under(home.clone(), "Movies/CapCut/User Data/Projects/com.lveditor.draft"),
        under(
            home,
            "Library/Containers/com.lemon.lvoverseas/Data/Movies/CapCut/User Data/Projects/com.lveditor.draft",
        ),
        under(
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
            "CapCut/User Data/Projects/com.lveditor.draft",
        ),
    ];
    candidates.into_iter().flatten().find(|folder| folder.is_dir())
}

/// Whether `folder` is a CapCut draft.
pub fn is_draft(folder: &Path) -> bool {
    DRAFT_FILES.iter().any(|file| folder.join(file).is_file())
}

/// A JSON file of the draft's, or `None` when it is missing.
fn json(path: &Path) -> Option<Result<Value, String>> {
    let bytes = std::fs::read(path).ok()?;
    Some(serde_json::from_slice(&bytes).map_err(|_| {
        // Not JSON: CapCut encrypts the drafts of some of its builds and
        // every draft made from one of its templates.
        "this CapCut draft is encrypted by CapCut, and cannot be read by anything else; \
         a draft made in an older CapCut, or not from a template, can"
            .to_owned()
    }))
}

/// The edit in `folder`: its first draft file there.
fn edit_in(folder: &Path) -> Option<Result<Value, String>> {
    DRAFT_FILES.iter().find_map(|file| json(&folder.join(file)))
}

/// Reads the draft in `folder`: its name and every timeline it has, the
/// one CapCut opens first first.
pub fn read(folder: &Path) -> Result<Draft, String> {
    let main = edit_in(folder)
        .ok_or_else(|| format!("{} is not a CapCut draft", folder.display()))??;
    let meta = json(&folder.join("draft_meta_info.json")).and_then(Result::ok);
    let name = meta
        .as_ref()
        .and_then(|meta| meta.get("draft_name"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| folder.file_name().map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "CapCut draft".to_owned());

    // A draft of several timelines lists them in `Timelines/project.json`,
    // each in a folder of its own; the draft's own file is the main one's.
    let mut timelines = Vec::new();
    let listed = json(&folder.join("Timelines").join("project.json")).and_then(Result::ok);
    if let Some(listed) = listed {
        let main_id = listed
            .get("main_timeline_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut entries: Vec<&Value> = listed
            .get("timelines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|entry| entry.get("is_marked_delete").and_then(Value::as_bool) != Some(true))
            .collect();
        entries.sort_by_key(|entry| entry.get("id").and_then(Value::as_str) != Some(main_id));
        for entry in entries {
            let id = entry.get("id").and_then(Value::as_str).unwrap_or("");
            let edit = if id == main_id {
                Some(Ok(main.clone()))
            } else {
                edit_in(&folder.join("Timelines").join(id))
            };
            if let Some(Ok(edit)) = edit {
                timelines.push(DraftTimeline {
                    name: entry
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    edit,
                });
            }
        }
    }
    if timelines.is_empty() {
        timelines.push(DraftTimeline {
            name: String::new(),
            edit: main,
        });
    }
    Ok(Draft {
        name,
        folder: folder.to_string_lossy().into_owned(),
        timelines,
    })
}

/// What an import made: the new project, and what of the draft did not
/// come across.
pub struct Import {
    /// The project, ready to open.
    pub project: ProjectInfo,
    /// One line for each kind of thing left out or changed.
    pub notes: Vec<String>,
}

/// Imports the CapCut draft in `folder` as a new project under `location`,
/// in a folder named after the draft - "Reel", then "Reel 2" if that is
/// taken - and writes its whole document there.
pub fn import(folder: &Path, location: &str) -> Result<Import, String> {
    let draft = read(folder)?;
    let imported = concat_project::capcut::import(&draft).map_err(|error| error.to_string())?;
    let settings = &imported.settings;
    let base = settings.name.trim();
    let base = if base.is_empty() { "CapCut draft" } else { base };
    let name = (1..)
        .map(|number| {
            if number == 1 {
                base.to_owned()
            } else {
                format!("{base} {number}")
            }
        })
        .find(|name| !Path::new(location).join(projects::folder_name(name)).exists())
        .expect("a free name");
    let project = projects::create(
        location,
        &name,
        settings.width,
        settings.height,
        settings.rate_num,
        settings.rate_den,
        settings.canvas,
    )?;
    let mut settings = imported.settings.clone();
    settings.name = name;
    let document = concat_project::to_document(&settings, &imported.project);
    projects::save(&project.path, &document)?;
    log::info!(
        "capcut: imported {} as {} ({} timeline(s), {} media)",
        folder.display(),
        project.path,
        imported.project.timelines.len(),
        imported.project.media.len()
    );
    for note in &imported.notes {
        log::info!("capcut: not carried: {note}");
    }
    Ok(Import {
        project,
        notes: imported.notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every draft CapCut has on this machine imports, into a scratch
    /// folder, and opens. Ignored: it needs CapCut's drafts, and reads
    /// them where CapCut keeps them. `cargo test -p concat-host --lib
    /// capcut -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs CapCut's drafts on this machine"]
    fn the_drafts_on_this_machine_import() {
        let Some(drafts) = drafts_folder() else {
            eprintln!("no CapCut drafts here");
            return;
        };
        let location = std::env::temp_dir().join(format!("concat-capcut-{}", std::process::id()));
        std::fs::create_dir_all(&location).expect("a scratch folder");
        for entry in std::fs::read_dir(&drafts).expect("the drafts").flatten() {
            let folder = entry.path();
            if !is_draft(&folder) {
                continue;
            }
            match import(&folder, &location.to_string_lossy()) {
                Ok(import) => {
                    let opened = projects::open(&import.project.path).expect("opens");
                    let document = projects::read_document(&opened.path)
                        .expect("reads")
                        .expect("a document");
                    let project =
                        concat_project::from_document(&document).expect("the document loads");
                    eprintln!(
                        "{}: {} timeline(s), {} clip(s) on the first, {} media; notes: {:?}",
                        folder.display(),
                        project.timelines.len(),
                        project.timelines[0].clips.len(),
                        project.media.len(),
                        import.notes
                    );
                }
                Err(error) => eprintln!("{}: {error}", folder.display()),
            }
        }
        let _ = std::fs::remove_dir_all(&location);
    }

    #[test]
    fn a_folder_without_a_draft_is_said_to_be_one() {
        let error = read(Path::new("/nonexistent/folder")).err().expect("refused");
        assert!(error.contains("not a CapCut draft"), "{error}");
    }
}
