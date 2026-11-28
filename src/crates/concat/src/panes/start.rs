// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The launch screen's sheet: a new project's name, place, shape, size
//! and rate, the verb that opens one that already exists, and the project
//! grid's own two verbs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use concat_host::projects;
use concat_project::commands::{ClipPatch, Command, NewMedia, TrimEdge};
use concat_project::model::{
    ClipMotion, ClipMove, KeyEase, KeyProperty, MediaKind, MediaOrigin,
};

use crate::i18n::{t, tf};
use crate::panes::threads::ThreadsBuildOptions;
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
    /// What shows where no picture covers the frame, `[r, g, b, a]`.
    CanvasChanged([u8; 4]),
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
    /// Make a project of a CapCut draft, picked from CapCut's drafts.
    ImportCapcut,
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
}

/// The projects in the order they were last opened, newest first.
pub const SORT_RECENT: i32 = 0;
/// The projects by name, A to Z.
pub const SORT_NAME: i32 = 1;

#[derive(Clone)]
struct SavedProjectSetup {
    name: String,
    aspect: usize,
    size: usize,
    rate: usize,
    custom_size: (u32, u32),
    custom_rate: (i64, i64),
}

struct ThreadsProjectSetup {
    options: ThreadsBuildOptions,
    saved: SavedProjectSetup,
}

const THREADS_MEME_DURATION: f64 = 1.5;
const CONCAT_DEFAULT_IMAGE_DURATION: f64 = 5.0;

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
        "report" => format!("{SITE}/guides/report-a-problem/"),
        "github" => REPOSITORY.to_owned(),
        // The gradient card: this version's "What's new" post on the
        // website's blog, the polished telling of its release notes.
        "news" => format!(
            "{SITE}/blog/whats-new-in-{}/",
            env!("CARGO_PKG_VERSION").replace('.', "-")
        ),
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
    pub canvas: [u8; 4],
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
    /// A Threads extraction waiting for the regular project sheet to finish.
    threads_setup: Option<ThreadsProjectSetup>,
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
            canvas: [0, 0, 0, 255],
            busy: false,
            error: String::new(),
            query: String::new(),
            sort: SORT_RECENT,
            dropped: Vec::new(),
            threads_setup: None,
        }
    }
}

/// Where a new project goes unless told otherwise; see [`location_on`].
/// On an iPhone the folder is made here, so that the Files app shows it
/// from the first launch and not only from the first project.
fn default_location() -> String {
    #[cfg(not(target_os = "ios"))]
    let desktop = dirs::desktop_dir();
    #[cfg(target_os = "ios")]
    let desktop = None;
    let location = location_on(std::env::consts::OS, desktop, home_folder);
    if cfg!(target_os = "ios")
        && !location.is_empty()
        && let Err(error) = std::fs::create_dir_all(&location)
    {
        log::warn!(
            "project: wanted the new-project folder {location} and could not make it: {error}; the first project makes it"
        );
    }
    location
}

/// Where a new project goes on `os`: a Concat folder on the desktop. The
/// desktop the system names, not `~/Desktop`: on Windows OneDrive often
/// moves it, and on Linux it is called what the desktop's language calls
/// it. A phone has no desk: an iPhone's live in the
/// app's Documents, the one folder of its sandbox the Files app can show:
/// `~/Desktop` there is a folder the app is not allowed to make, and every
/// new project failed with EPERM. https://github.com/jub0t/Concat/issues/299
fn location_on(os: &str, desktop: Option<PathBuf>, home: impl Fn(&str) -> String) -> String {
    match os {
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
    let untitled = untitled();
    std::iter::once(untitled.clone())
        .chain((2..).map(|n| format!("{untitled} {n}")))
        .find(|name| !taken(&projects::folder_name(name)))
        .expect("the numbers do not run out")
}

impl StartPane {
    /// Opens the normal new-project sheet with vertical video defaults and
    /// remembers the user's previous choices so the quick flow does not
    /// change later project creation.
    pub fn begin_threads_creation(&mut self, options: ThreadsBuildOptions) {
        let saved = SavedProjectSetup {
            name: self.name.clone(),
            aspect: self.aspect,
            size: self.size,
            rate: self.rate,
            custom_size: self.custom_size,
            custom_rate: self.custom_rate,
        };
        let title = options.extraction.title.trim();
        self.name = if title.is_empty() {
            self.free_untitled()
        } else {
            title.to_owned()
        };
        self.aspect = ASPECTS
            .iter()
            .position(|(name, _, _)| *name == "9:16")
            .unwrap_or(0);
        self.size = SIZES
            .iter()
            .position(|(name, _)| *name == "1080p")
            .unwrap_or(1);
        self.rate = RATES
            .iter()
            .position(|(name, _, _)| *name == "30")
            .unwrap_or(6);
        self.composing = true;
        self.error.clear();
        self.dropped.clear();
        self.threads_setup = Some(ThreadsProjectSetup { options, saved });
    }

    fn restore_project_setup(&mut self, saved: SavedProjectSetup) {
        self.name = saved.name;
        self.aspect = saved.aspect;
        self.size = saved.size;
        self.rate = saved.rate;
        self.custom_size = saved.custom_size;
        self.custom_rate = saved.custom_rate;
    }

    fn place_threads_frames(
        options: &ThreadsBuildOptions,
        project_path: &Path,
        studio: &mut Studio,
    ) {
        let extraction = &options.extraction;
        let voiceovers = &options.voiceovers;
        let video = studio.project().active().video;
        let frame_w = video.width.max(360);
        let frame_h = video.height.max(640);
        let frame_rate = (video.rate_num, video.rate_den.max(1));

        let media_dir = project_path.join("media");
        let audio_dir = project_path.join("audio");
        let media_dir_ready = match std::fs::create_dir_all(&media_dir) {
            Ok(()) => true,
            Err(error) => {
                studio.notify(&format!("Could not create the media folder: {error}"), true);
                false
            }
        };
        let audio_dir_ready = match std::fs::create_dir_all(&audio_dir) {
            Ok(()) => true,
            Err(error) => {
                studio.notify(
                    &format!("Could not create the voiceover folder: {error}"),
                    true,
                );
                false
            }
        };

        // Import deduplicated per-scene meme images into the project media folder.
        let mut meme_ids: HashMap<PathBuf, Option<String>> = HashMap::new();
        let mut requested_memes = 0usize;
        let mut placed_memes = 0usize;
        if media_dir_ready {
            for frame in &extraction.frames {
                let Some(source) = frame.meme_path.as_ref() else {
                    continue;
                };
                requested_memes += 1;
                if meme_ids.contains_key(source) {
                    continue;
                }
                let idx = meme_ids.len() + 1;
                let id = import_threads_meme(source, idx, &media_dir, studio);
                meme_ids.insert(source.clone(), id);
            }
        }

        // Render and import vertical gradient backdrop for bottom track T1.
        let backdrop_rgb = if media_dir_ready {
            render_backdrop_rgb(options.card_theme, frame_w, frame_h)
        } else {
            None
        };
        let backdrop_media_id = backdrop_rgb
            .as_ref()
            .and_then(|rgb| save_and_import_jpeg(rgb, "threads-backdrop.jpg", &media_dir, studio));

        let fonts = concat_text::Fonts::new();
        let mut at = 0.0f64;
        let mut last_card_clip: Option<String> = None;
        let mut cards_placed = 0usize;
        let mut audio_clips = Vec::with_capacity(extraction.frames.len());
        let mut audio_count = 0usize;
        let mut speech_spans: Vec<(f64, f64)> = Vec::with_capacity(extraction.frames.len());
        let mut karaoke_scenes: Vec<(String, f64, f64, Vec<crate::edge_tts::WordTiming>)> =
            Vec::with_capacity(extraction.frames.len());

        for (index, frame) in extraction.frames.iter().enumerate() {
            let mut audio_duration = voiceovers
                .get(index)
                .map(|audio| audio.duration_seconds)
                .unwrap_or_default();
            let mut voice_media_id = None;
            if audio_dir_ready && let Some(audio) = voiceovers.get(index) {
                let path = audio_dir.join(format!("threads-frame-{}.mp3", index + 1));
                match std::fs::write(&path, &audio.bytes) {
                    Ok(()) => match concat_host::media::probe(&path.to_string_lossy()) {
                        Ok(summary) => {
                            audio_duration = summary
                                .duration
                                .unwrap_or(audio_duration)
                                .max(audio_duration);
                            let mut item = summary.to_new_media();
                            item.origin = Some(MediaOrigin::Speech);
                            voice_media_id = import_media_item(item, &summary.path, studio);
                        }
                        Err(error) => log::error!(
                            "threads: could not read generated voiceover for {}: {error}",
                            frame.label
                        ),
                    },
                    Err(error) => log::error!(
                        "threads: could not save generated voiceover for {}: {error}",
                        frame.label
                    ),
                }
            }

            let scene_duration = audio_duration.max(3.5) + 0.35;
            speech_spans.push((at, at + scene_duration));
            let spoken_text = if frame.text.trim().is_empty() {
                format!("{} — {}", frame.label, frame.author)
            } else {
                frame.text.trim().to_owned()
            };
            let word_timings = voiceovers
                .get(index)
                .map(|audio| audio.word_timings.clone())
                .unwrap_or_default();
            karaoke_scenes.push((
                spoken_text,
                at,
                audio_duration.max(1.0),
                word_timings,
            ));

            // Render authentic Threads mock post card PNG on T2 with subtle pop entrance.
            if media_dir_ready {
                let replying_to = if !frame.is_post
                    && !frame.id.starts_with("custom-")
                    && !extraction.post_frame.username.trim().is_empty()
                {
                    format!(
                        "{} @{}",
                        t("threads.replyingTo"),
                        extraction.post_frame.username.trim()
                    )
                } else {
                    String::new()
                };
                let avatar = frame.avatar_bytes.as_deref().and_then(decode_avatar_pixels);
                let media = frame.media_bytes.as_deref().and_then(decode_avatar_pixels);
                let spec = concat_text::threads_card::ThreadsCardSpec {
                    is_post: frame.is_post,
                    author_name: frame.author.clone(),
                    author_handle: frame.username.clone(),
                    timestamp: frame.timestamp.clone(),
                    verified: frame.verified,
                    badge: frame.label.clone(),
                    replying_to,
                    text: frame.text.clone(),
                    likes: frame.likes.clone(),
                    replies: frame.replies.clone(),
                    reposts: frame.reposts.clone(),
                    avatar,
                    media,
                    media_is_video: frame.media_is_video,
                    theme: options.card_theme,
                };
                if let Some(card_media_id) = render_and_import_card(
                    &fonts,
                    &spec,
                    backdrop_rgb.as_ref(),
                    index + 1,
                    frame_w,
                    frame_h,
                    &media_dir,
                    studio,
                ) && let Some(clip_id) = place_still_clip(
                    studio,
                    card_media_id,
                    "T2",
                    at,
                    scene_duration,
                    Some(0.25),
                ) {
                    last_card_clip = Some(clip_id);
                    cards_placed += 1;
                }
            }

            if let Some(media_id) = voice_media_id {
                audio_count += 1;
                audio_clips.push(Command::AddClip {
                    media_id,
                    track_id: "T3".to_owned(),
                    start: at,
                    ripple: false,
                });
            }

            if let Some(source) = frame.meme_path.as_ref() {
                let meme_media = meme_ids.get(source).and_then(Option::as_ref).cloned();
                if let Some(media_id) = meme_media {
                    if place_still_clip(
                        studio,
                        media_id,
                        "T2",
                        at + scene_duration,
                        THREADS_MEME_DURATION,
                        Some(0.18),
                    )
                    .is_some()
                    {
                        placed_memes += 1;
                    }
                    at += scene_duration + THREADS_MEME_DURATION;
                } else {
                    at += scene_duration;
                }
            } else {
                at += scene_duration;
            }
        }

        let total_duration = at.max(1.0);
        if let Some(media_id) = backdrop_media_id {
            let _ = place_still_clip(studio, media_id, "T1", 0.0, total_duration, None);
        }

        if let Some(last_clip) = last_card_clip {
            studio.selection = vec![last_clip];
        }

        let voice_clips_added = if !audio_clips.is_empty() {
            studio
                .apply(Command::Batch {
                    commands: audio_clips,
                })
                .is_some()
        } else {
            false
        };

        // 6.2 Auto-BGM loop on T4 with fade-in/out and ducking under voiceovers.
        if audio_dir_ready && options.bgm_preset != crate::threads_bgm::BgmPreset::Off {
            place_threads_bgm(options, total_duration, &speech_spans, &audio_dir, studio);
        }

        // 7.1 Dynamic word-by-word karaoke subtitles on a dedicated top track.
        if options.karaoke_style != crate::threads_karaoke::KaraokeStyle::Off
            && !karaoke_scenes.is_empty()
            && let Some(karaoke_track_id) = studio.apply(Command::AddTrack { bottom: false })
        {
            let mut karaoke_commands = Vec::new();
            for (text, scene_start, dur, timings) in &karaoke_scenes {
                karaoke_commands.extend(
                    crate::threads_karaoke::build_scene_karaoke_commands(
                        &karaoke_track_id,
                        text,
                        *scene_start,
                        *dur,
                        timings,
                        options.karaoke_style,
                    ),
                );
            }
            if !karaoke_commands.is_empty() {
                let _ = studio.apply(Command::Batch {
                    commands: karaoke_commands,
                });
            }
        }

        if cards_placed == extraction.frames.len()
            && voice_clips_added
            && audio_count == extraction.frames.len()
        {
            studio.notify(&t("threads.voiceoversAdded"), false);
        } else {
            studio.notify(&t("threads.voiceoversImportPartial"), true);
        }
        if requested_memes > 0 {
            if placed_memes == requested_memes {
                studio.notify(&t("threads.memeAdded"), false);
            } else {
                studio.notify(&t("threads.memeImportPartial"), true);
            }
        }
        studio.export.apply_tiktok_preset_for_rate(frame_rate);
    }

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
                if let Some(setup) = self.threads_setup.take() {
                    self.restore_project_setup(setup.saved);
                }
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
            StartMsg::OpenChosen(folder) => Self::open_chosen(&folder, &self.location, studio),
            StartMsg::ImportCapcut => {
                let title = t("start.importCapcut");
                let drafts = concat_host::capcut::drafts_folder()
                    .map(|folder| folder.to_string_lossy().into_owned())
                    .unwrap_or_default();
                crate::host::after_dialog(
                    move || platform::pick_folder(&title, &drafts),
                    |studio, _, _, folder| {
                        studio.handle(crate::panes::Msg::Start(StartMsg::OpenChosen(folder)));
                    },
                );
            }
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
                if let Err(error) = studio.open_project_at(&path) {
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
                    log::warn!(
                        "start: the launch screen asked for a link called {name:?}, which there is none of; nothing opened"
                    );
                    return;
                };
                if let Err(error) = platform::open_link(&url) {
                    studio.notify(&tf("start.couldNotOpenLink", &[&url, &error]), true);
                }
            }
            StartMsg::Dropped(paths) => self.dropped_on(paths, studio),
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
        if let Some(folder) = paths
            .iter()
            .find(|path| projects::is_project(path) || concat_host::capcut::is_draft(path))
        {
            self.dropped.clear();
            self.composing = false;
            Self::open_chosen(folder, &self.location, studio);
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
            .inspect_err(|error| {
                log::error!(
                    "project: making {name} failed: {error}; the new-project sheet stays up"
                );
            })
            .and_then(|info| {
                let path = info.path.clone();
                studio.open_project(info).map(|()| PathBuf::from(path))
            });
        self.busy = false;
        match opened {
            Ok(project_path) => {
                if let Some(setup) = self.threads_setup.take() {
                    self.restore_project_setup(setup.saved);
                    Self::place_threads_frames(&setup.options, &project_path, studio);
                }
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

    /// The folder Open was answered with: checked, then opened. A CapCut
    /// draft is made a project of first, under `location` - where a new
    /// project goes - and the draft itself is left as it was (#477).
    fn open_chosen(folder: &Path, location: &str, studio: &mut Studio) {
        let path = folder.to_string_lossy().into_owned();
        if !projects::is_project(folder) && concat_host::capcut::is_draft(folder) {
            Self::import_capcut(folder, location, studio);
            return;
        }
        if !projects::is_project(folder) {
            studio.notify(&tf("start.notConcatProjectFolder", &[&path]), true);
            return;
        }
        if let Err(error) = studio.open_project_at(&path) {
            studio.notify(&error, true);
        }
    }

    /// Makes a project of the CapCut draft in `folder` and opens it, saying
    /// how much of it came across: what did not is named in the log.
    fn import_capcut(folder: &Path, location: &str, studio: &mut Studio) {
        let imported = concat_host::capcut::import(folder, location);
        match imported {
            Ok(import) => {
                let left = import.notes.len();
                if let Err(error) = studio.open_project(import.project) {
                    studio.notify(&error, true);
                    return;
                }
                let said = if left == 0 {
                    t("start.capcutImported")
                } else {
                    tf("start.capcutImportedSome", &[&left])
                };
                studio.notify(&said, false);
            }
            Err(error) => {
                log::error!(
                    "project: making a project of the draft {} failed: {error}; nothing was made",
                    folder.display()
                );
                studio.notify(&tf("start.capcutImportFailed", &[&error]), true);
            }
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
            canvas: slint::Color::from_argb_u8(
                self.canvas[3],
                self.canvas[0],
                self.canvas[1],
                self.canvas[2],
            ),
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

    /// The search finds by every word, in the name or the folder, and the
    /// sort by name ignores case; the list's own order is the recent one.
    #[test]
    fn the_projects_search_and_sort() {
        let recent = |name: &str, path: &str| projects::Recent {
            info: projects::ProjectInfo {
                canvas: [0, 0, 0, 255],
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
            "report",
            "github",
            "discord",
            "news",
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

    #[test]
    fn ducking_keyframes_ramp_down_on_speech_and_lift_during_meme_gap() {
        let speech = vec![(0.0, 4.0), (5.5, 9.0)];
        let keys = build_ducking_keyframes(0.0, 10.0, &speech, 0.06, 0.22);
        assert!(keys.len() >= 4);
        assert!((keys.first().unwrap().0 - 0.0).abs() < 1e-6);
        assert!((keys.first().unwrap().1 - 0.06).abs() < 1e-6);
        // Inside the [4.0, 5.5] meme break, gain lifts to 0.22.
        assert!(
            keys.iter()
                .any(|&(at, val)| at > 0.40 && at < 0.55 && (val - 0.22).abs() < 1e-6)
        );
        // At 0.70 (inside second speech), gain is ducked to 0.06.
        assert!(
            keys.iter()
                .any(|&(at, val)| at > 0.56 && (val - 0.06).abs() < 1e-6)
        );
    }
}

fn import_media_item(
    item: NewMedia,
    stored_path: &str,
    studio: &mut Studio,
) -> Option<String> {
    studio.apply(Command::AddMedia { item }).or_else(|| {
        studio
            .project()
            .media
            .iter()
            .find(|media| media.path == stored_path)
            .map(|media| media.id.clone())
    })
}

fn import_threads_meme(
    source: &Path,
    idx: usize,
    media_dir: &Path,
    studio: &mut Studio,
) -> Option<String> {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp"));
    let Some(extension) = extension else {
        studio.notify(&t("threads.memeUnsupported"), true);
        return None;
    };
    let path = media_dir.join(format!("threads-meme-{idx}.{extension}"));
    if let Err(error) = std::fs::copy(source, &path) {
        studio.notify(&format!("Could not copy the meme image: {error}"), true);
        return None;
    }
    match concat_host::media::probe(&path.to_string_lossy()) {
        Ok(summary) if summary.kind == MediaKind::Image => {
            let stored_path = summary.path.clone();
            import_media_item(summary.to_new_media(), &stored_path, studio)
        }
        _ => {
            // Fallback for FFmpeg builds without PNG/WebP decoders: transcode to JPEG.
            let bytes = std::fs::read(&path).ok()?;
            let rgb = image::load_from_memory(&bytes).ok()?.to_rgb8();
            let _ = std::fs::remove_file(&path);
            save_and_import_jpeg(&rgb, &format!("threads-meme-{idx}.jpg"), media_dir, studio)
        }
    }
}

fn render_backdrop_rgb(
    theme: concat_text::threads_card::ThreadsCardTheme,
    width: u32,
    height: u32,
) -> Option<image::RgbImage> {
    let png = match concat_text::threads_card::render_threads_backdrop(theme, width, height) {
        Ok(png) => png,
        Err(error) => {
            log::error!("threads: could not render backdrop: {error}");
            return None;
        }
    };
    match image::load_from_memory(&png) {
        Ok(img) => Some(img.to_rgb8()),
        Err(error) => {
            log::error!("threads: could not decode rendered backdrop: {error}");
            None
        }
    }
}

fn save_and_import_jpeg(
    rgb: &image::RgbImage,
    filename: &str,
    media_dir: &Path,
    studio: &mut Studio,
) -> Option<String> {
    let mut jpeg = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 94);
    if let Err(error) = encoder.encode_image(rgb) {
        log::error!("threads: could not encode {filename}: {error}");
        return None;
    }
    let path = media_dir.join(filename);
    if let Err(error) = std::fs::write(&path, &jpeg) {
        log::error!("threads: could not write {filename}: {error}");
        return None;
    }
    match concat_host::media::probe(&path.to_string_lossy()) {
        Ok(summary) if summary.kind == MediaKind::Image => {
            let stored = summary.path.clone();
            import_media_item(summary.to_new_media(), &stored, studio)
        }
        Ok(summary) => {
            log::error!(
                "threads: unexpected media kind {:?} when probing {filename}",
                summary.kind
            );
            None
        }
        Err(error) => {
            log::error!("threads: could not probe {filename}: {error}");
            None
        }
    }
}

fn render_and_import_card(
    fonts: &concat_text::Fonts,
    spec: &concat_text::threads_card::ThreadsCardSpec,
    backdrop: Option<&image::RgbImage>,
    number: usize,
    width: u32,
    height: u32,
    media_dir: &Path,
    studio: &mut Studio,
) -> Option<String> {
    let rendered = match concat_text::threads_card::render_threads_card(fonts, spec, width, height)
    {
        Ok(rendered) => rendered,
        Err(error) => {
            log::error!("threads: could not render card {number}: {error}");
            return None;
        }
    };
    let card_rgba = match image::load_from_memory(&rendered.png) {
        Ok(img) => img.to_rgba8(),
        Err(error) => {
            log::error!("threads: could not decode rendered card {number}: {error}");
            return None;
        }
    };
    let (cw, ch) = card_rgba.dimensions();
    let mut out = if let Some(bg) = backdrop
        && bg.width() == cw
        && bg.height() == ch
    {
        bg.clone()
    } else {
        let fill = match spec.theme {
            concat_text::threads_card::ThreadsCardTheme::Dark => image::Rgb([14, 16, 22]),
            concat_text::threads_card::ThreadsCardTheme::Light => image::Rgb([242, 244, 248]),
        };
        image::RgbImage::from_pixel(cw, ch, fill)
    };

    // Straight-alpha composite the rendered Threads card over the vertical backdrop.
    for (dst, src) in out.pixels_mut().zip(card_rgba.pixels()) {
        let a = f32::from(src[3]) / 255.0;
        if a <= 0.0 {
            continue;
        }
        let inv = 1.0 - a;
        dst[0] = (f32::from(src[0]) * a + f32::from(dst[0]) * inv).round() as u8;
        dst[1] = (f32::from(src[1]) * a + f32::from(dst[1]) * inv).round() as u8;
        dst[2] = (f32::from(src[2]) * a + f32::from(dst[2]) * inv).round() as u8;
    }

    save_and_import_jpeg(&out, &format!("threads-card-{number}.jpg"), media_dir, studio)
}

fn decode_avatar_pixels(bytes: &[u8]) -> Option<concat_text::threads_card::AvatarPixels> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(concat_text::threads_card::AvatarPixels {
        width,
        height,
        rgba: rgba.into_raw(),
    })
}

fn place_still_clip(
    studio: &mut Studio,
    media_id: String,
    track_id: &str,
    start: f64,
    duration: f64,
    pop_seconds: Option<f64>,
) -> Option<String> {
    let clip_id = studio.apply(Command::AddClip {
        media_id,
        track_id: track_id.to_owned(),
        start,
        ripple: false,
    })?;
    let delta = duration.max(0.25) - CONCAT_DEFAULT_IMAGE_DURATION;
    if delta.abs() > 1e-4 {
        studio.apply(Command::TrimClip {
            clip_id: clip_id.clone(),
            edge: TrimEdge::End,
            delta,
            ripple: false,
        });
    }
    if let Some(seconds) = pop_seconds {
        studio.apply(Command::UpdateClip {
            clip_id: clip_id.clone(),
            patch: ClipPatch {
                motion: Some(Some(ClipMotion {
                    enter: Some(ClipMove {
                        kind: "pop".to_owned(),
                        seconds,
                    }),
                    exit: None,
                    cycle: None,
                })),
                ..Default::default()
            },
        });
    }
    Some(clip_id)
}

fn place_threads_bgm(
    options: &ThreadsBuildOptions,
    total_duration: f64,
    speech_spans: &[(f64, f64)],
    audio_dir: &Path,
    studio: &mut Studio,
) {
    let src_path = match options.bgm_preset {
        crate::threads_bgm::BgmPreset::Off => return,
        crate::threads_bgm::BgmPreset::Custom => {
            let Some(custom) = options.bgm_custom_path.clone() else {
                studio.notify(&t("threads.bgmCustomMissing"), true);
                return;
            };
            custom
        }
        preset => match crate::threads_bgm::ensure_preset_wav(&studio.host.dirs.data, preset) {
            Ok(path) => path,
            Err(error) => {
                studio.notify(&error, true);
                return;
            }
        },
    };

    let ext = src_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|ext| !ext.is_empty())
        .unwrap_or_else(|| "wav".to_owned());
    let dest = audio_dir.join(format!("threads-bgm.{ext}"));
    if let Err(error) = std::fs::copy(&src_path, &dest) {
        studio.notify(&format!("Could not copy BGM file: {error}"), true);
        return;
    }

    let summary = match concat_host::media::probe(&dest.to_string_lossy()) {
        Ok(summary) if summary.kind == MediaKind::Audio => summary,
        Ok(_) => {
            studio.notify(&t("threads.bgmUnsupported"), true);
            return;
        }
        Err(error) => {
            studio.notify(&format!("{}: {error}", t("threads.bgmUnsupported")), true);
            return;
        }
    };

    let loop_dur = summary.duration.unwrap_or(0.0).max(0.5);
    let stored = summary.path.clone();
    let Some(media_id) = import_media_item(summary.to_new_media(), &stored, studio) else {
        return;
    };

    let base_vol = f64::from(options.bgm_volume).clamp(0.02, 0.55);
    let duck_vol = (base_vol * 0.38).clamp(0.015, 0.16);
    let swell_vol = (base_vol * 1.15).clamp(base_vol, 0.60);

    let mut at = 0.0f64;
    let mut is_first = true;
    for _ in 0..200 {
        let remaining = total_duration - at;
        if remaining <= 0.05 {
            break;
        }
        let clip_dur = remaining.min(loop_dur);
        let is_last = (total_duration - (at + clip_dur)) <= 0.05;
        let Some(clip_id) = studio.apply(Command::AddClip {
            media_id: media_id.clone(),
            track_id: "T4".to_owned(),
            start: at,
            ripple: false,
        }) else {
            break;
        };
        if loop_dur - clip_dur > 0.02 {
            studio.apply(Command::TrimClip {
                clip_id: clip_id.clone(),
                edge: TrimEdge::End,
                delta: clip_dur - loop_dur,
                ripple: false,
            });
        }

        let keys = build_ducking_keyframes(at, clip_dur, speech_spans, duck_vol, swell_vol);
        let has_variation = keys
            .first()
            .is_some_and(|&(_, first)| keys.iter().any(|&(_, v)| (v - first).abs() > 0.01));
        studio.apply(Command::UpdateClip {
            clip_id: clip_id.clone(),
            patch: ClipPatch {
                volume: Some(if has_variation { base_vol } else { duck_vol }),
                fade_in: is_first.then(|| 0.6f64.min(clip_dur * 0.25)),
                fade_out: is_last.then(|| 1.4f64.min(clip_dur * 0.35)),
                ..Default::default()
            },
        });
        if has_variation {
            for (frac, value) in keys {
                studio.apply(Command::SetClipKey {
                    clip_id: clip_id.clone(),
                    property: KeyProperty::Volume,
                    at: frac,
                    value,
                    ease: KeyEase::LINEAR,
                });
            }
        }

        at += clip_dur;
        is_first = false;
    }
}

fn build_ducking_keyframes(
    clip_start: f64,
    clip_dur: f64,
    speech_spans: &[(f64, f64)],
    duck_vol: f64,
    swell_vol: f64,
) -> Vec<(f64, f64)> {
    let clip_end = clip_start + clip_dur.max(0.1);
    let in_speech = |t: f64| {
        speech_spans
            .iter()
            .any(|&(s, e)| t >= s - 1e-4 && t <= e + 1e-4)
    };
    let mut raw: Vec<(f64, f64)> = Vec::new();
    raw.push((0.0, if in_speech(clip_start) { duck_vol } else { swell_vol }));

    for (idx, &(s, e)) in speech_spans.iter().enumerate() {
        // Duck in cleanly right before speech starts if preceded by a break.
        if s > clip_start + 0.25 && s < clip_end {
            raw.push((((s - 0.18 - clip_start) / clip_dur).clamp(0.0, 1.0), swell_vol));
            raw.push((((s + 0.04 - clip_start) / clip_dur).clamp(0.0, 1.0), duck_vol));
        }
        // Only swell between speech segments when there is a meaningful pause (e.g., 1.5s meme).
        let next_start = speech_spans
            .get(idx + 1)
            .map(|&(ns, _)| ns)
            .unwrap_or(f64::INFINITY);
        if next_start - e >= 0.55 && e >= clip_start && e < clip_end - 0.1 {
            raw.push((((e - 0.05 - clip_start) / clip_dur).clamp(0.0, 1.0), duck_vol));
            raw.push((((e + 0.18 - clip_start) / clip_dur).clamp(0.0, 1.0), swell_vol));
        }
    }
    raw.push((1.0, if in_speech(clip_end) { duck_vol } else { swell_vol }));

    raw.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(raw.len());
    for (pos, val) in raw {
        if let Some(last) = merged.last_mut() {
            if (pos - last.0).abs() <= 0.006 {
                last.1 = val;
                continue;
            }
        }
        merged.push((pos, val));
    }
    merged
}
