// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Templates: the gallery, and the sheet that fills one.
//!
//! The library is `concat_host::templates`; this is the window's side of
//! it. A card picked puts up the sheet with one row per slot; each row
//! takes a file, probed on a worker; once every slot has one the template
//! is unpacked into a new project, filled through the engine, and opened.

use std::path::{Path, PathBuf};

use concat_host::media::{self, MediaSummary};
use concat_host::projects::{self, ProjectInfo};
use concat_host::templates::{self, SlotFill, TemplateInfo};
use concat_project::model::MediaKind;
use slint::VecModel;

use crate::host::{image_at, spawn};
use crate::i18n::{t, tf};
use crate::studio::Studio;
use crate::ui::{TemplateCardData, TemplateFillData, TemplateSlotData};

/// Everything that can happen to the gallery and its sheet.
#[derive(Debug)]
pub enum TemplatesMsg {
    /// Read the library again: after a save, a delete, or at launch.
    Refresh,
    /// A card was picked: put up its sheet.
    Open(String),
    Dismiss,
    NameEdited(String),
    /// Choose the file for one slot.
    PickSlot(i32),
    /// Choose several files at once, for the open slots in order.
    PickAll,
    /// Files chosen for slots, by index, from either picker.
    Chosen(Vec<(usize, PathBuf)>),
    /// One slot's file is probed.
    Probed {
        index: usize,
        path: PathBuf,
        /// Boxed: a summary is large, and every message pays for the
        /// largest variant.
        result: Box<Result<MediaSummary, String>>,
    },
    ClearSlot(i32),
    Create,
    /// The worker that made the project is done.
    Created(Result<ProjectInfo, String>),
    /// A card's cross: ask before the template goes.
    AskDelete {
        path: String,
        name: String,
    },
}

/// One slot of the open sheet: what the template asks for, and what the
/// user has put in it so far.
struct Slot {
    info: concat_host::templates::SlotInfo,
    path: Option<PathBuf>,
    summary: Option<MediaSummary>,
    picture: Option<slint::Image>,
    busy: bool,
}

/// The gallery's state, and the sheet's.
#[derive(Default)]
pub struct TemplatesPane {
    /// Every template, the app's own first.
    pub list: Vec<TemplateInfo>,
    /// Posters by bundle folder, read once.
    posters: std::collections::HashMap<String, Option<slint::Image>>,
    /// The template the sheet is open on.
    open: Option<TemplateInfo>,
    slots: Vec<Slot>,
    name: String,
    busy: bool,
    error: String,
}

impl TemplatesPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: TemplatesMsg, studio: &mut Studio) {
        match msg {
            TemplatesMsg::Refresh => self.refresh(&studio.host.dirs.config),
            TemplatesMsg::Open(path) => self.open(&path, studio),
            TemplatesMsg::Dismiss => {
                if !self.busy {
                    self.close();
                }
            }
            TemplatesMsg::NameEdited(name) => self.name = name,
            TemplatesMsg::PickSlot(index) => {
                let Some(index) = self.index(index) else {
                    return;
                };
                crate::host::pick_files_then(
                    t("templates.chooseFileForSlot"),
                    None,
                    move |studio, _, _, paths| {
                        if let Some(path) = paths.into_iter().next() {
                            studio.handle(crate::panes::Msg::Templates(TemplatesMsg::Chosen(
                                vec![(index, path)],
                            )));
                        }
                    },
                );
            }
            TemplatesMsg::PickAll => {
                // The open slots first, in the order they play; with every
                // slot full, the files go from the top again, so a second
                // go replaces the first.
                let mut order: Vec<usize> = (0..self.slots.len())
                    .filter(|&index| self.slots[index].path.is_none())
                    .collect();
                if order.is_empty() {
                    order = (0..self.slots.len()).collect();
                }
                crate::host::pick_files_then(
                    t("templates.chooseFilesForSlots"),
                    None,
                    move |studio, _, _, paths| {
                        let chosen = order.into_iter().zip(paths).collect();
                        studio.handle(crate::panes::Msg::Templates(TemplatesMsg::Chosen(chosen)));
                    },
                );
            }
            TemplatesMsg::Chosen(chosen) => self.chosen(chosen),
            TemplatesMsg::Probed {
                index,
                path,
                result,
            } => self.probed(index, path, *result),
            TemplatesMsg::ClearSlot(index) => {
                if let Some(slot) = self
                    .index(index)
                    .and_then(|index| self.slots.get_mut(index))
                {
                    slot.path = None;
                    slot.summary = None;
                    slot.picture = None;
                    slot.busy = false;
                }
            }
            TemplatesMsg::Create => self.create(studio),
            TemplatesMsg::Created(result) => {
                self.busy = false;
                match result {
                    Ok(info) => {
                        self.close();
                        // The project the window is on, if any, is saved
                        // and closed first: the new one takes its place.
                        if studio.session.is_some() {
                            studio.close_project();
                            // A project that could not be saved stays
                            // open, and says why; the new one waits in
                            // its folder, to be opened from there.
                            if studio.session.is_some() {
                                return;
                            }
                        }
                        if let Err(error) = studio.open_project(info) {
                            studio.notify(&error, true);
                        }
                    }
                    Err(error) => self.error = error,
                }
            }
            TemplatesMsg::AskDelete { path, name } => {
                studio.confirm = Some(crate::studio::Confirm::DeleteTemplate { path, name });
            }
        }
    }

    /// Reads the library again. Posters are kept for the bundles still
    /// there; one that was deleted and saved again under the same name has
    /// a new folder time and is read afresh.
    pub fn refresh(&mut self, config: &Path) {
        self.list = templates::list(config);
        self.posters
            .retain(|path, _| self.list.iter().any(|info| &info.path == path));
        for info in self.list.iter().filter(|info| info.has_poster) {
            self.posters
                .entry(info.path.clone())
                .or_insert_with(|| image_at(&Path::new(&info.path).join("poster.jpg")));
        }
    }

    fn index(&self, index: i32) -> Option<usize> {
        usize::try_from(index)
            .ok()
            .filter(|index| *index < self.slots.len())
    }

    fn open(&mut self, path: &str, studio: &Studio) {
        let Some(info) = self.list.iter().find(|info| info.path == path).cloned() else {
            return;
        };
        self.slots = info
            .slots
            .iter()
            .cloned()
            .map(|info| Slot {
                info,
                path: None,
                summary: None,
                picture: None,
                busy: false,
            })
            .collect();
        self.name = free_name(&studio.start.location, &display_name(&info));
        self.error.clear();
        self.busy = false;
        self.open = Some(info);
    }

    fn close(&mut self) {
        self.open = None;
        self.slots.clear();
        self.error.clear();
    }

    /// Files went into slots: each is probed on a worker, and the row says
    /// so until it answers.
    fn chosen(&mut self, chosen: Vec<(usize, PathBuf)>) {
        for (index, path) in chosen {
            let Some(slot) = self.slots.get_mut(index) else {
                continue;
            };
            slot.path = Some(path.clone());
            slot.summary = None;
            slot.picture = None;
            slot.busy = true;
            spawn(
                {
                    let path = path.clone();
                    move || media::probe(&path.to_string_lossy())
                },
                move |studio, _, _, result| {
                    studio.handle(crate::panes::Msg::Templates(TemplatesMsg::Probed {
                        index,
                        path,
                        result: Box::new(result),
                    }));
                },
            );
        }
        self.error.clear();
    }

    fn probed(&mut self, index: usize, path: PathBuf, result: Result<MediaSummary, String>) {
        let Some(slot) = self.slots.get_mut(index) else {
            return;
        };
        // A file chosen again since this probe started is the one that
        // counts; this answer is for a file the slot no longer holds.
        if slot.path.as_deref() != Some(path.as_path()) {
            return;
        }
        slot.busy = false;
        match result {
            Ok(summary) if summary.kind != MediaKind::Audio => {
                slot.picture = (summary.kind == MediaKind::Image)
                    .then(|| image_at(&path))
                    .flatten();
                slot.summary = Some(summary);
            }
            Ok(_) => {
                slot.path = None;
                self.error = tf("templates.notAPicture", &[&file_name(&path)]);
            }
            Err(error) => {
                slot.path = None;
                self.error = tf("templates.couldNotRead", &[&file_name(&path), &error]);
            }
        }
    }

    /// Makes the project on a worker: the files copy nothing, but the
    /// template's own assets do, and a slow disk should not freeze the
    /// window.
    fn create(&mut self, studio: &Studio) {
        let Some(template) = self.open.as_ref() else {
            return;
        };
        let fills: Option<Vec<SlotFill>> = self
            .slots
            .iter()
            .map(|slot| {
                slot.summary.as_ref().map(|summary| SlotFill {
                    media_id: slot.info.media_id.clone(),
                    item: summary.to_new_media(),
                })
            })
            .collect();
        let Some(fills) = fills else {
            self.error = t("templates.fillEverySlot");
            return;
        };
        let location = studio.start.location.trim().to_owned();
        if location.is_empty() {
            self.error = t("start.chooseWhereProjectFolder");
            return;
        }
        let name = match self.name.trim() {
            "" => free_name(&location, &display_name(template)),
            typed => typed.to_owned(),
        };
        let bundle = template.path.clone();
        self.busy = true;
        self.error.clear();
        spawn(
            move || {
                std::fs::create_dir_all(&location)
                    .map_err(|error| format!("could not create {location}: {error}"))?;
                templates::instantiate(&bundle, &location, &name, fills)
            },
            |studio, _, _, result| {
                studio.handle(crate::panes::Msg::Templates(TemplatesMsg::Created(result)));
            },
        );
    }

    /// The gallery's cards.
    pub fn cards(&self) -> Vec<TemplateCardData> {
        self.list
            .iter()
            .map(|info| TemplateCardData {
                path: info.path.as_str().into(),
                name: display_name(info).into(),
                detail: detail(info).into(),
                poster: self
                    .posters
                    .get(&info.path)
                    .cloned()
                    .flatten()
                    .unwrap_or_default(),
                ratio: info.width as f32 / info.height.max(1) as f32,
                slots: info.slots.len() as i32,
                builtin: info.builtin,
            })
            .collect()
    }

    /// The sheet as Slint shows it.
    pub fn data(&self) -> TemplateFillData {
        let Some(info) = self.open.as_ref() else {
            return TemplateFillData::default();
        };
        let slots: Vec<TemplateSlotData> = self
            .slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                let seconds = tf("templates.seconds", &[&format!("{:.1}", slot.info.seconds)]);
                let named = !info.builtin && !slot.info.name.trim().is_empty();
                TemplateSlotData {
                    label: (index + 1).to_string().into(),
                    detail: if named {
                        format!("{seconds} · {}", slot.info.name)
                    } else {
                        seconds
                    }
                    .into(),
                    file: slot
                        .path
                        .as_deref()
                        .map(file_name)
                        .unwrap_or_default()
                        .into(),
                    picture: slot.picture.clone().unwrap_or_default(),
                    filled: slot.summary.is_some(),
                    still: slot
                        .summary
                        .as_ref()
                        .is_some_and(|summary| summary.kind == MediaKind::Image),
                    busy: slot.busy,
                }
            })
            .collect();
        TemplateFillData {
            open: true,
            title: display_name(info).into(),
            detail: format!("{} × {}", info.width, info.height).into(),
            ratio: info.width as f32 / info.height.max(1) as f32,
            poster: Default::default(),
            name: self.name.as_str().into(),
            filled: self
                .slots
                .iter()
                .filter(|slot| slot.summary.is_some())
                .count() as i32,
            total: self.slots.len() as i32,
            slots: slint::ModelRc::new(VecModel::from(slots)),
            busy: self.busy,
            error: self.error.as_str().into(),
        }
    }
}

/// A template's name in the interface's language: the app's own are
/// translated by id, the user's are what they were saved as.
pub fn display_name(info: &TemplateInfo) -> String {
    if !info.builtin {
        return info.name.clone();
    }
    match info.id.as_str() {
        "photo-slideshow" => t("templates.photoSlideshow"),
        "quick-cuts" => t("templates.quickCuts"),
        "vlog-intro" => t("templates.vlogIntro"),
        _ => info.name.clone(),
    }
}

/// "9:16 · 6 slots · 15s".
fn detail(info: &TemplateInfo) -> String {
    let slots = match info.slots.len() {
        1 => t("templates.oneSlot"),
        many => tf("templates.slots", &[&many]),
    };
    format!(
        "{} · {slots} · {}",
        aspect(info.width, info.height),
        tf(
            "templates.seconds",
            &[&format!("{:.0}", info.seconds.ceil())]
        )
    )
}

/// The frame's shape as people say it: "16:9", "9:16", "1:1"; the exact
/// size where it is none of the usual ones.
fn aspect(width: u32, height: u32) -> String {
    fn gcd(a: u32, b: u32) -> u32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let divisor = gcd(width, height).max(1);
    let (w, h) = (width / divisor, height / divisor);
    if w <= 32 && h <= 32 {
        format!("{w}:{h}")
    } else {
        format!("{width}×{height}")
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// `name`, or `name 2`, `name 3`, ... - the first with no folder of that
/// name in `location`, so a second video from one template is not refused.
fn free_name(location: &str, name: &str) -> String {
    let location = location.trim();
    let taken = |candidate: &str| {
        !location.is_empty()
            && Path::new(location)
                .join(projects::folder_name(candidate))
                .exists()
    };
    std::iter::once(name.to_owned())
        .chain((2..).map(|n| format!("{name} {n}")))
        .find(|candidate| !taken(candidate))
        .expect("the numbers do not run out")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_read_the_way_people_say_them() {
        assert_eq!(aspect(1920, 1080), "16:9");
        assert_eq!(aspect(1080, 1920), "9:16");
        assert_eq!(aspect(1080, 1080), "1:1");
        assert_eq!(aspect(1080, 1350), "4:5");
        assert_eq!(aspect(1999, 1000), "1999×1000");
    }

    #[test]
    fn a_second_video_from_one_template_gets_its_own_name() {
        let root =
            std::env::temp_dir().join(format!("concat-template-name-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Quick cuts")).unwrap();
        let location = root.to_string_lossy().into_owned();
        assert_eq!(free_name(&location, "Quick cuts"), "Quick cuts 2");
        assert_eq!(free_name(&location, "Vlog intro"), "Vlog intro");
        let _ = std::fs::remove_dir_all(&root);
    }
}
