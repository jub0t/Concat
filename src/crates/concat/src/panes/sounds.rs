// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The library's Audio page (#60): the user's own sounds, kept across
//! projects.
//!
//! A sound comes in by the page's Import button, or by being dropped on
//! the window while the page is up; it is copied into the app's own
//! folder (see `concat_host::sounds`) on a worker, and its card shows its
//! name, its length and its envelope. A click plays it through the source
//! preview, as a sound in the bin plays; its plus lays it at the playhead
//! on the first lane with room, and a drag lays it where it is dropped.
//! Either way it is copied into the project's folder and imported into the
//! bin first, so the project owns its own copy and a sound later removed
//! from the library takes nothing with it.
//!
//! A card is named by an id with its shelf in front - `mine-…` - so one
//! payload, `sound:<card>:<name>`, carries any shelf's sound.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use concat_host::sounds::{self, Catalogue, Library, Sound};
use concat_project::Command;
use slint::SharedString;

use crate::i18n::{t, tf};
use crate::panes::Msg;
use crate::studio::Studio;
use crate::ui::{SoundCardData, VoiceCardData};

/// The prefix of a card id on the Mine shelf.
const MINE: &str = "mine-";
/// The prefix of a card id on the Concat shelf.
const CONCAT: &str = "concat-";

/// The Concat shelf's categories, as the manifest names them; the sidebar
/// shows them in the interface's language, after All.
pub const CATEGORIES: [&str; 5] = ["Whoosh", "Impact", "Riser", "Ambience", "Music"];

/// Where the Concat shelf stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConcatState {
    /// Not asked for: the shelf offers to load it.
    #[default]
    Idle,
    Loading,
    Ready,
    /// The assets repository has no manifest yet.
    Soon,
    /// It could not be reached.
    Offline,
}

/// What to do with a Concat sound once it has arrived.
#[derive(Clone, Debug)]
pub enum Then {
    /// Play it.
    Preview,
    /// Lay it on this lane, or the first with room, at this moment.
    Place(Option<String>, f64),
}

/// Everything that can happen to the Audio page.
#[derive(Clone, Debug)]
pub enum SoundsMsg {
    /// Files picked, or dropped while the page was up.
    Import(Vec<PathBuf>),
    /// The import's worker is done copying and reading.
    Imported(Vec<Result<Sound, String>>),
    /// A card clicked: play it, or pause it when it is the one playing.
    Preview(String),
    /// A card's plus: onto the timeline at the playhead.
    Add(String),
    /// "Remove from library": the copy deleted.
    Remove(String),
    /// Whether a library is showing the page, so a drop of sound files on
    /// the window goes into the library rather than the bin.
    Shown(bool),
    /// "Load Concat sounds", or Refresh: the manifest fetched.
    LoadConcat,
    /// The manifest's worker is done: the list, none yet, or why not.
    ConcatLoaded(Result<Option<Catalogue>, String>),
    /// A category of the Concat shelf picked: 0 All, then [`CATEGORIES`].
    ConcatCategory(i32),
    /// A Concat sound's download, how far.
    Fetching(String, f32),
    /// A Concat sound's download is done.
    Fetched(String, Result<PathBuf, String>),
}

/// The page's state.
#[derive(Default)]
pub struct SoundsPane {
    /// The user's own sounds.
    pub library: Library,
    /// A library is showing the page.
    pub shown: bool,
    /// A sound import running.
    pub importing: bool,
    /// Concat's sounds, once the manifest has been read, and where the
    /// shelf stands.
    pub catalogue: Catalogue,
    pub concat: ConcatState,
    /// The category shown: 0 All, then a row of [`CATEGORIES`].
    pub category: usize,
    /// Downloads running, by card: how far, and what to do once in.
    fetching: HashMap<String, (f32, Then)>,
    /// Stops the downloads when the window goes.
    cancel: Arc<AtomicBool>,
    /// Where the downloads go: the app's data folder.
    dirs: Option<concat_host::AppDirs>,
    /// Each card's envelope as path commands, by card id: the cards are
    /// published on every frame of playback, and the path is the one
    /// expensive line of one.
    waves: RefCell<HashMap<String, SharedString>>,
}

impl SoundsPane {
    /// The library as it is on this machine's disk.
    pub fn open(dirs: &concat_host::AppDirs) -> SoundsPane {
        // The manifest as last fetched, if it ever was: the shelf is
        // there from the first frame, and no request is made for it.
        let catalogue = sounds::cached_catalogue(dirs);
        SoundsPane {
            library: Library::open(dirs),
            concat: if catalogue.is_some() {
                ConcatState::Ready
            } else {
                ConcatState::Idle
            },
            catalogue: catalogue.unwrap_or_default(),
            dirs: Some(dirs.clone()),
            ..SoundsPane::default()
        }
    }

    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: SoundsMsg, studio: &mut Studio) {
        match msg {
            SoundsMsg::Shown(on) => self.shown = on,
            SoundsMsg::LoadConcat => {
                let Some(dirs) = self.dirs.clone() else {
                    return;
                };
                if self.concat == ConcatState::Loading {
                    return;
                }
                if self.concat != ConcatState::Ready {
                    self.concat = ConcatState::Loading;
                }
                crate::host::spawn(
                    move || sounds::fetch_catalogue(&dirs),
                    |studio, _, _, result| {
                        studio.handle(Msg::Sounds(SoundsMsg::ConcatLoaded(result)));
                    },
                );
            }
            SoundsMsg::ConcatLoaded(result) => match result {
                Ok(Some(catalogue)) => {
                    self.catalogue = catalogue;
                    self.concat = ConcatState::Ready;
                }
                Ok(None) => self.concat = ConcatState::Soon,
                Err(error) => {
                    log::info!("sounds: the Concat list could not be fetched: {error}");
                    // A list already on screen stays: the refresh failed,
                    // not the shelf.
                    if self.concat != ConcatState::Ready {
                        self.concat = ConcatState::Offline;
                    } else {
                        studio.notify(&t("sounds.offline"), true);
                    }
                }
            },
            SoundsMsg::ConcatCategory(index) => {
                self.category = (index.max(0) as usize).min(CATEGORIES.len());
            }
            SoundsMsg::Fetching(card, done) => {
                if let Some((progress, _)) = self.fetching.get_mut(&card) {
                    *progress = done;
                }
            }
            SoundsMsg::Fetched(card, result) => {
                let Some((_, then)) = self.fetching.remove(&card) else {
                    return;
                };
                let name = self
                    .listed(&card)
                    .map(|sound| sound.name.clone())
                    .unwrap_or_default();
                match result {
                    Ok(path) => match then {
                        Then::Preview => studio.preview_file(&card, &name, &path),
                        Then::Place(track_id, start) => {
                            studio.place_sound(&name, &path, track_id, start);
                        }
                    },
                    Err(error) if error == "cancelled" => {}
                    Err(error) => {
                        studio.notify(&tf("sounds.downloadFailed", &[&name, &error]), true);
                    }
                }
            }
            SoundsMsg::Import(paths) => {
                let (taken, refused): (Vec<PathBuf>, Vec<PathBuf>) =
                    paths.into_iter().partition(|path| sounds::is_sound(path));
                if let Some(path) = refused.first() {
                    studio.notify(
                        &tf("sounds.notASound", &[&display_name(path)]),
                        true,
                    );
                }
                if taken.is_empty() {
                    return;
                }
                self.importing = true;
                let dir = self.library.dir.clone();
                crate::host::spawn(
                    move || {
                        taken
                            .iter()
                            .map(|path| sounds::prepare(&dir, path))
                            .collect::<Vec<_>>()
                    },
                    |studio, _, _, results| {
                        studio.handle(Msg::Sounds(SoundsMsg::Imported(results)));
                    },
                );
            }
            SoundsMsg::Imported(results) => {
                self.importing = false;
                let mut added = 0;
                let mut failure = None;
                // Oldest first, so the last picked ends up at the top
                // with the rest of them under it in the order picked.
                for result in results.into_iter().rev() {
                    match result.and_then(|sound| self.library.add(sound)) {
                        Ok(()) => added += 1,
                        Err(error) => failure = Some(error),
                    }
                }
                if let Some(error) = failure {
                    studio.notify(&error, true);
                } else if added == 1 {
                    studio.notify(&t("sounds.addedOne"), false);
                } else if added > 1 {
                    studio.notify(&tf("sounds.addedMany", &[&added]), false);
                }
            }
            SoundsMsg::Preview(card) => {
                if let Some((name, path)) = self.file_of(&card) {
                    studio.preview_file(&card, &name, &path);
                } else {
                    self.fetch(&card, Then::Preview);
                }
            }
            SoundsMsg::Add(card) => {
                let start = f64::from(studio.playhead.max(0.0));
                if let Some((name, path)) = self.file_of(&card) {
                    studio.place_sound(&name, &path, None, start);
                } else {
                    self.fetch(&card, Then::Place(None, start));
                }
            }
            SoundsMsg::Remove(card) => {
                let Some(id) = card.strip_prefix(MINE) else {
                    return;
                };
                // Not playing what is about to be deleted.
                if studio
                    .source
                    .as_ref()
                    .is_some_and(|source| source.card == card)
                {
                    studio.close_source();
                }
                let name = self
                    .library
                    .get(id)
                    .map(|sound| sound.name.clone())
                    .unwrap_or_default();
                match self.library.remove(id) {
                    Ok(()) => {
                        self.waves.borrow_mut().remove(&card);
                        studio.notify(&tf("sounds.removed", &[&name]), false);
                    }
                    Err(error) => studio.notify(&error, true),
                }
            }
        }
    }

    /// The name and the file a card stands for, when it is on disk: a
    /// sound of the user's own, or one of Concat's already downloaded.
    pub fn file_of(&self, card: &str) -> Option<(String, PathBuf)> {
        if let Some(sound) = self.listed(card) {
            let path = sounds::cached_file(self.dirs.as_ref()?, sound);
            return path.is_file().then(|| (sound.name.clone(), path));
        }
        let id = card.strip_prefix(MINE)?;
        let sound = self.library.get(id)?;
        Some((sound.name.clone(), self.library.path_of(sound)))
    }

    /// A card as a payload names it, for a drop: its name and its length.
    pub fn summary(&self, card: &str) -> Option<(String, f64)> {
        if let Some(sound) = self.listed(card) {
            return Some((sound.name.clone(), sound.duration));
        }
        let id = card.strip_prefix(MINE)?;
        let sound = self.library.get(id)?;
        Some((sound.name.clone(), sound.duration))
    }

    /// The Concat sound a card names.
    fn listed(&self, card: &str) -> Option<&sounds::Listed> {
        self.catalogue.get(card.strip_prefix(CONCAT)?)
    }

    /// Downloads a Concat sound on a worker, then does `then` with it. A
    /// second ask while it comes only changes what is done with it.
    pub fn fetch(&mut self, card: &str, then: Then) {
        let (Some(sound), Some(dirs)) = (self.listed(card).cloned(), self.dirs.clone()) else {
            return;
        };
        if let Some((_, held)) = self.fetching.get_mut(card) {
            *held = then;
            return;
        }
        self.fetching.insert(card.to_owned(), (0.0, then));
        let cancel = Arc::clone(&self.cancel);
        let (key, done) = (card.to_owned(), card.to_owned());
        crate::host::spawn(
            move || {
                sounds::fetch_sound(&dirs, &sound, &cancel, &mut |received, total| {
                    if total > 0 {
                        let fraction = (received as f32 / total as f32).clamp(0.0, 1.0);
                        let key = key.clone();
                        crate::host::on_ui(move |studio, _, _| {
                            studio.handle(Msg::Sounds(SoundsMsg::Fetching(key, fraction)));
                        });
                    }
                })
            },
            move |studio, _, _, result| {
                studio.handle(Msg::Sounds(SoundsMsg::Fetched(done, result)));
            },
        );
    }

    /// The Concat shelf's rows, narrowed to the category picked.
    pub fn concat_rows(&self) -> Vec<SoundCardData> {
        let category = self.category.checked_sub(1).and_then(|row| CATEGORIES.get(row));
        let Some(dirs) = self.dirs.as_ref() else {
            return Vec::new();
        };
        self.catalogue
            .sounds
            .iter()
            .filter(|sound| category.is_none_or(|wanted| sound.category.eq_ignore_ascii_case(wanted)))
            .map(|sound| {
                let card = format!("{CONCAT}{}", sound.id);
                let progress = self.fetching.get(&card).map_or(-1.0, |(done, _)| *done);
                SoundCardData {
                    ready: sounds::cached_file(dirs, sound).is_file(),
                    id: card.into(),
                    name: sound.name.as_str().into(),
                    duration: sound.duration as f32,
                    wave: SharedString::new(),
                    detail: category_label(&sound.category).into(),
                    removable: false,
                    progress,
                }
            })
            .collect()
    }

    /// The Mine shelf's cards.
    pub fn mine(&self) -> Vec<SoundCardData> {
        self.library
            .sounds
            .iter()
            .map(|sound| {
                let card = format!("{MINE}{}", sound.id);
                SoundCardData {
                    wave: self.wave(&card, &sound.bars),
                    id: card.into(),
                    name: sound.name.as_str().into(),
                    duration: sound.duration as f32,
                    detail: SharedString::new(),
                    removable: true,
                    ready: true,
                    progress: -1.0,
                }
            })
            .collect()
    }

    /// The envelope of a card, drawn once.
    fn wave(&self, card: &str, bars: &[f32]) -> SharedString {
        if bars.is_empty() {
            return SharedString::new();
        }
        self.waves
            .borrow_mut()
            .entry(card.to_owned())
            .or_insert_with(|| crate::format::bars_path(bars, 0.7).into())
            .clone()
    }
}

/// A category as the interface says it: one of the five in its language,
/// or the manifest's own word.
pub fn category_label(category: &str) -> String {
    match category.to_ascii_lowercase().as_str() {
        "whoosh" => t("sounds.catWhoosh"),
        "impact" => t("sounds.catImpact"),
        "riser" => t("sounds.catRiser"),
        "ambience" => t("sounds.catAmbience"),
        "music" => t("sounds.catMusic"),
        _ => category.to_owned(),
    }
}

/// A file's name, for a notice.
fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Studio {
    /// The Voices shelf: the user's saved clones, the speakers of the voice
    /// models on disk, and the styles; and whether any model is on disk.
    pub fn voice_cards(&self) -> (Vec<VoiceCardData>, Vec<VoiceCardData>, Vec<VoiceCardData>, bool) {
        use crate::panes::speech::{STYLES, letter_avatar, voice_names};
        let saved = self
            .speech
            .saved
            .iter()
            .map(|voice| VoiceCardData {
                id: format!("saved-{}", voice.id).into(),
                name: voice.name.as_str().into(),
                detail: t("speech.savedVoice").into(),
                avatar: self.speech.saved_avatar(voice),
                removable: true,
            })
            .collect();
        let models = crate::panes::settings::installed(&self.settings.voices);
        let families: Vec<_> = models
            .iter()
            .map(|model| concat_speech::tts::family_of(&model.id))
            .collect();
        let builtin = self
            .speech
            .speakers
            .iter()
            .filter(|speaker| {
                families.contains(&speaker.family) && !concat_speech::tts::is_clone(speaker.id)
            })
            .map(|speaker| {
                let (name, detail) = voice_names(&speaker.name);
                VoiceCardData {
                    id: format!("builtin-{}", speaker.id).into(),
                    avatar: letter_avatar(&name),
                    name: name.into(),
                    detail: detail.into(),
                    removable: false,
                }
            })
            .collect();
        let catalogue = concat_effects::Catalogue::builtin();
        let styles = STYLES
            .iter()
            .enumerate()
            .map(|(row, style)| {
                let name = t(style.key);
                let effects: Vec<String> = style
                    .effects
                    .iter()
                    .filter_map(|id| catalogue.get(id))
                    .map(|package| package.manifest.effect.name.clone())
                    .collect();
                VoiceCardData {
                    id: format!("style-{row}").into(),
                    avatar: letter_avatar(&name),
                    name: name.into(),
                    detail: effects.join(" + ").into(),
                    removable: false,
                }
            })
            .collect();
        (saved, builtin, styles, !models.is_empty())
    }

    /// A card on the Voices shelf clicked: Text to speech on it.
    pub fn open_voice(&mut self, card: &str) {
        use crate::panes::speech::{SpeechMsg, VoiceChoice};
        let choice = if let Some(id) = card.strip_prefix("saved-") {
            VoiceChoice::Saved(id.to_owned())
        } else if let Some(id) = card.strip_prefix("builtin-").and_then(|id| id.parse().ok()) {
            VoiceChoice::Builtin(id)
        } else if let Some(row) = card.strip_prefix("style-").and_then(|row| row.parse().ok()) {
            VoiceChoice::Style(row)
        } else {
            return;
        };
        self.handle(Msg::Speech(SpeechMsg::OpenWith(choice)));
    }

    /// A saved voice's bin on the Voices shelf.
    pub fn forget_voice(&mut self, card: &str) {
        if let Some(id) = card.strip_prefix("saved-") {
            self.handle(Msg::Speech(crate::panes::speech::SpeechMsg::ForgetVoice(
                id.to_owned(),
            )));
        }
    }

    /// What a `sound:` payload's card is, for the drop's plan: its name
    /// and length.
    pub fn sound_summary(&self, card: &str) -> Option<(String, f64)> {
        self.sounds.summary(card)
    }

    /// A card dropped on a lane, or added at the playhead: see
    /// [`Studio::place_sound`].
    pub fn place_card(&mut self, card: &str, track_id: Option<String>, start: f64) {
        if let Some((name, path)) = self.sounds.file_of(card) {
            self.place_sound(&name, &path, track_id, start);
        } else {
            // One of Concat's not downloaded yet: it lands where it was
            // dropped once it is in.
            self.sounds.fetch(card, Then::Place(track_id, start));
        }
    }

    /// Lays a library sound on the timeline: copied into the project's
    /// folder and imported into the bin first - once; a second use finds
    /// the copy already there - then cut onto `track_id` at `start`, or at
    /// `start` on the first lane with room. Selects what it made.
    pub fn place_sound(&mut self, name: &str, path: &Path, track_id: Option<String>, start: f64) {
        let media_id = match self.adopt_sound(name, path) {
            Ok(id) => id,
            Err(error) => {
                self.notify(&error, true);
                return;
            }
        };
        // Onto the timeline: the monitor and the speakers go back to it.
        self.close_source();
        let created = match track_id {
            Some(track_id) => self.apply(Command::AddClip {
                media_id,
                track_id,
                start,
                ripple: true,
            }),
            None => self.apply(Command::AddClipAtFirstFree { media_id, start }),
        };
        if let Some(id) = created {
            self.selection = vec![id];
        }
    }

    /// The bin's id for a library sound in this project: its copy in the
    /// project's `audio` folder, made and imported when it is not there
    /// yet. A file of that name that is not this sound - another size -
    /// leaves it alone and the copy takes a numbered name.
    fn adopt_sound(&mut self, name: &str, path: &Path) -> Result<String, String> {
        let project = self
            .session
            .as_ref()
            .map(|session| PathBuf::from(session.path()))
            .ok_or_else(|| t("sounds.openProjectFirst"))?;
        let folder = project.join("audio");
        let size = std::fs::metadata(path)
            .map_err(|error| format!("{}: {error}", path.display()))?
            .len();
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_owned());
        let extension = path
            .extension()
            .map(|extension| format!(".{}", extension.to_string_lossy()))
            .unwrap_or_default();
        let mut target = folder.join(format!("{stem}{extension}"));
        let mut n = 2;
        while target.exists()
            && std::fs::metadata(&target).map(|meta| meta.len()).ok() != Some(size)
        {
            target = folder.join(format!("{stem} {n}{extension}"));
            n += 1;
        }
        let target_text = target.to_string_lossy().into_owned();
        if let Some(item) = self
            .project()
            .media
            .iter()
            .find(|item| item.path == target_text)
        {
            return Ok(item.id.clone());
        }
        if !target.exists() {
            std::fs::create_dir_all(&folder)
                .and_then(|()| std::fs::copy(path, &target).map(drop))
                .map_err(|error| tf("sounds.couldNotCopy", &[&name, &error]))?;
        }
        let summary = concat_host::media::probe(&target_text)?;
        let mut item = summary.to_new_media();
        item.name = format!("{name}{extension}");
        let created = self.apply(Command::AddMedia { item });
        created
            .or_else(|| {
                self.project()
                    .media
                    .iter()
                    .find(|item| item.path == target_text)
                    .map(|item| item.id.clone())
            })
            .ok_or_else(|| tf("sounds.couldNotCopy", &[&name, &"?"]))
    }
}
