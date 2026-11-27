// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

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

use concat_doc::commands::Command;
use concat_doc::model::{self, Media, MediaId, MediaOrigin, Model};
use concat_host::media::{self, MediaSummary};

use crate::doc_view::{self, Kind};

use crate::format::{WAVE_FLOOR, wave_path};
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
    /// A card clicked, by its row. `toggle` (Ctrl/Cmd) adds or removes
    /// one card; `range` (Shift) selects from the anchor to this card in
    /// the visible order (#696).
    Select {
        row: i32,
        toggle: bool,
        range: bool,
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
    /// The bin took a press: Ctrl+A should select its cards (#696).
    Focused,
    /// Every visible card, for Ctrl+A while the bin has focus (#696).
    SelectAll,
    Remove(i32),
    RemoveSelected,
    /// Files picked, dropped or handed over by the platform.
    Import(Vec<PathBuf>),
    /// The import's worker is done probing.
    Imported(Vec<Result<MediaSummary, String>>),
    /// Everything selected goes on the timeline at the playhead.
    AddSelectedAtPlayhead,
    /// The phone's Add: what was picked, in the order it was picked, onto
    /// the main lane - at the playhead, or after the last picture on it
    /// when `at_end` (the lanes' plus).
    AddPicked {
        at_end: bool,
    },
    /// The phone's Media sheet opened: a pick starts from nothing.
    ClearPicks,
}

/// One card of the bin: a file of the document, and its id as the panels
/// hold it. Reads as the file it is.
#[derive(Clone)]
pub struct Card<'a> {
    pub id: String,
    pub media: MediaId,
    pub item: &'a Media,
}

impl std::ops::Deref for Card<'_> {
    type Target = Media;
    fn deref(&self) -> &Media {
        self.item
    }
}

/// The bin's state.
pub struct MediaBin {
    /// Slint's rows are integers; the document's ids are strings. Assigned
    /// once per id and never reused, so a payload in flight names the row it
    /// was dragged from.
    rows: HashMap<String, i32>,
    next_row: i32,
    pub selected: HashSet<String>,
    /// The selection in the order it was made, oldest first: the order a
    /// phone's picks are numbered and laid down in, as every phone
    /// editor's picker numbers them. May name what has since been let go
    /// of; [`MediaBin::picked_in_order`] reads it against `selected`.
    picked: Vec<String>,
    /// The last plain or toggled click: Shift+click selects from here
    /// through the card in the visible order (#696).
    anchor: Option<String>,
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
            picked: Vec::new(),
            anchor: None,
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
            MediaMsg::Select { row, toggle, range } => {
                let Some(id) = self
                    .by_row(studio.committed(), row)
                    .map(|item| item.id.clone())
                else {
                    return;
                };
                studio.media_focused = true;
                self.select(studio.committed(), id.clone(), toggle, range);
                // A plain click shows a picture on the monitor, or plays a
                // sound, without putting it on the timeline.
                if !toggle && !range {
                    studio.preview_media(&id, row);
                }
            }
            MediaMsg::Band {
                columns,
                from_col,
                to_col,
                from_row,
                to_row,
                additive,
            } => {
                studio.media_focused = true;
                self.selected = self.band(
                    studio.committed(),
                    columns,
                    (from_col, to_col),
                    (from_row, to_row),
                    additive,
                );
                let selected = &self.selected;
                self.picked.retain(|id| selected.contains(id));
                for id in selected {
                    if !self.picked.iter().any(|held| held == id) {
                        self.picked.push(id.clone());
                    }
                }
            }
            MediaMsg::Focused => studio.media_focused = true,
            MediaMsg::SelectAll => {
                studio.media_focused = true;
                let ids: Vec<String> = self
                    .visible(studio.committed())
                    .into_iter()
                    .map(|item| item.id.clone())
                    .collect();
                self.selected = ids.iter().cloned().collect();
                self.picked = ids;
            }
            MediaMsg::Remove(row) => {
                if let Some((id, media)) = self
                    .by_row(studio.committed(), row)
                    .map(|item| (item.id.clone(), item.media))
                {
                    self.selected.remove(&id);
                    studio.apply(Command::RemoveMedia { media });
                }
            }
            MediaMsg::RemoveSelected => {
                let doomed: Vec<String> = self.selected.drain().collect();
                if doomed.is_empty() {
                    return;
                }
                studio.apply(Command::Batch {
                    commands: doomed
                        .iter()
                        .filter_map(|id| doc_view::parse::<MediaId>(id))
                        .map(|media| Command::RemoveMedia { media })
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
                            media: summary.to_new_media(),
                        }),
                        Err(error) => failures.push(error),
                    }
                }
                let added = commands.len();
                if !commands.is_empty() {
                    let before: HashSet<MediaId> = studio.committed().media.ids().collect();
                    studio.apply(Command::Batch { commands });
                    // On a phone the files just brought in come back picked,
                    // in the order the system's picker gave them, so Add is
                    // the one tap left between the picker and the timeline.
                    if crate::platform::phone() {
                        let fresh: Vec<String> = studio
                            .committed()
                            .media
                            .ids()
                            .filter(|id| !before.contains(id))
                            .map(|id| id.to_string())
                            .collect();
                        if !fresh.is_empty() {
                            self.selected = fresh.iter().cloned().collect();
                            self.picked = fresh;
                        }
                    }
                }
                if let Some(error) = failures.first() {
                    studio.notify(&probe_error(error), true);
                } else if added > 0 {
                    studio.notify(
                        &if added == 1 {
                            t("mediaBin.importedOneFile")
                        } else {
                            tf("mediaBin.importedFiles", &[&added])
                        },
                        false,
                    );
                }
            }
            MediaMsg::AddSelectedAtPlayhead => {
                // Onto the timeline: the monitor goes back to it.
                studio.close_source();
                let media: Vec<MediaId> = cards(studio.committed())
                    .filter(|item| self.selected.contains(&item.id))
                    .map(|item| item.media)
                    .collect();
                // One after another on one lane, as one step - not each at
                // the playhead on a lane of its own, which stacked them.
                let start = doc_view::tick(f64::from(studio.playhead.max(0.0)));
                studio.apply(Command::AddClipsInSequence {
                    media,
                    lane: None,
                    start,
                    ripple: false,
                });
            }
            MediaMsg::AddPicked { at_end } => self.add_picked(studio, at_end),
            MediaMsg::ClearPicks => {
                self.selected.clear();
                self.picked.clear();
            }
        }
    }

    /// The selection in the order it was picked: what was picked one by
    /// one first, then anything selected another way, in the bin's order.
    pub fn picked_in_order(&self, doc: &Model) -> Vec<String> {
        let mut ids: Vec<String> = Vec::new();
        for id in &self.picked {
            if self.selected.contains(id) && !ids.contains(id) && card(doc, id).is_some() {
                ids.push(id.clone());
            }
        }
        for item in cards(doc) {
            if self.selected.contains(&item.id) && !ids.contains(&item.id) {
                ids.push(item.id);
            }
        }
        ids
    }

    /// The phone's Add. Pictures go onto the main lane - the lowest one
    /// holding a picture, the main track - inserted where the playhead
    /// is (at the nearer edge of the clip under it, the rest of the lane
    /// moved along to make room) or after the lane's last clip, never past
    /// it into a gap; a timeline with no picture yet starts one. Anything
    /// with a sound file in it keeps the desk's rule, on the lowest lane
    /// free at the playhead for the whole run, since a song inserted into
    /// the picture would push the cut apart. The playhead goes to the last
    /// frame of what landed, so the next Add follows on, and the picks are
    /// let go of.
    fn add_picked(&mut self, studio: &mut Studio, at_end: bool) {
        let doc = studio.committed();
        let ids = self.picked_in_order(doc);
        if ids.is_empty() {
            return;
        }
        let media: Vec<MediaId> = ids.iter().filter_map(|id| doc_view::parse(id)).collect();
        let sound = media.iter().any(|id| {
            doc_view::media(doc, *id).is_some_and(|item| item.kind == model::MediaKind::Audio)
        });
        let playhead = doc_view::tick(f64::from(studio.playhead.max(0.0)));
        let lane = if sound { None } else { picture_lane(doc) };
        let (lane, start, ripple) = match lane {
            Some(lane) => {
                let end = doc_view::on_lane(doc, lane)
                    .map(|(_, clip)| doc_view::end(clip))
                    .max()
                    .unwrap_or(concat_doc::Tick::ZERO);
                // Under a clip, the nearer of its two edges: a playhead a
                // second into a long take means before it, not after it.
                let start = if at_end {
                    end
                } else {
                    doc_view::on_lane(doc, lane)
                        .find(|(_, clip)| clip.start < playhead && playhead < doc_view::end(clip))
                        .map_or(playhead.min(end), |(_, clip)| {
                            if (playhead - clip.start).get() < clip.len.get() / 2 {
                                clip.start
                            } else {
                                doc_view::end(clip)
                            }
                        })
                };
                (Some(lane), start, true)
            }
            None if at_end && !sound => (None, doc_view::duration(doc), false),
            None => (None, playhead, false),
        };
        let before: HashSet<model::ItemId> = doc.items.ids().collect();
        // Onto the timeline: the monitor goes back to it.
        studio.close_source();
        studio.apply(Command::AddClipsInSequence {
            media,
            lane,
            start,
            ripple,
        });
        let landed = doc_view::items(studio.committed())
            .into_iter()
            .filter(|(id, _)| !before.contains(id))
            .map(|(_, clip)| doc_view::end_s(clip))
            .fold(None, |end: Option<f64>, at| {
                Some(end.map_or(at, |end| end.max(at)))
            });
        self.selected.clear();
        self.picked.clear();
        // A frame short of the end, so the monitor shows the last of what
        // landed rather than the black past it, and the next Add - nearer
        // that clip's end than its start - follows on after it.
        if let Some(end) = landed {
            let frame = 1.0 / f64::from(studio.frame_rate().max(1.0));
            studio.seek((end - frame).max(0.0) as f32);
        }
    }

    /// Gives every media item the integer row Slint knows it by. New items
    /// get the next number; nothing is ever renumbered.
    pub fn assign_rows(&mut self, doc: &Model) {
        for id in doc.media.ids() {
            let id = id.to_string();
            if !self.rows.contains_key(&id) {
                self.rows.insert(id, self.next_row);
                self.next_row += 1;
            }
        }
    }

    /// The item Slint's row names, if it is still in the project.
    pub fn by_row<'a>(&self, doc: &'a Model, row: i32) -> Option<Card<'a>> {
        let id = self.rows.iter().find(|(_, held)| **held == row)?.0;
        card(doc, id)
    }

    /// The row an id is shown at, for a message that names rows.
    pub fn row_of(&self, id: &str) -> Option<i32> {
        self.rows.get(id).copied()
    }

    /// Whether the filter lets an item through. The Media shelves, "All
    /// media" included, are the imports: a file the editor made is on its
    /// origin's shelf under Generated and nowhere else, so a read-aloud
    /// voice is not also the fifth thing under Audio.
    fn shows(filter: MediaFilter, item: &Media) -> bool {
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

    /// A click: one card, a Ctrl/Cmd toggle, or a Shift range from the
    /// anchor through this card in the visible order (#696).
    fn select(&mut self, doc: &Model, id: String, toggle: bool, range: bool) {
        if range {
            let visible = self.visible(doc);
            let anchor = self
                .anchor
                .as_ref()
                .and_then(|anchor| visible.iter().position(|item| item.id == *anchor));
            let here = visible.iter().position(|item| item.id == id);
            if let (Some(a), Some(b)) = (anchor, here) {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                let slice = &visible[lo..=hi];
                self.selected = slice.iter().map(|item| item.id.clone()).collect();
                self.picked = slice.iter().map(|item| item.id.clone()).collect();
                return;
            }
            // No usable anchor: fall through as a plain click.
        }
        if toggle {
            if self.selected.remove(&id) {
                self.picked.retain(|held| *held != id);
            } else {
                self.selected.insert(id.clone());
                self.picked.retain(|held| *held != id);
                self.picked.push(id.clone());
            }
            self.anchor = Some(id);
        } else {
            self.selected.clear();
            self.selected.insert(id.clone());
            self.picked = vec![id.clone()];
            self.anchor = Some(id);
        }
    }

    /// A marquee closed over the grid, as the block of cells it caught. The
    /// walk is over the visible order, because that is what the grid was
    /// laid out from (#696).
    fn band(
        &self,
        doc: &Model,
        columns: i32,
        (from_col, to_col): (i32, i32),
        (from_row, to_row): (i32, i32),
        additive: bool,
    ) -> HashSet<String> {
        let mut next = if additive {
            self.selected.clone()
        } else {
            HashSet::new()
        };
        let columns = columns.max(1);
        for (cell, item) in (0..).zip(self.visible(doc)) {
            let (row, col) = (cell / columns, cell % columns);
            if row >= from_row && row <= to_row && col >= from_col && col <= to_col {
                next.insert(item.id.clone());
            }
        }
        next
    }

    /// The items the bin shows, filtered and in the chosen order.
    fn visible<'a>(&self, doc: &'a Model) -> Vec<Card<'a>> {
        let mut visible: Vec<Card<'a>> = cards(doc)
            .filter(|item| Self::shows(self.filter, item))
            .collect();
        match self.sort {
            // Added: the import order, as the document keeps it.
            0 => {}
            1 => visible.sort_by_key(|item| item.name.to_lowercase()),
            // Video, then audio, then stills; each by name.
            2 => visible.sort_by(|a, b| {
                let rank = |kind: &model::MediaKind| match kind {
                    model::MediaKind::Video => 0,
                    model::MediaKind::Audio => 1,
                    model::MediaKind::Image => 2,
                    model::MediaKind::Unknown(_) => 3,
                };
                rank(&a.kind)
                    .cmp(&rank(&b.kind))
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }),
            _ => {}
        }
        visible
    }

    /// The cards, as Slint shows them.
    pub fn rows(&self, studio: &Studio) -> Vec<MediaItemData> {
        let order = self.picked_in_order(studio.committed());
        self.visible(studio.committed())
            .into_iter()
            .map(|item| {
                // A card is a couple of hundred pixels: sixty-four bars,
                // each three quarters of its pitch, is what reads best at
                // that size - fuller than a lane's candles, on purpose.
                let duration = item.length.map_or(0.0, doc_view::len_secs) as f32;
                let wave = match studio.peaks.get(&item.id) {
                    Some(peaks) if item.kind == model::MediaKind::Audio => {
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
                                    wave_path(peaks, 0.0, duration, 64, 0.75, WAVE_FLOOR)
                                        .as_str()
                                        .into();
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
                    kind: media_kind_of(&item.kind),
                    duration,
                    format: std::path::Path::new(&item.path)
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .map(|extension| extension.to_ascii_lowercase())
                        .unwrap_or_default()
                        .into(),
                    thumbnail: self.thumbs.get(&item.id).cloned().unwrap_or_default(),
                    wave,
                    selected: self.selected.contains(&item.id),
                    pick: order
                        .iter()
                        .position(|id| *id == item.id)
                        .map_or(0, |at| at as i32 + 1),
                    preparing: concat_host::proxy::progress(&item.path).unwrap_or(-1.0),
                }
            })
            .collect()
    }
}

/// The document's kind as the card's. A kind a later build wrote shows
/// as a video, the card that says least about it.
fn media_kind_of(kind: &model::MediaKind) -> MediaKind {
    match kind {
        model::MediaKind::Video | model::MediaKind::Unknown(_) => MediaKind::Video,
        model::MediaKind::Audio => MediaKind::Audio,
        model::MediaKind::Image => MediaKind::Image,
    }
}

/// Every file of the bin as a card, in the order they came in.
fn cards(doc: &Model) -> impl Iterator<Item = Card<'_>> {
    doc.media.iter().map(|(media, item)| Card {
        id: media.to_string(),
        media,
        item: &**item,
    })
}

/// The card of the file an id the panels hold names.
fn card<'a>(doc: &'a Model, id: &str) -> Option<Card<'a>> {
    let media: MediaId = doc_view::parse(id)?;
    let item = doc_view::media(doc, media)?;
    Some(Card {
        id: media.to_string(),
        media,
        item,
    })
}

/// The lowest lane of the sequence showing that holds a picture: the main
/// track Add lays pictures onto. None while the timeline has no picture.
fn picture_lane(doc: &Model) -> Option<model::LaneId> {
    doc_view::lanes(doc).into_iter().find(|lane| {
        doc_view::on_lane(doc, *lane)
            .any(|(_, item)| matches!(doc_view::kind(doc, item), Kind::Video | Kind::Image))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc_view::bench::Bench;
    use model::MediaKind as Kind;

    /// A bin of four files, each named, and their ids by the short names
    /// the tests use: v1 Zebra.mp4, a1 alpha.wav, i1 Mid.png, v2 apple.mov.
    struct Bin {
        bench: Bench,
        ids: Vec<(&'static str, String)>,
    }

    impl Bin {
        fn doc(&self) -> &Model {
            self.bench.doc()
        }

        /// The id a short name stands for.
        fn id(&self, short: &str) -> String {
            self.ids
                .iter()
                .find(|(name, _)| *name == short)
                .map(|(_, id)| id.clone())
                .unwrap_or_else(|| short.to_owned())
        }

        fn add(&mut self, short: &'static str, name: &str, kind: Kind, origin: Option<MediaOrigin>) {
            let media = Media {
                path: format!("/media/{name}"),
                name: name.to_owned(),
                has_sound: kind != Kind::Image,
                kind,
                origin,
                ..Media::default()
            };
            let id = self
                .bench
                .apply(&Command::AddMedia { media })
                .created
                .and_then(|id| id.typed::<MediaId>())
                .expect("added");
            self.ids.push((short, id.to_string()));
        }

        fn remove(&mut self, short: &str) {
            let media = doc_view::parse(&self.id(short)).expect("an id");
            self.bench.apply(&Command::RemoveMedia { media });
        }
    }

    fn project() -> Bin {
        let mut bin = Bin {
            bench: Bench::new(),
            ids: Vec::new(),
        };
        for (short, name, kind) in [
            ("v1", "Zebra.mp4", Kind::Video),
            ("a1", "alpha.wav", Kind::Audio),
            ("i1", "Mid.png", Kind::Image),
            ("v2", "apple.mov", Kind::Video),
        ] {
            bin.add(short, name, kind, None);
        }
        bin
    }

    #[test]
    fn rows_are_assigned_once_and_never_reused() {
        let mut bin = MediaBin::default();
        let mut project = project();
        bin.assign_rows(project.doc());
        let zebra = bin.by_row(project.doc(), 1).expect("row 1").id;
        assert_eq!(zebra, project.id("v1"));
        // The first item goes; its row goes with it and is never given out
        // again, so a payload in flight cannot name the wrong file.
        project.remove("v1");
        bin.assign_rows(project.doc());
        assert!(bin.by_row(project.doc(), 1).is_none());
        project.add("v3", "new.mp4", Kind::Video, None);
        bin.assign_rows(project.doc());
        assert_eq!(
            bin.by_row(project.doc(), 5).expect("a new row").id,
            project.id("v3")
        );
        assert!(bin.by_row(project.doc(), 0).is_none(), "zero names nothing");
        assert!(bin.by_row(project.doc(), -7).is_none());
    }

    #[test]
    fn a_filter_and_a_sort_shape_the_visible_list() {
        let mut bin = MediaBin::default();
        let project = project();
        bin.filter = MediaFilter::Video;
        bin.sort = 1;
        let names: Vec<String> = bin
            .visible(project.doc())
            .iter()
            .map(|item| item.name.clone())
            .collect();
        assert_eq!(names, ["apple.mov", "Zebra.mp4"], "by name, case aside");
        bin.filter = MediaFilter::All;
        bin.sort = 2;
        let names: Vec<String> = bin
            .visible(project.doc())
            .iter()
            .map(|item| item.name.clone())
            .collect();
        assert_eq!(names, ["apple.mov", "Zebra.mp4", "alpha.wav", "Mid.png"]);
        bin.sort = 99;
        assert_eq!(
            bin.visible(project.doc()).len(),
            4,
            "an unknown sort keeps the order"
        );
    }

    #[test]
    fn a_generated_voice_is_on_its_own_shelf_and_no_other() {
        let mut bin = MediaBin::default();
        let mut project = project();
        project.add("s1", "Voice 1.wav", Kind::Audio, Some(MediaOrigin::Speech));
        let names = |bin: &MediaBin| -> Vec<String> {
            bin.visible(project.doc())
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
        let caught = bin.band(project.doc(), 3, (0, 2), (0, 0), false);
        assert_eq!(caught.len(), 1);
        assert!(caught.contains(&project.id("s1")));
        // Rows are minted for it like any other, so a drag can name it.
        bin.assign_rows(project.doc());
        assert_eq!(
            bin.by_row(project.doc(), 5).expect("row 5").id,
            project.id("s1")
        );
    }

    #[test]
    fn a_click_toggles_only_when_asked() {
        let mut bin = MediaBin::default();
        let project = project();
        let id = |short: &str| project.id(short);
        bin.select(project.doc(), id("a1"), false, false);
        bin.select(project.doc(), id("v1"), false, false);
        assert_eq!(bin.selected.len(), 1);
        assert!(bin.selected.contains(&id("v1")));
        bin.select(project.doc(), id("a1"), true, false);
        assert_eq!(bin.selected.len(), 2);
        bin.select(project.doc(), id("a1"), true, false);
        assert!(!bin.selected.contains(&id("a1")));
    }

    #[test]
    fn shift_click_selects_a_range_in_visible_order() {
        let mut bin = MediaBin::default();
        let project = project();
        let id = |short: &str| project.id(short);
        // Added order: v1, a1, i1, v2. Anchor on v1, Shift on i1 → those three.
        bin.select(project.doc(), id("v1"), false, false);
        bin.select(project.doc(), id("i1"), false, true);
        assert_eq!(bin.selected.len(), 3);
        assert!(
            bin.selected.contains(&id("v1"))
                && bin.selected.contains(&id("a1"))
                && bin.selected.contains(&id("i1"))
        );
        // Sorted by name: alpha.wav, apple.mov, Mid.png, Zebra.mp4.
        // Anchor alpha → Mid includes apple between them.
        bin.sort = 1;
        bin.select(project.doc(), id("a1"), false, false);
        bin.select(project.doc(), id("i1"), false, true);
        assert_eq!(bin.selected.len(), 3);
        assert!(
            bin.selected.contains(&id("a1"))
                && bin.selected.contains(&id("v2"))
                && bin.selected.contains(&id("i1"))
        );
        assert!(!bin.selected.contains(&id("v1")));
    }

    #[test]
    fn picks_are_numbered_in_the_order_they_were_made() {
        let mut bin = MediaBin::default();
        let project = project();
        let id = |short: &str| project.id(short);
        let ids = |shorts: &[&str]| -> Vec<String> { shorts.iter().map(|s| id(s)).collect() };
        bin.select(project.doc(), id("i1"), true, false);
        bin.select(project.doc(), id("v1"), true, false);
        bin.select(project.doc(), id("a1"), true, false);
        assert_eq!(bin.picked_in_order(project.doc()), ids(&["i1", "v1", "a1"]));
        // Let go of and picked again, it goes to the back of the line.
        bin.select(project.doc(), id("i1"), true, false);
        assert_eq!(bin.picked_in_order(project.doc()), ids(&["v1", "a1"]));
        bin.select(project.doc(), id("i1"), true, false);
        assert_eq!(bin.picked_in_order(project.doc()), ids(&["v1", "a1", "i1"]));
        // Selected some other way, it follows in the bin's order.
        bin.selected.insert(id("v2"));
        assert_eq!(
            bin.picked_in_order(project.doc()),
            ids(&["v1", "a1", "i1", "v2"])
        );
        // A plain click starts over.
        bin.select(project.doc(), id("v2"), false, false);
        assert_eq!(bin.picked_in_order(project.doc()), ids(&["v2"]));
    }

    #[test]
    fn a_band_walks_the_visible_grid_and_survives_zero_columns() {
        let mut bin = MediaBin::default();
        let project = project();
        let id = |short: &str| project.id(short);
        // Two columns over all four: rows are (v1 a1) (i1 v2).
        let caught = bin.band(project.doc(), 2, (1, 1), (0, 1), false);
        assert_eq!(caught.len(), 2);
        assert!(caught.contains(&id("a1")) && caught.contains(&id("v2")));
        // Zero or negative columns is one column, not a division by zero.
        let caught = bin.band(project.doc(), 0, (0, 0), (0, 3), false);
        assert_eq!(caught.len(), 4);
        let caught = bin.band(project.doc(), -3, (0, 0), (5, 9), false);
        assert!(caught.is_empty(), "rows past the end catch nothing");
        // Additive keeps what was there; a filter changes the walk.
        bin.selected.insert("x".into());
        bin.filter = MediaFilter::Images;
        let caught = bin.band(project.doc(), 1, (0, 0), (0, 0), true);
        assert_eq!(caught.len(), 2);
        assert!(caught.contains(&id("i1")) && caught.contains("x"));
        // Sort by name: alpha.wav, apple.mov, Mid.png, Zebra.mp4 → cell 1 is apple.
        bin.filter = MediaFilter::All;
        bin.sort = 1;
        let caught = bin.band(project.doc(), 1, (0, 0), (1, 1), false);
        assert_eq!(caught.len(), 1);
        assert!(caught.contains(&id("v2")), "{caught:?}");
    }

    /// The phone's Add lays pictures on the lowest lane that holds one,
    /// and a timeline with none yet has no such lane.
    #[test]
    fn the_picture_lane_is_the_lowest_with_a_picture() {
        let mut project = project();
        assert_eq!(picture_lane(project.doc()), None);
        let still: MediaId = doc_view::parse(&project.id("i1")).unwrap();
        let sound: MediaId = doc_view::parse(&project.id("a1")).unwrap();
        let main = project.bench.main_lane();
        let above = project.bench.lane(0);
        project.bench.add_clip(sound, main, 0.0);
        assert_eq!(picture_lane(project.doc()), None, "a sound is no picture");
        project.bench.add_clip(still, above, 0.0);
        assert_eq!(picture_lane(project.doc()), Some(above));
    }
}
