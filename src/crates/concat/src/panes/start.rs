// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The launch screen's sheet: a new project's name, place, shape, size
//! and rate, the verb that opens one that already exists, and the project
//! grid's own two verbs.

use concat_host::projects;

use crate::i18n::{t, tf};
use crate::platform;
use crate::studio::{
    ASPECTS, SIZES, START_RATES, Studio, custom_frame, custom_rate, fps_of, frame_size, home_folder,
};
use crate::ui::StartData;

/// Everything that can happen to the launch screen's sheet.
#[derive(Clone, Debug)]
pub enum StartMsg {
    /// Put up the sheet a new project is described on.
    Compose,
    /// Take it down, keeping what was typed for next time.
    Dismiss,
    NameEdited(String),
    LocationEdited(String),
    /// The frame's shape: 16:9, 9:16, 1:1, 4:3.
    AspectChanged(i32),
    /// The frame's size, as the short edge: 720p, 1080p, 4K.
    SizeChanged(i32),
    RateChanged(i32),
    /// The custom frame's sides and rate, as typed.
    CustomWidth(f32),
    CustomHeight(f32),
    CustomFps(f32),
    DismissError,
    /// Pick where the project folder goes.
    Browse,
    Create,
    /// Open a project that already exists, picked from disk.
    Open,
    OpenRecent(String),
    ForgetRecent(String),
}

/// The sheet on the launch screen.
pub struct StartPane {
    /// The sheet is up. Down again once the project it describes opens,
    /// and not before: a create that fails keeps the form, and its notice,
    /// on screen.
    pub composing: bool,
    pub name: String,
    pub location: String,
    /// Index into [`ASPECTS`].
    pub aspect: usize,
    /// Index into [`SIZES`], or one past its end for a custom frame.
    pub size: usize,
    /// Index into [`START_RATES`], or one past its end for a custom rate.
    pub rate: usize,
    /// The custom frame, even on both sides; seeded from the preset that
    /// was picked when Custom was, so the fields start from a real frame.
    pub custom_size: (u32, u32),
    /// The custom rate, as an exact fraction.
    pub custom_rate: (i64, i64),
    pub busy: bool,
    pub error: String,
}

impl Default for StartPane {
    fn default() -> Self {
        Self {
            composing: false,
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
            custom_size: (1920, 1080),
            custom_rate: (30, 1),
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
            StartMsg::Compose => self.composing = true,
            StartMsg::Dismiss => {
                self.composing = false;
                // The notice belongs to the attempt it reported on, not to
                // the next time the sheet comes up.
                self.error.clear();
            }
            StartMsg::NameEdited(name) => self.name = name,
            StartMsg::LocationEdited(path) => self.location = path,
            StartMsg::AspectChanged(index) => {
                self.aspect = (index.max(0) as usize).min(ASPECTS.len() - 1);
            }
            StartMsg::SizeChanged(index) => {
                let index = (index.max(0) as usize).min(SIZES.len());
                if index == SIZES.len() && !self.custom_frame() {
                    self.custom_size = self.frame();
                }
                self.size = index;
            }
            StartMsg::RateChanged(index) => {
                let index = (index.max(0) as usize).min(START_RATES.len());
                if index == START_RATES.len() && !self.custom_rate_on() {
                    self.custom_rate = self.rate();
                }
                self.rate = index;
            }
            StartMsg::CustomWidth(width) => {
                self.custom_size = custom_frame(width, self.custom_size.1 as f32);
            }
            StartMsg::CustomHeight(height) => {
                self.custom_size = custom_frame(self.custom_size.0 as f32, height);
            }
            StartMsg::CustomFps(fps) => self.custom_rate = custom_rate(f64::from(fps)),
            StartMsg::DismissError => self.error.clear(),
            StartMsg::Browse => {
                if let Some(folder) =
                    platform::pick_folder(&t("start.whereShouldProjectFolder"), &self.location)
                {
                    self.location = folder.to_string_lossy().into_owned();
                }
            }
            StartMsg::Create => self.create(studio),
            StartMsg::Open => self.open(studio),
            // Both of the grid's verbs report through the toast, as Open
            // does: the sheet is down while a card is pressed, and a notice
            // inside it would go unseen.
            StartMsg::OpenRecent(path) => {
                if let Err(error) = projects::open(&path).and_then(|info| studio.open_project(info))
                {
                    studio.notify(&error, true);
                }
            }
            StartMsg::ForgetRecent(path) => {
                if let Err(error) = projects::forget(&studio.host.dirs.config, &path) {
                    studio.notify(&error, true);
                }
                studio.recents = projects::list(&studio.host.dirs.config);
            }
        }
    }

    /// Makes the project the sheet describes and opens it. The sheet comes
    /// down with the project open behind it; a failure leaves it up, with
    /// the reason in its notice.
    fn create(&mut self, studio: &mut Studio) {
        let name = self.name.trim().to_owned();
        let name = if name.is_empty() {
            "Untitled project".to_owned()
        } else {
            name
        };
        let (width, height) = self.frame();
        let (num, den) = self.rate();
        if self.location.trim().is_empty() {
            self.error = t("start.chooseWhereProjectFolder");
            return;
        }
        let opened = projects::create(&self.location, &name, width, height, num, den)
            .and_then(|info| studio.open_project(info));
        self.busy = false;
        match opened {
            Ok(()) => {
                self.composing = false;
                self.error.clear();
            }
            Err(error) => self.error = error,
        }
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
    /// through the toast rather than through the sheet's notice: the sheet
    /// is down when this is pressed, and half the presses of this never see
    /// the launch screen at all.
    fn open(&mut self, studio: &mut Studio) {
        let Some(folder) = platform::pick_folder(&t("start.openAProject"), &self.location) else {
            return;
        };
        let path = folder.to_string_lossy().into_owned();
        if !projects::is_project(&folder) {
            studio.notify(&tf("start.notConcatProjectFolder", &[&path]), true);
            return;
        }
        if let Err(error) = projects::open(&path).and_then(|info| studio.open_project(info)) {
            studio.notify(&error, true);
        }
    }

    /// Whether the frame is typed rather than picked.
    fn custom_frame(&self) -> bool {
        self.size >= SIZES.len()
    }

    /// Whether the rate is typed rather than picked.
    fn custom_rate_on(&self) -> bool {
        self.rate >= START_RATES.len()
    }

    /// The frame the sheet describes: the typed one, or the shape at the
    /// size.
    fn frame(&self) -> (u32, u32) {
        if self.custom_frame() {
            self.custom_size
        } else {
            frame_size(self.aspect, self.size)
        }
    }

    /// The rate the sheet describes, as an exact fraction.
    fn rate(&self) -> (i64, i64) {
        if self.custom_rate_on() {
            self.custom_rate
        } else {
            let (_, num, den) = START_RATES[self.rate];
            (num, den)
        }
    }

    /// The sheet as Slint shows it.
    pub fn data(&self) -> StartData {
        let (width, height) = self.frame();
        let (num, den) = self.rate();
        StartData {
            composing: self.composing,
            name: self.name.as_str().into(),
            location: self.location.as_str().into(),
            aspect: self.aspect as i32,
            size: self.size as i32,
            rate: self.rate as i32,
            size_readout: format!("{width} x {height}").into(),
            custom_width: width as f32,
            custom_height: height as f32,
            custom_fps: fps_of(num, den) as f32,
            frame_aspect: width as f32 / height.max(1) as f32,
            rate_readout: format!("{num}/{den} fps").into(),
            busy: self.busy,
            error: self.error.as_str().into(),
        }
    }
}
