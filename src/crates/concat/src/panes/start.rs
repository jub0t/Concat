// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The launch screen's sheet: a new project's name, place, shape, size
//! and rate, the verb that opens one that already exists, and the project
//! grid's own two verbs.

use std::path::{Path, PathBuf};

use concat_host::projects;

use crate::i18n::{t, tf};
use crate::platform;
use crate::studio::{
    ASPECTS, RATES, SIZES, Studio, custom_frame, custom_rate, fps_of, frame_size, home_folder,
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
    /// The folder the picker answered with.
    FolderChosen(String),
    Create,
    /// Open a project that already exists, picked from disk.
    Open,
    /// The folder the Open picker answered with.
    OpenChosen(PathBuf),
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
    /// Index into [`RATES`], or one past its end for a custom rate.
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
            name: UNTITLED.into(),
            location: default_location(),
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

/// Where a new project goes unless told otherwise; see [`location_on`].
/// On an iPhone the folder is made here, so that the Files app shows it
/// from the first launch and not only from the first project.
fn default_location() -> String {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let desktop = dirs::desktop_dir();
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let desktop = None;
    let location = location_on(std::env::consts::OS, desktop, home_folder);
    if cfg!(target_os = "ios")
        && !location.is_empty()
        && let Err(error) = std::fs::create_dir_all(&location)
    {
        log::warn!("could not make {location}: {error}");
    }
    location
}

/// Where a new project goes on `os`: a Concat folder on the desktop. The
/// desktop the system names, not `~/Desktop`: on Windows OneDrive often
/// moves it, and on Linux it is called what the desktop's language calls
/// it. A phone has no desk. An Android phone's projects live at the top of
/// the folder the file manager shows for the app. An iPhone's live in the
/// app's Documents, the one folder of its sandbox the Files app can show:
/// `~/Desktop` there is a folder the app is not allowed to make, and every
/// new project failed with EPERM. https://github.com/jub0t/Concat/issues/299
fn location_on(os: &str, desktop: Option<PathBuf>, home: impl Fn(&str) -> String) -> String {
    match os {
        "android" => home("Concat"),
        "ios" => home("Documents/Concat"),
        _ => desktop
            .map(|desktop| desktop.join("Concat").to_string_lossy().into_owned())
            .unwrap_or_else(|| home("Desktop/Concat")),
    }
}

/// What an unnamed project is called.
const UNTITLED: &str = "Untitled project";

/// Whether `name` is one this sheet made up rather than one somebody
/// typed: "Untitled project", or that with a number after it.
fn is_untitled(name: &str) -> bool {
    let name = name.trim();
    name.is_empty()
        || name == UNTITLED
        || name
            .strip_prefix(UNTITLED)
            .and_then(|rest| rest.strip_prefix(' '))
            .is_some_and(|number| number.parse::<u32>().is_ok_and(|n| n >= 2))
}

/// The first of "Untitled project", "Untitled project 2", ... whose folder
/// is not `taken`. A second new project in the same place used to be
/// refused as "already exists" until it was renamed by hand.
/// https://github.com/jub0t/Concat/issues/151
fn first_free_untitled(taken: impl Fn(&str) -> bool) -> String {
    std::iter::once(UNTITLED.to_owned())
        .chain((2..).map(|n| format!("{UNTITLED} {n}")))
        .find(|name| !taken(&projects::folder_name(name)))
        .expect("the numbers do not run out")
}

impl StartPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: StartMsg, studio: &mut Studio) {
        match msg {
            StartMsg::Compose => {
                self.composing = true;
                // A name somebody typed is kept for next time; one this
                // sheet made up moves on past the folders already there.
                if is_untitled(&self.name) {
                    self.name = self.free_untitled();
                }
            }
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
                let index = (index.max(0) as usize).min(RATES.len());
                if index == RATES.len() && !self.custom_rate_on() {
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
            // The picker runs with the studio unborrowed and answers as a
            // message of its own; see `host::after_dialog`.
            StartMsg::Browse => {
                let title = t("start.whereShouldProjectFolder");
                let current = self.location.clone();
                crate::host::after_dialog(
                    move || platform::pick_folder(&title, &current),
                    |studio, _, _, folder| {
                        studio.handle(crate::panes::Msg::Start(StartMsg::FolderChosen(
                            folder.to_string_lossy().into_owned(),
                        )));
                    },
                );
            }
            StartMsg::FolderChosen(folder) => self.location = folder,
            StartMsg::Create => self.create(studio),
            StartMsg::Open => self.open(),
            StartMsg::OpenChosen(folder) => Self::open_chosen(&folder, studio),
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
            self.free_untitled()
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
    ///
    /// The picker runs with the studio unborrowed, as Browse's does, and
    /// the folder comes back as [`StartMsg::OpenChosen`]; see
    /// `host::after_dialog`.
    fn open(&self) {
        let title = t("start.openAProject");
        let current = self.location.clone();
        crate::host::after_dialog(
            move || platform::pick_folder(&title, &current),
            |studio, _, _, folder| {
                studio.handle(crate::panes::Msg::Start(StartMsg::OpenChosen(folder)));
            },
        );
    }

    /// The folder Open was answered with: checked, then opened.
    fn open_chosen(folder: &Path, studio: &mut Studio) {
        let path = folder.to_string_lossy().into_owned();
        if !projects::is_project(folder) {
            studio.notify(&tf("start.notConcatProjectFolder", &[&path]), true);
            return;
        }
        if let Err(error) = projects::open(&path).and_then(|info| studio.open_project(info)) {
            studio.notify(&error, true);
        }
    }

    /// The first untitled name with no folder of that name where the sheet
    /// says the project goes. Any folder counts, not only a project: a new
    /// project is not made inside something that is already there.
    fn free_untitled(&self) -> String {
        let location = self.location.trim();
        if location.is_empty() {
            return UNTITLED.to_owned();
        }
        first_free_untitled(|folder| Path::new(location).join(folder).exists())
    }

    /// Whether the frame is typed rather than picked.
    fn custom_frame(&self) -> bool {
        self.size >= SIZES.len()
    }

    /// Whether the rate is typed rather than picked.
    fn custom_rate_on(&self) -> bool {
        self.rate >= RATES.len()
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
            let (_, num, den) = RATES[self.rate];
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
            rate_options: slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
                RATES
                    .iter()
                    .map(|(label, _, _)| slint::SharedString::from(format!("{label} fps")))
                    .chain(std::iter::once(t("common.custom").into()))
                    .collect::<Vec<_>>(),
            ))),
            busy: self.busy,
            error: self.error.as_str().into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An iPhone's projects go in its Documents, whatever a desktop would
    /// have said; a desktop's go on the desktop the system names.
    #[test]
    fn each_platform_has_a_place_for_new_projects() {
        let home = |name: &str| format!("/home/{name}");
        let desktop = || Some(PathBuf::from("/desk"));
        assert_eq!(
            location_on("ios", desktop(), home),
            "/home/Documents/Concat"
        );
        assert_eq!(location_on("ios", None, home), "/home/Documents/Concat");
        assert_eq!(location_on("android", None, home), "/home/Concat");
        assert_eq!(
            PathBuf::from(location_on("macos", desktop(), home)),
            Path::new("/desk").join("Concat")
        );
        assert_eq!(location_on("linux", None, home), "/home/Desktop/Concat");
    }

    /// The next untitled name skips every folder already there, and starts
    /// over at the plain name once that is free again.
    #[test]
    fn a_second_untitled_project_gets_a_name_of_its_own() {
        assert_eq!(first_free_untitled(|_| false), "Untitled project");
        let taken = ["Untitled project", "Untitled project 2"];
        assert_eq!(
            first_free_untitled(|folder| taken.contains(&folder)),
            "Untitled project 3"
        );
        let taken = ["Untitled project 2"];
        assert_eq!(
            first_free_untitled(|folder| taken.contains(&folder)),
            "Untitled project"
        );
    }

    /// Only a name the sheet made up is replaced; a typed one is kept.
    #[test]
    fn only_a_made_up_name_moves_on() {
        for name in [
            "",
            "  ",
            "Untitled project",
            "Untitled project 2",
            "Untitled project 41",
        ] {
            assert!(is_untitled(name), "{name:?}");
        }
        for name in [
            "My film",
            "Untitled project 1",
            "Untitled project two",
            "Untitled projects",
            "Untitled project 2 final",
        ] {
            assert!(!is_untitled(name), "{name:?}");
        }
    }

    /// Composing again after a project was made in the same place offers
    /// the next free name, not the one that would be refused.
    #[test]
    fn compose_offers_a_name_that_is_free_in_the_location() {
        let root = std::env::temp_dir().join(format!("concat-untitled-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Untitled project")).unwrap();
        let pane = StartPane {
            location: root.to_string_lossy().into_owned(),
            ..StartPane::default()
        };
        assert_eq!(pane.free_untitled(), "Untitled project 2");
        let _ = std::fs::remove_dir_all(&root);
    }
}
