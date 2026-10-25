// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The launch screen's sheet: a new project's name, place, shape, size
//! and rate, the verb that opens one that already exists, and the project
//! grid's own two verbs.

use std::path::{Path, PathBuf};

use concat_host::media::MediaSummary;
use concat_host::projects;
use concat_project::Command;
use concat_project::model::MediaKind;

use crate::host::probe_error;
use crate::i18n::{t, tf};
use crate::platform;
use crate::studio::{
    ASPECTS, FRAME_RATES, FRAME_SIDES, RATES, SIZES, Studio, custom_frame, custom_rate, fps_of,
    frame_size, home_folder,
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
    /// What shows where no picture covers the frame, `[r, g, b]`.
    CanvasChanged([u8; 3]),
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
    /// Show a project's folder in the file manager.
    RevealRecent(String),
    /// What the projects' search field holds now.
    SearchEdited(String),
    /// How the projects are ordered: [`SORT_RECENT`] or [`SORT_NAME`].
    Sort(i32),
    /// One of the learn-and-community cards, or the version line, by the
    /// name [`link`] knows it by.
    Link(String),
    /// Files or folders dropped on the launch screen: a project folder
    /// opens, and anything else starts a new project to go into.
    Dropped(Vec<PathBuf>),
    /// Files a file manager's Open with handed over at launch: a project
    /// folder opens, as a drop's does, and media goes straight into a
    /// project made in its picture's shape, with no sheet in between.
    OpenWith(Vec<PathBuf>),
    /// The files [`StartMsg::OpenWith`] handed over, probed on a worker.
    OpenWithProbed(Vec<Result<OpenedFile, String>>),
}

/// A file Open with handed over, probed: what goes in the bin, and the
/// frame and rate its picture asks of a project, when it has a picture.
#[derive(Clone, Debug)]
pub struct OpenedFile {
    summary: MediaSummary,
    frame: Option<((u32, u32), (i64, i64))>,
}

/// The projects in the order they were last opened, newest first.
pub const SORT_RECENT: i32 = 0;
/// The projects by name, A to Z.
pub const SORT_NAME: i32 = 1;

/// The recents the grid shows: those whose name or folder holds every
/// word of the query, ignoring case, newest first or by name. A missing
/// folder is found and sorted like any other.
pub fn shown<'a>(
    recents: &'a [projects::Recent],
    query: &str,
    sort: i32,
) -> Vec<&'a projects::Recent> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut shown: Vec<&projects::Recent> = recents
        .iter()
        .filter(|recent| {
            let name = recent.info.name.to_lowercase();
            let path = recent.info.path.to_lowercase();
            words
                .iter()
                .all(|word| name.contains(word.as_str()) || path.contains(word.as_str()))
        })
        .collect();
    if sort == SORT_NAME {
        shown.sort_by_cached_key(|recent| recent.info.name.to_lowercase());
    }
    shown
}

/// The official site, which the guides and the developer docs are pages of.
const SITE: &str = "https://concat.standalones.org";
/// The repository: the code, the issues, the releases.
pub const REPOSITORY: &str = "https://github.com/jub0t/Concat";

/// Where each of the launch screen's links goes, by the name the screen
/// sends. One table, so a moved page is one line here. Every address is a
/// page of the website's own content - src/content/guides in its
/// repository - or the repository's, and None for a name nobody sends.
/// "shortcuts" is not here: it opens Settings, not a page.
pub fn link(name: &str) -> Option<String> {
    Some(match name {
        "guides" => format!("{SITE}/guides/"),
        "first-edit" => format!("{SITE}/guides/first-edit/"),
        "report" => format!("{SITE}/guides/report-a-problem/"),
        "github" => REPOSITORY.to_owned(),
        // The invite the README carries.
        "discord" => "https://discord.gg/DVuPfpXfqP".to_owned(),
        // The notes for the release this build is, under the tag the
        // release workflow pushes.
        "release-notes" => format!("{REPOSITORY}/releases/tag/v{}", env!("CARGO_PKG_VERSION")),
        _ => return None,
    })
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
    /// What shows where no picture covers the frame (#456): black, as
    /// every project before it had.
    pub canvas: [u8; 3],
    pub busy: bool,
    pub error: String,
    /// The projects' search, as typed; empty shows them all.
    pub query: String,
    /// [`SORT_RECENT`] or [`SORT_NAME`].
    pub sort: i32,
    /// Media dropped on the launch screen, waiting for the project the
    /// sheet is about to make: imported into it once it opens, and let go
    /// if the sheet is put down instead.
    pub dropped: Vec<PathBuf>,
}

impl Default for StartPane {
    fn default() -> Self {
        Self {
            composing: false,
            name: untitled(),
            location: default_location(),
            aspect: 0,
            // 1080p, not the first of the three. The size everything else
            // in the app assumes, and the one a phone and a desk agree on.
            size: 1,
            rate: 3,
            custom_size: (1920, 1080),
            custom_rate: (30, 1),
            canvas: [0, 0, 0],
            busy: false,
            error: String::new(),
            query: String::new(),
            sort: SORT_RECENT,
            dropped: Vec::new(),
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

/// What an unnamed project is called, in the interface's language: the name
/// this sheet suggests, the folder it makes, and what the title bar shows
/// until the project is named.
fn untitled() -> String {
    t("common.untitledProject")
}

/// Whether `name` is one this sheet made up rather than one somebody
/// typed: "Untitled project", or that with a number after it.
fn is_untitled(name: &str) -> bool {
    let untitled = untitled();
    let name = name.trim();
    name.is_empty()
        || name == untitled.as_str()
        || name
            .strip_prefix(untitled.as_str())
            .and_then(|rest| rest.strip_prefix(' '))
            .is_some_and(|number| number.parse::<u32>().is_ok_and(|n| n >= 2))
}

/// The first of "Untitled project", "Untitled project 2", ... whose folder
/// is not `taken`. A second new project in the same place used to be
/// refused as "already exists" until it was renamed by hand.
/// https://github.com/jub0t/Concat/issues/151
fn first_free_untitled(taken: impl Fn(&str) -> bool) -> String {
    first_free(&untitled(), taken)
}

/// The first of `name`, "`name` 2", ... whose folder is not `taken`.
fn first_free(name: &str, taken: impl Fn(&str) -> bool) -> String {
    std::iter::once(name.to_owned())
        .chain((2..).map(|n| format!("{name} {n}")))
        .find(|name| !taken(&projects::folder_name(name)))
        .expect("the numbers do not run out")
}

/// The rate a project made for a picture runs at: the rate its frames come
/// at, exactly, when it is one the sheet's Custom field would take, and the
/// closest one it would take otherwise.
fn project_rate(num: i64, den: i64) -> (i64, i64) {
    let fps = num as f64 / den.max(1) as f64;
    let (low, high) = FRAME_RATES;
    if num > 0 && den > 0 && (low..=high).contains(&fps) {
        (num, den)
    } else {
        custom_rate(fps)
    }
}

/// The frame of a project made for a picture `width` x `height`: its own,
/// scaled down whole where it is larger than a project may be, so the shape
/// is kept, and held to what the sheet's Custom fields take.
fn project_frame(width: u32, height: u32) -> (u32, u32) {
    let (_, high) = FRAME_SIDES;
    let scale = (f64::from(high) / f64::from(width.max(height).max(1))).min(1.0);
    custom_frame(
        (f64::from(width) * scale) as f32,
        (f64::from(height) * scale) as f32,
    )
}

/// Probes a file Open with handed over: the bin entry an import would make
/// of it, and for a video the frame and rate of a project in its shape -
/// the size it is displayed at, rotation and all, and the rate its frames
/// come at, which for a phone or a screen recording that varies its rate
/// is the rate it records at rather than its average. A still has no rate
/// of its own and a sound no frame, so neither gives a project its shape.
fn probe_opened(path: &Path) -> Result<OpenedFile, String> {
    let info = concat_media::probe(path).map_err(concat_host::media::describe)?;
    let picture = info
        .video
        .as_ref()
        .map(|video| (video.width, video.height, video.cadence.fps()));
    let summary = MediaSummary::from(info);
    let frame =
        picture
            .filter(|_| summary.kind == MediaKind::Video)
            .map(|(width, height, rate)| {
                (
                    project_frame(width, height),
                    project_rate(rate.numerator(), rate.denominator()),
                )
            });
    Ok(OpenedFile { summary, frame })
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
                // the next time the sheet comes up; and so do the files
                // dropped to start it.
                self.error.clear();
                self.dropped.clear();
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
            StartMsg::CanvasChanged(canvas) => self.canvas = canvas,
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
                // A folder that has gone says so in words, not as the
                // manifest file it could not read.
                if !projects::is_project(Path::new(&path)) {
                    studio.notify(&tf("start.folderMissing", &[&path]), true);
                    studio.recents = projects::recents(&studio.host.dirs.config);
                    return;
                }
                if let Err(error) = projects::open(&path).and_then(|info| studio.open_project(info))
                {
                    studio.notify(&error, true);
                }
            }
            StartMsg::ForgetRecent(path) => {
                if let Err(error) = projects::forget(&studio.host.dirs.config, &path) {
                    studio.notify(&error, true);
                }
                studio.recents = projects::recents(&studio.host.dirs.config);
            }
            StartMsg::RevealRecent(path) => {
                if let Err(error) = platform::reveal(&path) {
                    studio.notify(&error, true);
                }
            }
            StartMsg::SearchEdited(query) => self.query = query,
            StartMsg::Sort(sort) => {
                self.sort = if sort == SORT_NAME {
                    SORT_NAME
                } else {
                    SORT_RECENT
                };
            }
            StartMsg::Link(name) => {
                if name == "shortcuts" {
                    use crate::panes::settings::{SHORTCUTS_PAGE, SettingsMsg};
                    studio.handle(crate::panes::Msg::Settings(SettingsMsg::Open));
                    studio.handle(crate::panes::Msg::Settings(SettingsMsg::PageChanged(
                        SHORTCUTS_PAGE,
                    )));
                    return;
                }
                let Some(url) = link(&name) else {
                    log::warn!("start: no link is called {name:?}");
                    return;
                };
                if let Err(error) = platform::open_link(&url) {
                    studio.notify(&tf("start.couldNotOpenLink", &[&url, &error]), true);
                }
            }
            StartMsg::Dropped(paths) => self.dropped_on(paths, studio),
            StartMsg::OpenWith(paths) => self.open_with(paths, studio),
            StartMsg::OpenWithProbed(probed) => self.open_probed(probed, studio),
        }
    }

    /// Files handed over by Open with. A project folder among them opens,
    /// as a drop's does; media is probed on a worker, and held as what was
    /// dropped meanwhile, so a drop while it runs is ignored as it is while
    /// a project is being made.
    fn open_with(&mut self, paths: Vec<PathBuf>, studio: &mut Studio) {
        if self.busy {
            return;
        }
        if paths.iter().any(|path| projects::is_project(path)) {
            self.dropped_on(paths, studio);
            return;
        }
        let media: Vec<PathBuf> = paths.into_iter().filter(|path| path.is_file()).collect();
        if media.is_empty() {
            studio.notify(&t("start.dropNothingToOpen"), true);
            return;
        }
        self.busy = true;
        self.dropped = media.clone();
        crate::host::spawn(
            move || {
                media
                    .iter()
                    .map(|path| probe_opened(path))
                    .collect::<Vec<_>>()
            },
            |studio, _, _, probed| {
                studio.handle(crate::panes::Msg::Start(StartMsg::OpenWithProbed(probed)));
            },
        );
    }

    /// The files Open with handed over, probed. With a picture among them,
    /// a project in the shape of the first is made where the sheet would
    /// make one, named after its file, and opened with every file in the
    /// bin and laid end to end on the timeline. Without one there is no
    /// shape to go by, and they wait on the sheet as a drop's do; that is
    /// also where a project that could not be made says why.
    fn open_probed(&mut self, probed: Vec<Result<OpenedFile, String>>, studio: &mut Studio) {
        self.busy = false;
        let files = std::mem::take(&mut self.dropped);
        if studio.session.is_some() {
            // A project was opened from the launch screen meanwhile.
            return;
        }
        if !probed.iter().any(Result::is_ok) {
            if let Some(Err(error)) = probed.first() {
                studio.notify(&probe_error(error), true);
            }
            return;
        }
        let Some((path, ((width, height), (num, den)))) = probed
            .iter()
            .flatten()
            .find_map(|file| Some((&file.summary.path, file.frame?)))
        else {
            self.dropped_on(files, studio);
            return;
        };
        let location = self.location.trim().to_owned();
        if location.is_empty() {
            self.dropped_on(files, studio);
            return;
        }
        let stem = Path::new(path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().trim().to_owned())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(untitled);
        let name = first_free(&stem, |folder| Path::new(&location).join(folder).exists());
        let opened = projects::create(&location, &name, width, height, num, den, self.canvas)
            .and_then(|info| studio.open_project(info));
        if let Err(error) = opened {
            self.dropped_on(files, studio);
            self.error = error;
            return;
        }
        let mut commands = Vec::new();
        let mut failures = Vec::new();
        for result in probed {
            match result {
                Ok(file) => commands.push(Command::AddMedia {
                    item: file.summary.to_new_media(),
                }),
                Err(error) => failures.push(error),
            }
        }
        studio.apply(Command::Batch { commands });
        // The project is new, so everything in its bin came from here.
        let media_ids = studio
            .project()
            .media
            .iter()
            .map(|item| item.id.clone())
            .collect();
        studio.apply(Command::AddClipsInSequence {
            media_ids,
            track_id: None,
            start: 0.0,
            ripple: false,
        });
        if let Some(error) = failures.first() {
            studio.notify(&probe_error(error), true);
        }
    }

    /// Files or folders dropped on the launch screen. A project folder
    /// among them opens, the first if there are several, as Open would;
    /// anything else is media, and puts up the new-project sheet with the
    /// files waiting to go into the project it makes.
    fn dropped_on(&mut self, paths: Vec<PathBuf>, studio: &mut Studio) {
        if self.busy {
            return;
        }
        if let Some(folder) = paths.iter().find(|path| projects::is_project(path)) {
            self.dropped.clear();
            self.composing = false;
            Self::open_chosen(folder, studio);
            return;
        }
        let media: Vec<PathBuf> = paths.into_iter().filter(|path| path.is_file()).collect();
        if media.is_empty() {
            studio.notify(&t("start.dropNothingToOpen"), true);
            return;
        }
        // A second drop while the sheet is up adds to the first.
        for path in media {
            if !self.dropped.contains(&path) {
                self.dropped.push(path);
            }
        }
        if !self.composing {
            self.update(StartMsg::Compose, studio);
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
        let opened = projects::create(&self.location, &name, width, height, num, den, self.canvas)
            .and_then(|info| studio.open_project(info));
        self.busy = false;
        match opened {
            Ok(()) => {
                studio.prefs.last_project_location = Some(self.location.clone());
                studio.prefs.save(&studio.host.dirs);
                self.composing = false;
                self.error.clear();
                // What was dropped to start it goes in, the way an import
                // does: probed on a worker, with its own notice.
                let dropped = std::mem::take(&mut self.dropped);
                if !dropped.is_empty() {
                    studio.handle(crate::panes::Msg::Media(
                        crate::panes::media_bin::MediaMsg::Import(dropped),
                    ));
                }
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
            return untitled();
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
            canvas: slint::Color::from_rgb_u8(self.canvas[0], self.canvas[1], self.canvas[2]),
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
            dropped: self.dropped.len() as i32,
            query: self.query.as_str().into(),
            sort: self.sort,
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
        // The name is the interface's, so the test holds English while it
        // reads it: another test may choose a language at any moment.
        let _english = crate::i18n::language_lock();
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

    /// A project opened from a file is named after it, and numbered past
    /// the folders already there, by the folder its name would make.
    #[test]
    fn a_project_from_a_file_gets_a_name_of_its_own() {
        assert_eq!(first_free("Holiday", |_| false), "Holiday");
        let taken = ["Holiday", "Holiday 2"];
        assert_eq!(
            first_free("Holiday", |folder| taken.contains(&folder)),
            "Holiday 3"
        );
        let taken = ["Take- one"];
        assert_eq!(
            first_free("Take: one", |folder| taken.contains(&folder)),
            "Take: one 2"
        );
    }

    /// A picture's rate is the project's, exactly, where the sheet would
    /// take it, and the nearest one it would take where not.
    #[test]
    fn a_project_runs_at_its_picture_s_rate() {
        assert_eq!(project_rate(30, 1), (30, 1));
        assert_eq!(project_rate(30000, 1001), (30000, 1001));
        assert_eq!(project_rate(120, 1), (120, 1));
        let (low, high) = FRAME_RATES;
        assert_eq!(project_rate(1, 2), custom_rate(low));
        assert_eq!(project_rate(1000, 1), custom_rate(high));
    }

    /// A picture's frame is the project's, kept to even sides, and scaled
    /// down whole rather than cut to a side where it is too large.
    #[test]
    fn a_project_takes_its_picture_s_frame() {
        assert_eq!(project_frame(1080, 1920), (1080, 1920));
        assert_eq!(project_frame(1081, 1921), (1082, 1922));
        assert_eq!(project_frame(10240, 4320), (8192, 3456));
        assert_eq!(project_frame(4320, 10240), (3456, 8192));
        let (low, _) = FRAME_SIDES;
        assert_eq!(project_frame(2, 2), (low, low));
    }

    /// A probed file gives the frame it is displayed at - wide pixels
    /// stretched square - and the rate its frames come at, not the average
    /// a variable-rate recording that holds still comes out at.
    #[test]
    fn a_probed_file_gives_its_picture_s_frame_and_rate() {
        let fixture = |name: &str| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../concat-media/fixtures")
                .join(name)
        };
        let hdv = probe_opened(&fixture("wide-pixels-hdv.m2ts")).expect("probes");
        assert_eq!(hdv.frame, Some(((64, 32), (25, 1))));
        let still = probe_opened(&fixture("vfr-still-screen.mp4")).expect("probes");
        assert_eq!(still.frame, Some(((32, 32), (30, 1))));
        assert!(still.summary.video.is_some());
        assert!(probe_opened(&fixture("no-such-file.mp4")).is_err());
    }

    /// A still goes in the bin but gives a project no shape: it has no rate
    /// of its own, and a photograph's frame is not a video's.
    #[test]
    fn a_still_gives_no_project_its_shape() {
        let root = std::env::temp_dir().join(format!("concat-still-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("still.png");
        image::RgbImage::new(64, 48).save(&path).unwrap();
        let still = probe_opened(&path).expect("probes");
        assert_eq!(still.summary.kind, MediaKind::Image);
        assert_eq!(still.frame, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The search finds by every word, in the name or the folder, and the
    /// sort by name ignores case; the list's own order is the recent one.
    #[test]
    fn the_projects_search_and_sort() {
        let recent = |name: &str, path: &str| projects::Recent {
            info: projects::ProjectInfo {
                canvas: [0, 0, 0],
                path: path.into(),
                name: name.into(),
                width: 1920,
                height: 1080,
                rate_num: 30,
                rate_den: 1,
                opened_at: 0,
            },
            present: true,
            edited_at: None,
        };
        let list = [
            recent("beach vlog", "/v/summer/beach"),
            recent("Podcast 12", "/v/pods/12"),
            recent("Apple ad", "/v/clients/apple"),
        ];
        let names = |shown: Vec<&projects::Recent>| -> Vec<String> {
            shown
                .iter()
                .map(|recent| recent.info.name.clone())
                .collect()
        };
        assert_eq!(
            names(shown(&list, "", SORT_RECENT)),
            ["beach vlog", "Podcast 12", "Apple ad"]
        );
        assert_eq!(
            names(shown(&list, "", SORT_NAME)),
            ["Apple ad", "beach vlog", "Podcast 12"]
        );
        assert_eq!(names(shown(&list, "SUMMER", SORT_RECENT)), ["beach vlog"]);
        assert_eq!(names(shown(&list, "pod 12", SORT_RECENT)), ["Podcast 12"]);
        assert!(shown(&list, "nothing", SORT_RECENT).is_empty());
    }

    /// Every link the launch screen sends goes somewhere, and the release
    /// notes are this build's tag.
    #[test]
    fn every_link_has_an_address() {
        for name in [
            "guides",
            "first-edit",
            "report",
            "github",
            "discord",
            "release-notes",
        ] {
            assert!(
                link(name).is_some_and(|url| url.starts_with("https://")),
                "{name}"
            );
        }
        assert_eq!(
            link("release-notes").unwrap(),
            format!("{REPOSITORY}/releases/tag/v{}", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(link("nowhere"), None);
    }

    /// Only a name the sheet made up is replaced; a typed one is kept.
    #[test]
    fn only_a_made_up_name_moves_on() {
        let _english = crate::i18n::language_lock();
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
        let _english = crate::i18n::language_lock();
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
