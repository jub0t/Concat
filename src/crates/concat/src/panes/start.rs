// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The launch screen's form: a new project's name, place, shape, size and
//! rate, the verb that opens one that already exists, and the recent
//! list's own two verbs.

use concat_host::projects;

use crate::i18n::{t, tf};
use crate::platform;
use crate::studio::{ASPECTS, SIZES, START_RATES, Studio, frame_size, home_folder};
use crate::ui::StartData;

/// Everything that can happen to the launch screen's form.
#[derive(Clone, Debug)]
pub enum StartMsg {
    NameEdited(String),
    LocationEdited(String),
    /// The frame's shape: 16:9, 9:16, 1:1, 4:3.
    AspectChanged(i32),
    /// The frame's size, as the short edge: 720p, 1080p, 4K.
    SizeChanged(i32),
    RateChanged(i32),
    DismissError,
    /// Pick where the project folder goes.
    Browse,
    Create,
    /// Open a project that already exists, picked from disk.
    Open,
    OpenRecent(String),
    ForgetRecent(String),
}

/// The form on the launch screen.
pub struct StartPane {
    pub name: String,
    pub location: String,
    /// Index into [`ASPECTS`].
    pub aspect: usize,
    /// Index into [`SIZES`].
    pub size: usize,
    pub rate: usize,
    pub busy: bool,
    pub error: String,
}

impl Default for StartPane {
    fn default() -> Self {
        Self {
            name: "Untitled project".into(),
            // A phone has no desk: its projects live at the top of the
            // folder the file manager shows for the app.
            location: home_folder(if cfg!(target_os = "android") {
                "Concat"
            } else {
                "Desktop/Concat"
            }),
            aspect: 0,
            // 1080p, not the first of the three. The size everything else
            // in the app assumes, and the one a phone and a desk agree on.
            size: 1,
            rate: 3,
            busy: false,
            error: String::new(),
        }
    }
}

impl StartPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: StartMsg, studio: &mut Studio) {
        match msg {
            StartMsg::NameEdited(name) => self.name = name,
            StartMsg::LocationEdited(path) => self.location = path,
            StartMsg::AspectChanged(index) => {
                self.aspect = (index.max(0) as usize).min(ASPECTS.len() - 1);
            }
            StartMsg::SizeChanged(index) => {
                self.size = (index.max(0) as usize).min(SIZES.len() - 1);
            }
            StartMsg::RateChanged(index) => {
                self.rate = (index.max(0) as usize).min(START_RATES.len() - 1);
            }
            StartMsg::DismissError => self.error.clear(),
            StartMsg::Browse => {
                if let Some(folder) =
                    platform::pick_folder(&t("Where should the project folder go?"), &self.location)
                {
                    self.location = folder.to_string_lossy().into_owned();
                }
            }
            StartMsg::Create => self.create(studio),
            StartMsg::Open => self.open(studio),
            StartMsg::OpenRecent(path) => {
                let opened = projects::open(&path).and_then(|info| studio.open_project(info));
                self.opened(opened);
            }
            StartMsg::ForgetRecent(path) => {
                if let Err(error) = projects::forget(&studio.host.dirs.config, &path) {
                    self.error = error;
                }
                studio.recents = projects::list(&studio.host.dirs.config);
            }
        }
    }

    /// Makes the project the form describes and opens it.
    fn create(&mut self, studio: &mut Studio) {
        let name = self.name.trim().to_owned();
        let name = if name.is_empty() {
            "Untitled project".to_owned()
        } else {
            name
        };
        let (width, height) = frame_size(self.aspect, self.size);
        let (_, num, den) = START_RATES[self.rate.min(START_RATES.len() - 1)];
        if self.location.trim().is_empty() {
            self.error = t("Choose where the project folder should go");
            return;
        }
        let opened = projects::create(&self.location, &name, width, height, num, den)
            .and_then(|info| studio.open_project(info));
        self.opened(opened);
    }

    /// Opens a project folder that already exists.
    ///
    /// The folder is checked before it is read, so picking the wrong one
    /// says which folder and what was wrong with it rather than reporting a
    /// missing file by its path — which is what `projects::open` has to say
    /// about a folder that was never a project in the first place.
    ///
    /// This is also what File › Open project does, from the menu bar of a
    /// window that already has a project in it. That is why it reports
    /// through the toast rather than through the form's own notice: the
    /// notice is on the launch screen, and half the presses of this never
    /// see the launch screen at all.
    fn open(&mut self, studio: &mut Studio) {
        let Some(folder) = platform::pick_folder(&t("Open a project"), &self.location) else {
            return;
        };
        let path = folder.to_string_lossy().into_owned();
        if !projects::is_project(&folder) {
            studio.notify(&tf("{0} is not a Concat project folder", &[&path]), true);
            return;
        }
        if let Err(error) = projects::open(&path).and_then(|info| studio.open_project(info)) {
            studio.notify(&error, true);
        }
    }

    /// The form after an open: at rest, and saying why when it failed.
    fn opened(&mut self, result: Result<(), String>) {
        self.busy = false;
        match result {
            Ok(()) => self.error.clear(),
            Err(error) => self.error = error,
        }
    }

    /// The form as Slint shows it.
    pub fn data(&self) -> StartData {
        let (width, height) = frame_size(self.aspect, self.size);
        let (_, num, den) = START_RATES[self.rate.min(START_RATES.len() - 1)];
        StartData {
            name: self.name.as_str().into(),
            location: self.location.as_str().into(),
            aspect: self.aspect as i32,
            size: self.size as i32,
            rate: self.rate as i32,
            size_readout: format!("{width} x {height}").into(),
            frame_aspect: width as f32 / height.max(1) as f32,
            rate_readout: format!("{num}/{den} fps").into(),
            busy: self.busy,
            error: self.error.as_str().into(),
        }
    }
}
