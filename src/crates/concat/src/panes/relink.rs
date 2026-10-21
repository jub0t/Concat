// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The missing media dialog: what a project cannot find, and one folder
//! to look for it in.

use std::collections::{HashMap, HashSet};
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
    /// The window came back to the front: what the project cannot find
    /// now, which may have changed while it was away.
    Recheck(Vec<MissingMedia>),
    /// Pick a folder and look for every missing file inside it.
    RelinkAll,
    /// The folder the picker answered with: look inside it.
    RelinkIn(PathBuf),
    Dismiss,
}

/// The relink dialog's state.
#[derive(Default)]
pub struct RelinkPane {
    pub open: bool,
    pub items: Vec<MissingMedia>,
    /// The media whose dialog the user closed: no dialog mentions them
    /// again until they come back and go missing anew.
    dismissed: HashSet<String>,
}

impl RelinkPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: RelinkMsg, studio: &mut Studio) {
        match msg {
            RelinkMsg::Show(items) => self.show(items),
            // A return to the front: forget media that is back (losing it
            // again will be news), keep the silence the user asked for on
            // the rest, and open on whatever missing is new to them.
            RelinkMsg::Recheck(missing) => self.recheck(missing),
            // The picker runs with the studio unborrowed and answers as a
            // message of its own; see `host::after_dialog`.
            RelinkMsg::RelinkAll => {
                let title = t("relink.selectFolderContainingMedia");
                crate::host::after_dialog(
                    move || platform::pick_folder(&title, ""),
                    |studio, _, _, folder| {
                        studio.handle(crate::panes::Msg::Relink(RelinkMsg::RelinkIn(folder)));
                    },
                );
            }
            RelinkMsg::RelinkIn(folder) => self.relink_in(studio, folder),
            RelinkMsg::Dismiss => self.dismiss(),
        }
    }

    /// Opens the dialog on a project's missing files, as opening a project
    /// does. A dismissal is about the project it was made in, so one from
    /// an earlier project is forgotten here.
    fn show(&mut self, items: Vec<MissingMedia>) {
        self.open = true;
        self.items = items;
        self.dismissed.clear();
    }

    /// Applies a check made while the project is open - on a return to the
    /// front - to the dialog: media that is back is forgotten, dismissed
    /// media stays quiet, and the rest is the dialog's list.
    fn recheck(&mut self, missing: Vec<MissingMedia>) {
        let still: HashSet<&str> = missing.iter().map(|item| item.id.as_str()).collect();
        self.dismissed.retain(|id| still.contains(id.as_str()));
        self.items = missing
            .into_iter()
            .filter(|item| !self.dismissed.contains(&item.id))
            .collect();
        self.open = !self.items.is_empty();
    }

    /// Closes the dialog, and keeps it closed for what it was listing
    /// until those files are back and go missing again.
    fn dismiss(&mut self) {
        self.dismissed
            .extend(self.items.iter().map(|item| item.id.clone()));
        self.open = false;
    }

    /// Relinks missing media by searching `folder` (recursively) for files
    /// whose basename matches: each missing item looks for its own filename
    /// inside it. Successful relinks go through the editor as
    /// `UpdateMediaPath`, so undo covers the whole batch.
    fn relink_in(&mut self, studio: &mut Studio, folder: PathBuf) {
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
        // remainder (often empty, in which case it closes). A dismissal
        // only outlives its file while that file is missing - one that is
        // relinked here and lost again is news.
        let remaining = studio.project().missing_media();
        let still: HashSet<&str> = remaining.iter().map(|item| item.id.as_str()).collect();
        self.dismissed.retain(|id| still.contains(id.as_str()));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(id: &str, path: &str) -> MissingMedia {
        MissingMedia {
            id: id.to_owned(),
            name: format!("{id}.mp4"),
            path: path.to_owned(),
        }
    }

    #[test]
    fn a_recheck_opens_the_dialog_on_what_is_missing() {
        let mut pane = RelinkPane::default();
        pane.recheck(vec![missing("m1", "/gone/a.mp4")]);
        assert!(pane.open);
        assert_eq!(pane.items.len(), 1);
        assert_eq!(pane.items[0].id, "m1");
    }

    #[test]
    fn a_dismissed_file_stays_quiet_while_it_is_missing() {
        let mut pane = RelinkPane::default();
        pane.recheck(vec![missing("m1", "/gone/a.mp4")]);
        pane.dismiss();
        assert!(!pane.open, "dismissing closes the dialog");

        pane.recheck(vec![missing("m1", "/gone/a.mp4")]);
        assert!(!pane.open, "and the same missing file does not reopen it");
        assert!(pane.items.is_empty());
    }

    #[test]
    fn a_file_that_comes_back_and_goes_missing_again_is_news() {
        let mut pane = RelinkPane::default();
        pane.recheck(vec![missing("m1", "/gone/a.mp4")]);
        pane.dismiss();
        // Back on disk: the check finds nothing and forgets the dismissal.
        pane.recheck(Vec::new());
        assert!(!pane.open);
        // Gone a second time: the dialog should be back.
        pane.recheck(vec![missing("m1", "/gone/a.mp4")]);
        assert!(pane.open);
        assert_eq!(pane.items.len(), 1);
    }

    #[test]
    fn a_new_missing_file_opens_the_dialog_beside_a_dismissed_one() {
        let mut pane = RelinkPane::default();
        pane.recheck(vec![missing("m1", "/gone/a.mp4")]);
        pane.dismiss();
        pane.recheck(vec![
            missing("m1", "/gone/a.mp4"),
            missing("m2", "/gone/b.mp4"),
        ]);
        assert!(pane.open);
        assert_eq!(pane.items.len(), 1);
        assert_eq!(pane.items[0].id, "m2", "only the news is listed");
    }

    #[test]
    fn a_recheck_that_finds_nothing_closes_the_dialog() {
        let mut pane = RelinkPane::default();
        pane.recheck(vec![missing("m1", "/gone/a.mp4")]);
        pane.recheck(Vec::new());
        assert!(!pane.open);
        assert!(pane.items.is_empty());
    }

    #[test]
    fn opening_a_project_forgets_an_earlier_projects_dismissals() {
        let mut pane = RelinkPane::default();
        pane.recheck(vec![missing("m1", "/gone/a.mp4")]);
        pane.dismiss();
        // Another project opens with the same id missing: Show is the
        // project-open path, and it starts the dialog afresh.
        pane.show(vec![missing("m1", "/gone/a.mp4")]);
        assert!(pane.open);
        assert_eq!(pane.items.len(), 1);
    }
}
