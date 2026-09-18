// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The launch screen's form: a new project's name, place, frame and
//! rate, and the recent list's verbs.

use concat_host::projects;

use crate::i18n::t;
use crate::platform;
use crate::studio::{RESOLUTIONS, START_RATES, Studio, home_folder};
use crate::ui::StartData;

/// Everything that can happen to the launch screen's form.
#[derive(Clone, Debug)]
pub enum StartMsg {
    NameEdited(String),
    LocationEdited(String),
    ResolutionChanged(i32),
    RateChanged(i32),
    DismissError,
    /// Pick where the project folder goes.
    Browse,
    Create,
    OpenRecent(String),
    ForgetRecent(String),
}

/// The form on the launch screen.
pub struct StartPane {
    pub name: String,
    pub location: String,
    pub resolution: usize,
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
            resolution: 0,
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
            StartMsg::ResolutionChanged(index) => {
                self.resolution = (index.max(0) as usize).min(RESOLUTIONS.len() - 1);
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
        let (_, width, height) = RESOLUTIONS[self.resolution.min(RESOLUTIONS.len() - 1)];
        let (_, num, den) = START_RATES[self.rate.min(START_RATES.len() - 1)];
        if self.location.trim().is_empty() {
            self.error = t("Choose where the project folder should go");
            return;
        }
        let opened = projects::create(&self.location, &name, width, height, num, den)
            .and_then(|info| studio.open_project(info));
        self.opened(opened);
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
        let (_, width, height) = RESOLUTIONS[self.resolution.min(RESOLUTIONS.len() - 1)];
        let (_, num, den) = START_RATES[self.rate.min(START_RATES.len() - 1)];
        StartData {
            name: self.name.as_str().into(),
            location: self.location.as_str().into(),
            resolution: self.resolution as i32,
            rate: self.rate as i32,
            size_readout: format!("{width} x {height}").into(),
            rate_readout: format!("{num}/{den} fps").into(),
            busy: self.busy,
            error: self.error.as_str().into(),
        }
    }
}
