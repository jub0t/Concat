// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The missing media dialog: what a project cannot find, and one folder
//! to look for it in.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use concat_project::Command;
use concat_project::model::{MediaKind, MissingMedia};
use slint::VecModel;

use crate::i18n::{t, tf};
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
    /// Pick one missing file by hand.
    Locate(String),
    /// The file the picker answered with for that one: it, whatever it
    /// is called now, and its folder searched for the rest.
    Located(String, PathBuf),
    /// Take one missing file out of the project, with the clips that use
    /// it, so it is never asked after again. One undo brings both back.
    Remove(String),
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
            RelinkMsg::Locate(id) => {
                let Some(item) = self.items.iter().find(|item| item.id == id) else {
                    return;
                };
                let title = tf("relink.locateFile", &[&item.name]);
                crate::host::pick_files_then(title, None, move |studio, _, _, paths| {
                    if let Some(path) = paths.into_iter().next() {
                        studio.handle(crate::panes::Msg::Relink(RelinkMsg::Located(id, path)));
                    }
                });
            }
            RelinkMsg::Located(id, path) => self.located(studio, &id, path),
            RelinkMsg::Remove(id) => self.remove(studio, &id),
            RelinkMsg::Dismiss => self.dismiss(),
        }
    }

    /// Removes one missing file from the project, and every clip of it on
    /// every timeline, as `RemoveMedia` does; the dialog closes once
    /// nothing is left on it.
    fn remove(&mut self, studio: &mut Studio, id: &str) {
        let Some(name) = self
            .items
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.name.clone())
        else {
            return;
        };
        if studio.session.is_none() {
            return;
        }
        studio.apply(Command::RemoveMedia {
            media_id: id.to_owned(),
        });
        self.items.retain(|item| item.id != id);
        self.dismissed.remove(id);
        if self.items.is_empty() {
            self.open = false;
        }
        studio.notify(&crate::i18n::tf("relink.removed", &[&name]), false);
        studio.request_media_art();
        studio.request_preview();
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
        let total = self.items.len();
        if total == 0 {
            self.open = false;
            return;
        }
        let index = index_folder(&folder, true);
        let commands = self.matches(&index, None);
        let found = commands.len();
        self.apply(studio, commands);
        if found == 0 {
            let shown = shorten(&folder.to_string_lossy());
            studio.notify(&tf("relink.notInFolder", &[&shown]), false);
        } else {
            studio.notify(&tf("relink.found", &[&found, &total]), false);
        }
    }

    /// One file picked by hand for `id`, whatever it is called now. Missing
    /// files are usually missing together - a folder moved, a drive
    /// renamed - so the folder it was found in is looked in, at its own
    /// level, for the rest, as the folder search would.
    fn located(&mut self, studio: &mut Studio, id: &str, path: PathBuf) {
        if !self.items.iter().any(|item| item.id == id) {
            return;
        }
        let total = self.items.len();
        let mut commands = vec![Command::UpdateMediaPath {
            media_id: id.to_owned(),
            new_path: path.to_string_lossy().to_string(),
        }];
        if let Some(folder) = path.parent() {
            commands.extend(self.matches(&index_folder(folder, false), Some(id)));
        }
        let found = commands.len();
        self.apply(studio, commands);
        if found > 1 {
            studio.notify(&tf("relink.found", &[&found, &total]), false);
        }
    }

    /// An `UpdateMediaPath` for every missing file but `skip` whose name
    /// is in `index`.
    fn matches(&self, index: &HashMap<String, PathBuf>, skip: Option<&str>) -> Vec<Command> {
        self.items
            .iter()
            .filter(|item| Some(item.id.as_str()) != skip)
            .filter_map(|item| {
                let name = Path::new(&item.path).file_name()?.to_str()?;
                let found = index.get(name)?;
                Some(Command::UpdateMediaPath {
                    media_id: item.id.clone(),
                    new_path: found.to_string_lossy().to_string(),
                })
            })
            .collect()
    }

    /// Applies relinks as one undo, then shows what is still missing: often
    /// nothing, and the dialog closes. A dismissal only outlives its file
    /// while that file is missing - one relinked here and lost again is
    /// news.
    fn apply(&mut self, studio: &mut Studio, commands: Vec<Command>) {
        if studio.session.is_none() {
            return;
        }
        if !commands.is_empty() {
            studio.apply(Command::Batch { commands });
        }
        let remaining = studio.project().missing_media();
        let still: HashSet<&str> = remaining.iter().map(|item| item.id.as_str()).collect();
        self.dismissed.retain(|id| still.contains(id.as_str()));
        self.open = !remaining.is_empty();
        self.items = remaining;
        studio.request_media_art();
        studio.request_preview();
    }

    /// The dialog as Slint shows it.
    pub fn data(&self, studio: &Studio) -> RelinkData {
        // How many clips each missing file is in, across every timeline:
        // what its Remove takes with it.
        let clips_of = |id: &str| -> i32 {
            studio.session.as_ref().map_or(0, |session| {
                session
                    .project()
                    .timelines
                    .iter()
                    .map(|timeline| {
                        timeline
                            .clips
                            .iter()
                            .filter(|clip| clip.media_id == id)
                            .count()
                    })
                    .sum::<usize>() as i32
            })
        };
        let kind_of = |id: &str| -> i32 {
            let kind = studio.session.as_ref().and_then(|session| {
                session
                    .project()
                    .media
                    .iter()
                    .find(|media| media.id == id)
                    .map(|media| media.kind)
            });
            match kind {
                Some(MediaKind::Audio) => 1,
                Some(MediaKind::Image) => 2,
                _ => 0,
            }
        };
        RelinkData {
            open: self.open,
            items: slint::ModelRc::new(VecModel::from(
                self.items
                    .iter()
                    .map(|item| MissingMediaItem {
                        id: item.id.clone().into(),
                        name: item.name.clone().into(),
                        folder: Path::new(&item.path)
                            .parent()
                            .map(|folder| shorten(&folder.to_string_lossy()))
                            .unwrap_or_default()
                            .into(),
                        kind: kind_of(&item.id),
                        clips: clips_of(&item.id),
                    })
                    .collect::<Vec<_>>(),
            )),
        }
    }
}

/// Every file under `folder` by name, the one nearest the top first when
/// a name is there twice: the likeliest right copy. `deep` goes into the
/// folders inside it; a folder found from one picked file is only looked
/// at itself, since it may be the home folder.
fn index_folder(folder: &Path, deep: bool) -> HashMap<String, PathBuf> {
    let mut index: HashMap<String, PathBuf> = HashMap::new();
    let mut queue = std::collections::VecDeque::from([folder.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                if deep {
                    queue.push_back(path);
                }
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                index.entry(name.to_owned()).or_insert(path);
            }
        }
    }
    index
}

/// A path as a row shows it: the home folder as ~.
fn shorten(path: &str) -> String {
    let Some(home) = std::env::home_dir() else {
        return path.to_owned();
    };
    let home = home.to_string_lossy();
    match path.strip_prefix(home.as_ref()) {
        Some(rest) if rest.is_empty() || rest.starts_with(std::path::MAIN_SEPARATOR) => {
            format!("~{rest}")
        }
        _ => path.to_owned(),
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
