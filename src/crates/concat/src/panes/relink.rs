// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The missing media dialog: what a project cannot find, and one folder
//! to look for it in.

use std::collections::HashMap;
use std::path::PathBuf;

use concat_project::Command;
use concat_project::model::MissingMedia;
use slint::VecModel;

use crate::i18n::t;
use crate::platform;
use crate::studio::Studio;
use crate::ui::{MissingMediaItem, RelinkData};

/// Everything that can happen to the relink dialog.
#[derive(Clone, Debug)]
pub enum RelinkMsg {
    /// A project opened with files it cannot find.
    Show(Vec<MissingMedia>),
    /// Pick a folder and look for every missing file inside it.
    RelinkAll,
    Dismiss,
}

/// The relink dialog's state.
#[derive(Default)]
pub struct RelinkPane {
    pub open: bool,
    pub items: Vec<MissingMedia>,
}

impl RelinkPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: RelinkMsg, studio: &mut Studio) {
        match msg {
            RelinkMsg::Show(items) => {
                self.open = true;
                self.items = items;
            }
            RelinkMsg::RelinkAll => self.relink_all(studio),
            RelinkMsg::Dismiss => self.open = false,
        }
    }

    /// Relinks missing media by searching a folder (recursively) for files
    /// whose basename matches. The user picks one folder; each missing item
    /// looks for its own filename inside it. Successful relinks go through
    /// the editor as `UpdateMediaPath`, so undo covers the whole batch.
    fn relink_all(&mut self, studio: &mut Studio) {
        let Some(folder) = platform::pick_folder(&t("relink.selectFolderContainingMedia"), "")
        else {
            return;
        };

        // Snapshot the missing list now: as relinks land the list shrinks,
        // and we want a stable target for the toast count.
        let items: Vec<(String, String)> = self
            .items
            .iter()
            .map(|m| (m.id.clone(), m.path.clone()))
            .collect();
        let total = items.len();
        if total == 0 {
            self.open = false;
            return;
        }

        // Build a basename -> full path index of every file under the folder
        // so the per-item lookup is O(1) rather than a walk each time.
        let mut index: HashMap<String, PathBuf> = HashMap::new();
        let mut stack = vec![folder.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    // First match wins: if the user has duplicates, the one
                    // closest to the root is the most likely correct copy.
                    index.entry(name.to_owned()).or_insert(path);
                }
            }
        }

        if studio.session.is_none() {
            return;
        }

        let mut relinked = 0usize;
        let mut commands: Vec<Command> = Vec::new();
        for (id, path) in items {
            let Some(basename) = std::path::Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_owned())
            else {
                continue;
            };
            if let Some(found) = index.get(&basename) {
                commands.push(Command::UpdateMediaPath {
                    media_id: id,
                    new_path: found.to_string_lossy().to_string(),
                });
                relinked += 1;
            }
        }

        if !commands.is_empty() {
            studio.apply(Command::Batch { commands });
        }

        // Re-check what is still missing: the dialog updates to the
        // remainder (often empty, in which case it closes).
        let remaining = studio.project().missing_media();
        if remaining.is_empty() {
            self.open = false;
            self.items.clear();
        } else {
            self.items = remaining;
        }

        studio.notify(
            &format!(
                "Relinked {relinked} of {total} file{}",
                if total == 1 { "" } else { "s" }
            ),
            false,
        );
        studio.request_media_art();
        studio.request_preview();
    }

    /// The dialog as Slint shows it.
    pub fn data(&self) -> RelinkData {
        RelinkData {
            open: self.open,
            items: slint::ModelRc::new(VecModel::from(
                self.items
                    .iter()
                    .map(|item| MissingMediaItem {
                        id: item.id.clone().into(),
                        name: item.name.clone().into(),
                        path: item.path.clone().into(),
                    })
                    .collect::<Vec<_>>(),
            )),
        }
    }
}
