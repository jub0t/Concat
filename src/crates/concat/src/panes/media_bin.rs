// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The media bin: the project's files as cards, filtered, sorted and
//! selected.
//!
//! The bin owns what is the window's about the media and not the
//! document's: the integer row Slint knows each item by, the selection,
//! the filter and the sort, and the card artwork. The peaks and
//! filmstrips are not here: the lanes draw them too, so they stay on the
//! controller's art store. An import probes on a worker and comes back
//! as [`MediaMsg::Imported`], one batch of `AddMedia` commands.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use concat_host::media::{self, MediaSummary};
use concat_project::Command;
use concat_project::model::{self, MediaItem, MediaOrigin, Project};

use crate::format::wave_path;
use crate::host::{probe_error, spawn_in_project};
use crate::i18n::{t, tf};
use crate::panes::Msg;
use crate::studio::Studio;
use crate::ui::{MediaFilter, MediaItemData, MediaKind};

/// Everything that can happen to the bin.
#[derive(Clone, Debug)]
pub enum MediaMsg {
    FilterChanged(MediaFilter),
    /// 0 added, 1 name, 2 kind.
    SortChanged(i32),
    /// A card clicked, by its row; additive keeps what was selected.
    Select {
        row: i32,
        additive: bool,
    },
    /// A marquee closed over the grid: the block of cells it caught.
    Band {
        columns: i32,
        from_col: i32,
        to_col: i32,
        from_row: i32,
        to_row: i32,
        additive: bool,
    },
    Remove(i32),
    RemoveSelected,
    /// Files picked, dropped or handed over by the platform.
    Import(Vec<PathBuf>),
    /// The import's worker is done probing.
    Imported(Vec<Result<MediaSummary, String>>),
    /// Everything selected goes on the timeline at the playhead.
    AddSelectedAtPlayhead,
}

/// The bin's state.
pub struct MediaBin {
    /// Slint's rows are integers; the document's ids are strings. Assigned
    /// once per id and never reused, so a payload in flight names the row it
    /// was dragged from.
    rows: HashMap<String, i32>,
    next_row: i32,
    pub selected: HashSet<String>,
    pub filter: MediaFilter,
    /// 0 = Added, 1 = Name, 2 = Kind
    pub sort: usize,
    /// The cards' pictures by media id.
    pub thumbs: HashMap<String, slint::Image>,
    /// A card's waveform path by media id, with the peaks it was drawn
    /// from and the duration it spans: the rows are rebuilt on every
    /// publish, thirty times a second in playback, and the path is the
    /// one expensive line of a row.
    waves: std::cell::RefCell<HashMap<String, (usize, f32, slint::SharedString)>>,
}

impl Default for MediaBin {
    fn default() -> Self {
        Self {
            rows: HashMap::new(),
            next_row: 1,
            selected: HashSet::new(),
            filter: MediaFilter::All,
            sort: 0,
            thumbs: HashMap::new(),
            waves: Default::default(),
        }
    }
}

impl MediaBin {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: MediaMsg, studio: &mut Studio) {
        match msg {
            MediaMsg::FilterChanged(filter) => self.filter = filter,
            MediaMsg::SortChanged(index) => self.sort = (index.max(0) as usize).min(2),
            MediaMsg::Select { row, additive } => {
                let Some(id) = self
                    .by_row(studio.project(), row)
                    .map(|item| item.id.clone())
                else {
                    return;
                };
                self.select(id, additive);
            }
            MediaMsg::Band {
                columns,
                from_col,
                to_col,
                from_row,
                to_row,
                additive,
            } => {
                self.selected = self.band(
                    studio.project(),
                    columns,
                    (from_col, to_col),
                    (from_row, to_row),
                    additive,
                );
            }
            MediaMsg::Remove(row) => {
                if let Some(id) = self
                    .by_row(studio.project(), row)
                    .map(|item| item.id.clone())
                {
                    self.selected.remove(&id);
                    studio.apply(Command::RemoveMedia { media_id: id });
                }
            }
            MediaMsg::RemoveSelected => {
                let doomed: Vec<String> = self.selected.drain().collect();
                if doomed.is_empty() {
                    return;
                }
                studio.apply(Command::Batch {
                    commands: doomed
                        .into_iter()
                        .map(|media_id| Command::RemoveMedia { media_id })
                        .collect(),
                });
            }
            MediaMsg::Import(paths) => {
                if paths.is_empty() || studio.session.is_none() {
                    return;
                }
                spawn_in_project(
                    move || {
                        paths
                            .iter()
                            .map(|path| media::probe(&path.to_string_lossy()))
                            .collect::<Vec<_>>()
                    },
                    |studio, _, _, results| studio.handle(Msg::Media(MediaMsg::Imported(results))),
                );
            }
            MediaMsg::Imported(results) => {
                let mut commands = Vec::new();
                let mut failures = Vec::new();
                for result in results {
                    match result {
                        Ok(summary) => commands.push(Command::AddMedia {
                            item: summary.to_new_media(),
                        }),
                        Err(error) => failures.push(error),
                    }
                }
                let added = commands.len();
                if !commands.is_empty() {
                    studio.apply(Command::Batch { commands });
                }
                if let Some(error) = failures.first() {
                    studio.notify(&probe_error(error), true);
                } else if added > 0 {
                    studio.notify(
                        &if added == 1 {
                            t("Imported 1 file")
                        } else {
                            tf("Imported {0} files", &[&added])
                        },
                        false,
                    );
                }
            }
            MediaMsg::AddSelectedAtPlayhead => {
                let ids: Vec<String> = studio
                    .project()
                    .media
                    .iter()
                    .filter(|item| self.selected.contains(&item.id))
                    .map(|item| item.id.clone())
                    .collect();
                let start = f64::from(studio.playhead.max(0.0));
                for media_id in ids {
                    studio.apply(Command::AddClipAtFirstFree { media_id, start });
                }
            }
        }
    }

    /// Gives every media item the integer row Slint knows it by. New items
    /// get the next number; nothing is ever renumbered.
    pub fn assign_rows(&mut self, project: &Project) {
        for item in &project.media {
            if !self.rows.contains_key(&item.id) {
                self.rows.insert(item.id.clone(), self.next_row);
                self.next_row += 1;
            }
        }
    }

    /// The item Slint's row names, if it is still in the project.
    pub fn by_row<'a>(&self, project: &'a Project, row: i32) -> Option<&'a MediaItem> {
        let id = self.rows.iter().find(|(_, held)| **held == row)?.0;
        project.media_by_id(id)
    }

    /// The row an id is shown at, for a message that names rows.
    pub fn row_of(&self, id: &str) -> Option<i32> {
        self.rows.get(id).copied()
    }

    /// Whether the filter lets an item through. The Media shelves, "All
    /// media" included, are the imports: a file the editor made is on its
    /// origin's shelf under Generated and nowhere else, so a read-aloud
    /// voice is not also the fifth thing under Audio.
    fn shows(filter: MediaFilter, item: &MediaItem) -> bool {
        let imported = item.origin.is_none();
        match filter {
            MediaFilter::All => imported,
            MediaFilter::Video => imported && item.kind == model::MediaKind::Video,
            MediaFilter::Audio => imported && item.kind == model::MediaKind::Audio,
            MediaFilter::Images => imported && item.kind == model::MediaKind::Image,
            MediaFilter::Speech => item.origin == Some(MediaOrigin::Speech),
            MediaFilter::Processed => item.origin == Some(MediaOrigin::Processed),
        }
    }

    /// A click: the one card, or toggled into what is held.
    fn select(&mut self, id: String, additive: bool) {
        if additive {
            if !self.selected.remove(&id) {
                self.selected.insert(id);
            }
        } else {
            self.selected.clear();
            self.selected.insert(id);
        }
    }

    /// A marquee closed over the grid, as the block of cells it caught. The
    /// walk is over the filtered order, because that is what the grid was
    /// laid out from.
    fn band(
        &self,
        project: &Project,
        columns: i32,
        (from_col, to_col): (i32, i32),
        (from_row, to_row): (i32, i32),
        additive: bool,
    ) -> HashSet<String> {
        let mut cell = 0;
        let mut next = if additive {
            self.selected.clone()
        } else {
            HashSet::new()
        };
        for item in &project.media {
            if !Self::shows(self.filter, item) {
                continue;
            }
            let (row, col) = (cell / columns.max(1), cell % columns.max(1));
            if row >= from_row && row <= to_row && col >= from_col && col <= to_col {
                next.insert(item.id.clone());
            }
            cell += 1;
        }
        next
    }

    /// The items the bin shows, filtered and in the chosen order.
    fn visible<'a>(&self, project: &'a Project) -> Vec<&'a MediaItem> {
        let mut visible: Vec<&MediaItem> = project
            .media
            .iter()
            .filter(|item| Self::shows(self.filter, item))
            .collect();
        match self.sort {
            // Added: the import order, as the document keeps it.
            0 => {}
            1 => visible.sort_by_key(|item| item.name.to_lowercase()),
            // Video, then audio, then stills; each by name.
            2 => visible.sort_by(|a, b| {
                let rank = |kind: model::MediaKind| match kind {
                    model::MediaKind::Video => 0,
                    model::MediaKind::Audio => 1,
                    model::MediaKind::Image => 2,
                };
                rank(a.kind)
                    .cmp(&rank(b.kind))
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }),
            _ => {}
        }
        visible
    }

    /// The cards, as Slint shows them.
    pub fn rows(&self, studio: &Studio) -> Vec<MediaItemData> {
        self.visible(studio.project())
            .into_iter()
            .map(|item| {
                // A card is a couple of hundred pixels: sixty-four bars,
                // each three quarters of its pitch, is what reads best at
                // that size - fuller than a lane's candles, on purpose.
                let wave = match studio.peaks.get(&item.id) {
                    Some(peaks) if item.kind == model::MediaKind::Audio => {
                        let duration = item.duration.unwrap_or(0.0) as f32;
                        let drawn_from = std::sync::Arc::as_ptr(peaks) as usize;
                        let mut waves = self.waves.borrow_mut();
                        match waves.get(&item.id) {
                            Some((from, span, path))
                                if *from == drawn_from && *span == duration =>
                            {
                                path.clone()
                            }
                            _ => {
                                let path: slint::SharedString =
                                    wave_path(peaks, 0.0, duration, 64, 0.75).as_str().into();
                                waves.insert(item.id.clone(), (drawn_from, duration, path.clone()));
                                path
                            }
                        }
                    }
                    _ => slint::SharedString::default(),
                };
                MediaItemData {
                    id: *self.rows.get(&item.id).unwrap_or(&0),
                    name: item.name.as_str().into(),
                    kind: media_kind_of(item.kind),
                    duration: item.duration.unwrap_or(0.0) as f32,
                    format: std::path::Path::new(&item.path)
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .map(|extension| extension.to_ascii_lowercase())
                        .unwrap_or_default()
                        .into(),
                    thumbnail: self.thumbs.get(&item.id).cloned().unwrap_or_default(),
                    wave,
                    selected: self.selected.contains(&item.id),
                }
            })
            .collect()
    }
}

/// The document's kind as the card's.
fn media_kind_of(kind: model::MediaKind) -> MediaKind {
    match kind {
        model::MediaKind::Video => MediaKind::Video,
        model::MediaKind::Audio => MediaKind::Audio,
        model::MediaKind::Image => MediaKind::Image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::model::{MediaKind as Kind, Project};

    fn project() -> Project {
        let mut project = Project::default();
        for (id, name, kind) in [
            ("v1", "Zebra.mp4", Kind::Video),
            ("a1", "alpha.wav", Kind::Audio),
            ("i1", "Mid.png", Kind::Image),
            ("v2", "apple.mov", Kind::Video),
        ] {
            project.media.push(MediaItem {
                id: id.to_owned(),
                name: name.to_owned(),
                kind,
                ..MediaItem::default()
            });
        }
        project
    }

    #[test]
    fn rows_are_assigned_once_and_never_reused() {
        let mut bin = MediaBin::default();
        let mut project = project();
        bin.assign_rows(&project);
        let zebra = bin.by_row(&project, 1).expect("row 1").id.clone();
        assert_eq!(zebra, "v1");
        // The first item goes; its row goes with it and is never given out
        // again, so a payload in flight cannot name the wrong file.
        project.media.remove(0);
        bin.assign_rows(&project);
        assert!(bin.by_row(&project, 1).is_none());
        project.media.push(MediaItem {
            id: "v3".to_owned(),
            ..MediaItem::default()
        });
        bin.assign_rows(&project);
        assert_eq!(bin.by_row(&project, 5).expect("a new row").id, "v3");
        assert!(bin.by_row(&project, 0).is_none(), "zero names nothing");
        assert!(bin.by_row(&project, -7).is_none());
    }

    #[test]
    fn a_filter_and_a_sort_shape_the_visible_list() {
        let mut bin = MediaBin::default();
        let project = project();
        bin.filter = MediaFilter::Video;
        bin.sort = 1;
        let names: Vec<&str> = bin
            .visible(&project)
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(names, ["apple.mov", "Zebra.mp4"], "by name, case aside");
        bin.filter = MediaFilter::All;
        bin.sort = 2;
        let names: Vec<&str> = bin
            .visible(&project)
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        assert_eq!(names, ["apple.mov", "Zebra.mp4", "alpha.wav", "Mid.png"]);
        bin.sort = 99;
        assert_eq!(
            bin.visible(&project).len(),
            4,
            "an unknown sort keeps the order"
        );
    }

    #[test]
    fn a_generated_voice_is_on_its_own_shelf_and_no_other() {
        let mut bin = MediaBin::default();
        let mut project = project();
        project.media.push(MediaItem {
            id: "s1".to_owned(),
            name: "Voice 1.wav".to_owned(),
            kind: Kind::Audio,
            origin: Some(MediaOrigin::Speech),
            ..MediaItem::default()
        });
        let names = |bin: &MediaBin| -> Vec<String> {
            bin.visible(&project)
                .iter()
                .map(|item| item.name.clone())
                .collect()
        };
        // Not under All media, not under Audio, though it is audio.
        bin.filter = MediaFilter::All;
        assert_eq!(names(&bin).len(), 4, "the imports, and only them");
        bin.filter = MediaFilter::Audio;
        assert_eq!(names(&bin), ["alpha.wav"]);
        bin.filter = MediaFilter::Speech;
        assert_eq!(names(&bin), ["Voice 1.wav"]);
        // A marquee on the Speech shelf walks the Speech shelf.
        let caught = bin.band(&project, 3, (0, 2), (0, 0), false);
        assert_eq!(caught.len(), 1);
        assert!(caught.contains("s1"));
        // Rows are minted for it like any other, so a drag can name it.
        bin.assign_rows(&project);
        assert_eq!(bin.by_row(&project, 5).expect("row 5").id, "s1");
    }

    #[test]
    fn a_click_toggles_only_when_additive() {
        let mut bin = MediaBin::default();
        bin.select("a".into(), false);
        bin.select("b".into(), false);
        assert_eq!(bin.selected.len(), 1);
        assert!(bin.selected.contains("b"));
        bin.select("a".into(), true);
        assert_eq!(bin.selected.len(), 2);
        bin.select("a".into(), true);
        assert!(!bin.selected.contains("a"));
    }

    #[test]
    fn a_band_walks_the_filtered_grid_and_survives_zero_columns() {
        let mut bin = MediaBin::default();
        let project = project();
        // Two columns over all four: rows are (v1 a1) (i1 v2).
        let caught = bin.band(&project, 2, (1, 1), (0, 1), false);
        assert_eq!(caught.len(), 2);
        assert!(caught.contains("a1") && caught.contains("v2"));
        // Zero or negative columns is one column, not a division by zero.
        let caught = bin.band(&project, 0, (0, 0), (0, 3), false);
        assert_eq!(caught.len(), 4);
        let caught = bin.band(&project, -3, (0, 0), (5, 9), false);
        assert!(caught.is_empty(), "rows past the end catch nothing");
        // Additive keeps what was there; a filter changes the walk.
        bin.selected.insert("x".into());
        bin.filter = MediaFilter::Images;
        let caught = bin.band(&project, 1, (0, 0), (0, 0), true);
        assert_eq!(caught.len(), 2);
        assert!(caught.contains("i1") && caught.contains("x"));
    }
}
