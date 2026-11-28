// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use concat_host::playback::ClipSpec;
use concat_project::model::MediaKind;
use concat_text::threads_card::ThreadsCardTheme;

use crate::studio::Studio;
use crate::tgs::{self, CommentSort, Extraction};
use crate::threads_bgm::{self, BgmPreset};
use crate::threads_karaoke::KaraokeStyle;
use crate::ui::{ThreadsData, ThreadsFrameData};

#[derive(Clone, Debug)]
pub struct ThreadsBuildOptions {
    pub extraction: Extraction,
    pub voiceovers: Vec<tgs::VoiceoverAudio>,
    pub card_theme: ThreadsCardTheme,
    pub karaoke_style: KaraokeStyle,
    pub bgm_preset: BgmPreset,
    pub bgm_custom_path: Option<PathBuf>,
    pub bgm_volume: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviewKind {
    Voice(usize),
    Bgm,
}

#[derive(Clone, Debug)]
pub enum ThreadsMsg {
    Open,
    Close,
    UrlEdited(String),
    Extract,
    SelectFrame(i32),
    FrameAuthorEdited(String),
    FrameTextEdited(String),
    MoveFrameUp(i32),
    MoveFrameDown(i32),
    RemoveFrame(i32),
    AddCustomFrame,
    AddNextComment,
    ResetFrames,
    SortChanged(i32),
    CardThemeChanged(i32),
    VoiceChanged(i32),
    VoiceRateChanged(i32),
    KaraokeStyleChanged(i32),
    GenerateVoiceovers,
    PreviewSceneVoice(i32),
    ChooseSceneMeme(i32),
    RemoveSceneMeme(i32),
    ApplyMemeToAll(i32),
    BgmPresetChanged(i32),
    ChooseBgmFile,
    BgmVolumeChanged(f32),
    PreviewBgm,
    StopPreview,
    PreviewTick,
    CreateProject,
}

pub struct ThreadsPane {
    pub open: bool,
    pub url: String,
    pub running: bool,
    pub stage: String,
    pub error: String,
    pub extraction: Option<Extraction>,
    pub selected: usize,
    pub sort: CommentSort,
    pub card_theme: ThreadsCardTheme,
    pub voice: usize,
    pub voice_rate: usize,
    pub karaoke_style: KaraokeStyle,
    pub voiceovers: Vec<Option<tgs::VoiceoverAudio>>,
    pub generating_voiceovers: bool,
    pub bgm_preset: BgmPreset,
    pub bgm_custom_path: Option<PathBuf>,
    pub bgm_volume: f32,
    pub scene_revision: i32,
    preview_kind: Option<PreviewKind>,
    preview_duration: f64,
    preview_loading_scene: Option<usize>,
    preview_req_seq: u64,
    preview_timer: slint::Timer,
}

impl Default for ThreadsPane {
    fn default() -> Self {
        Self {
            open: false,
            url: String::new(),
            running: false,
            stage: String::new(),
            error: String::new(),
            extraction: None,
            selected: 0,
            sort: CommentSort::Likes,
            card_theme: ThreadsCardTheme::Dark,
            voice: 0,
            // Default to 1.15× (+15%) so TikTok narration feels brisk out of the box.
            voice_rate: 1,
            karaoke_style: KaraokeStyle::ClassicGold,
            voiceovers: Vec::new(),
            generating_voiceovers: false,
            bgm_preset: BgmPreset::LoFiChill,
            bgm_custom_path: None,
            bgm_volume: 0.18,
            scene_revision: 0,
            preview_kind: None,
            preview_duration: 0.0,
            preview_loading_scene: None,
            preview_req_seq: 0,
            preview_timer: slint::Timer::default(),
        }
    }
}

fn should_stop_preview(msg: &ThreadsMsg) -> bool {
    matches!(
        msg,
        ThreadsMsg::Close
            | ThreadsMsg::UrlEdited(_)
            | ThreadsMsg::Extract
            | ThreadsMsg::MoveFrameUp(_)
            | ThreadsMsg::MoveFrameDown(_)
            | ThreadsMsg::RemoveFrame(_)
            | ThreadsMsg::ResetFrames
            | ThreadsMsg::SortChanged(_)
            | ThreadsMsg::VoiceChanged(_)
            | ThreadsMsg::VoiceRateChanged(_)
            | ThreadsMsg::GenerateVoiceovers
            | ThreadsMsg::BgmPresetChanged(_)
            | ThreadsMsg::ChooseBgmFile
            | ThreadsMsg::StopPreview
            | ThreadsMsg::CreateProject
    )
}

impl ThreadsPane {
    /// Stops any active voiceover or BGM audition playback and restores project audio if loaded.
    pub fn stop_preview(&mut self, studio: &mut Studio) {
        self.preview_timer.stop();
        let was_playing = self.preview_kind.take().is_some();
        self.preview_duration = 0.0;
        if !was_playing {
            return;
        }
        studio.host.playback.pause();
        if studio.session.is_some() {
            studio.sync_audio();
        } else {
            studio.host.playback.set_clips(PathBuf::new(), Vec::new());
        }
    }

    fn invalidate_voiceovers(&mut self) {
        for slot in &mut self.voiceovers {
            *slot = None;
        }
        self.preview_loading_scene = None;
        self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
    }

    /// Handles pure in-memory scene editing, theme, sorting, meme, voice, and BGM
    /// state transitions that do not touch the host or studio.
    fn apply_local(&mut self, msg: ThreadsMsg) -> Result<(), ThreadsMsg> {
        match msg {
            ThreadsMsg::Open => {
                self.open = true;
                self.error.clear();
            }
            ThreadsMsg::Close if !self.running => {
                self.open = false;
                self.preview_loading_scene = None;
                self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
            }
            ThreadsMsg::Close => {}
            ThreadsMsg::UrlEdited(url) => {
                self.url = url;
                self.error.clear();
                self.extraction = None;
                self.voiceovers.clear();
                self.selected = 0;
                self.preview_loading_scene = None;
                self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
                self.scene_revision += 1;
            }
            ThreadsMsg::SelectFrame(index) => {
                let Some(extraction) = &self.extraction else {
                    return Ok(());
                };
                if extraction.frames.is_empty() {
                    return Ok(());
                }
                let next = (index.max(0) as usize).min(extraction.frames.len() - 1);
                if next != self.selected {
                    self.selected = next;
                    self.scene_revision += 1;
                }
            }
            ThreadsMsg::FrameAuthorEdited(author) => {
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                let Some(frame) = extraction.frames.get_mut(self.selected) else {
                    return Ok(());
                };
                if frame.author != author {
                    let invalidate = frame.text.trim().is_empty();
                    frame.author = author;
                    if invalidate && let Some(slot) = self.voiceovers.get_mut(self.selected) {
                        *slot = None;
                    }
                    self.error.clear();
                }
            }
            ThreadsMsg::FrameTextEdited(text) => {
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                let Some(frame) = extraction.frames.get_mut(self.selected) else {
                    return Ok(());
                };
                if frame.text != text {
                    let changed_spoken = frame.text.trim() != text.trim();
                    frame.text = text;
                    if changed_spoken && let Some(slot) = self.voiceovers.get_mut(self.selected) {
                        *slot = None;
                    }
                    self.error.clear();
                }
            }
            ThreadsMsg::MoveFrameUp(index) => {
                if self.running {
                    return Ok(());
                }
                let idx = index.max(0) as usize;
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                if idx == 0 || idx >= extraction.frames.len() {
                    return Ok(());
                }
                extraction.frames.swap(idx, idx - 1);
                if self.voiceovers.len() == extraction.frames.len() {
                    self.voiceovers.swap(idx, idx - 1);
                }
                if self.selected == idx {
                    self.selected = idx - 1;
                } else if self.selected == idx - 1 {
                    self.selected = idx;
                }
                self.preview_loading_scene = None;
                self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
                self.scene_revision += 1;
            }
            ThreadsMsg::MoveFrameDown(index) => {
                if self.running {
                    return Ok(());
                }
                let idx = index.max(0) as usize;
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                if idx + 1 >= extraction.frames.len() {
                    return Ok(());
                }
                extraction.frames.swap(idx, idx + 1);
                if self.voiceovers.len() == extraction.frames.len() {
                    self.voiceovers.swap(idx, idx + 1);
                }
                if self.selected == idx {
                    self.selected = idx + 1;
                } else if self.selected == idx + 1 {
                    self.selected = idx;
                }
                self.preview_loading_scene = None;
                self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
                self.scene_revision += 1;
            }
            ThreadsMsg::RemoveFrame(index) => {
                if self.running {
                    return Ok(());
                }
                let idx = index.max(0) as usize;
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                if extraction.frames.len() <= 1 || idx >= extraction.frames.len() {
                    return Ok(());
                }
                extraction.frames.remove(idx);
                if idx < self.voiceovers.len() {
                    self.voiceovers.remove(idx);
                }
                if self.selected >= extraction.frames.len() {
                    self.selected = extraction.frames.len() - 1;
                } else if self.selected > idx {
                    self.selected -= 1;
                }
                self.preview_loading_scene = None;
                self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
                self.scene_revision += 1;
            }
            ThreadsMsg::AddCustomFrame => {
                if self.running {
                    return Ok(());
                }
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                let custom_count = extraction
                    .frames
                    .iter()
                    .filter(|frame| frame.id.starts_with("custom-"))
                    .count()
                    + 1;
                let default_author = extraction.post_frame.author.clone();
                let default_username = extraction.post_frame.username.clone();
                let default_avatar = extraction.post_frame.avatar_bytes.clone();
                extraction.frames.push(tgs::Frame {
                    id: format!("custom-{custom_count}"),
                    label: crate::i18n::t("threads.customScene"),
                    author: default_author,
                    username: default_username,
                    avatar_bytes: default_avatar,
                    ..tgs::Frame::default()
                });
                self.voiceovers.push(None);
                self.selected = extraction.frames.len() - 1;
                self.error.clear();
                self.scene_revision += 1;
            }
            ThreadsMsg::AddNextComment => {
                if self.running {
                    return Ok(());
                }
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                let Some(next) = tgs::next_pool_comment(extraction, self.sort) else {
                    return Ok(());
                };
                extraction.frames.push(next);
                self.voiceovers.push(None);
                self.selected = extraction.frames.len() - 1;
                self.error.clear();
                self.scene_revision += 1;
            }
            ThreadsMsg::ResetFrames => {
                if self.running {
                    return Ok(());
                }
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                let pool = tgs::pool_for(extraction, self.sort);
                extraction.frames = tgs::default_frames(&extraction.post_frame, pool);
                self.voiceovers = vec![None; extraction.frames.len()];
                self.selected = 0;
                self.error.clear();
                self.preview_loading_scene = None;
                self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
                self.scene_revision += 1;
            }
            ThreadsMsg::SortChanged(index) => {
                if self.running {
                    return Ok(());
                }
                let next_sort = if index == 1 {
                    CommentSort::Replies
                } else {
                    CommentSort::Likes
                };
                if next_sort == self.sort {
                    return Ok(());
                }
                self.sort = next_sort;
                if let Some(extraction) = &mut self.extraction {
                    let pool = tgs::pool_for(extraction, self.sort);
                    extraction.frames = tgs::default_frames(&extraction.post_frame, pool);
                    self.voiceovers = vec![None; extraction.frames.len()];
                    self.selected = 0;
                    self.error.clear();
                    self.preview_loading_scene = None;
                    self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
                    self.scene_revision += 1;
                }
            }
            ThreadsMsg::CardThemeChanged(index) => {
                self.card_theme = if index == 1 {
                    ThreadsCardTheme::Light
                } else {
                    ThreadsCardTheme::Dark
                };
            }
            ThreadsMsg::VoiceChanged(index) => {
                if self.running {
                    return Ok(());
                }
                let next = (index.max(0) as usize).min(tgs::VOICES.len() - 1);
                if next != self.voice {
                    self.voice = next;
                    self.invalidate_voiceovers();
                    self.error.clear();
                }
            }
            ThreadsMsg::VoiceRateChanged(index) => {
                if self.running {
                    return Ok(());
                }
                let next = (index.max(0) as usize).min(tgs::VOICE_RATES.len() - 1);
                if next != self.voice_rate {
                    self.voice_rate = next;
                    self.invalidate_voiceovers();
                    self.error.clear();
                }
            }
            ThreadsMsg::KaraokeStyleChanged(index) => {
                self.karaoke_style = KaraokeStyle::from_index(index);
            }
            ThreadsMsg::RemoveSceneMeme(index) => {
                if self.running {
                    return Ok(());
                }
                let idx = if index < 0 { self.selected } else { index as usize };
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                if let Some(frame) = extraction.frames.get_mut(idx) {
                    frame.meme_path = None;
                    self.error.clear();
                }
            }
            ThreadsMsg::ApplyMemeToAll(index) => {
                if self.running {
                    return Ok(());
                }
                let idx = if index < 0 { self.selected } else { index as usize };
                let Some(extraction) = &mut self.extraction else {
                    return Ok(());
                };
                let source = extraction
                    .frames
                    .get(idx)
                    .and_then(|frame| frame.meme_path.clone());
                for frame in &mut extraction.frames {
                    frame.meme_path.clone_from(&source);
                }
                self.error.clear();
            }
            ThreadsMsg::BgmPresetChanged(index) => {
                let next = BgmPreset::from_index(index);
                if next != self.bgm_preset {
                    self.bgm_preset = next;
                    self.error.clear();
                }
            }
            ThreadsMsg::BgmVolumeChanged(volume) => {
                if volume.is_finite() {
                    self.bgm_volume = volume.clamp(0.0, 0.5);
                }
            }
            other => return Err(other),
        }
        Ok(())
    }

    pub fn update(&mut self, msg: ThreadsMsg, studio: &mut Studio) {
        if should_stop_preview(&msg) {
            self.stop_preview(studio);
        }
        let Err(msg) = self.apply_local(msg) else {
            return;
        };
        match msg {
            ThreadsMsg::StopPreview => {}
            ThreadsMsg::PreviewTick => {
                if self.preview_kind.is_none() {
                    self.preview_timer.stop();
                    return;
                }
                let pos = studio.host.playback.position_now();
                if pos + 0.03 >= self.preview_duration {
                    self.stop_preview(studio);
                }
            }
            ThreadsMsg::Extract => {
                let normalized = match tgs::normalize_threads_url(&self.url) {
                    Ok(url) => url,
                    Err(error) => {
                        self.error = error;
                        return;
                    }
                };
                self.url = normalized.clone();
                self.running = true;
                self.stage = "Fetching public Threads post…".to_owned();
                self.error.clear();
                self.extraction = None;
                self.voiceovers.clear();
                self.selected = 0;
                self.generating_voiceovers = false;
                self.preview_loading_scene = None;
                self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
                self.scene_revision += 1;

                crate::host::spawn_long(
                    move || {
                        let mut last = String::new();
                        tgs::extract(&normalized, move |stage| {
                            if stage == last {
                                return;
                            }
                            last.clone_from(&stage);
                            crate::host::on_ui(move |studio, _, _| {
                                studio.threads.stage = stage;
                            });
                        })
                    },
                    |studio, _, _, result| studio.threads.finish(result),
                );
            }
            ThreadsMsg::GenerateVoiceovers => {
                let Some(extraction) = self.extraction.as_ref() else {
                    return;
                };
                if self.running {
                    return;
                }
                if self.voiceovers.len() != extraction.frames.len() {
                    self.voiceovers.resize_with(extraction.frames.len(), || None);
                }
                let pending: Vec<(usize, tgs::Frame)> = extraction
                    .frames
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| {
                        self.voiceovers
                            .get(*idx)
                            .is_none_or(Option::is_none)
                    })
                    .map(|(idx, frame)| (idx, frame.clone()))
                    .collect();
                if pending.is_empty() {
                    self.error.clear();
                    return;
                }
                if pending.iter().any(|(_, frame)| {
                    frame.text.trim().is_empty() && frame.author.trim().is_empty()
                }) {
                    self.error = crate::i18n::t("threads.emptySceneText");
                    return;
                }
                let project_id = extraction.project_id.clone();
                let (voice_id, _) = tgs::voice_at(self.voice);
                let (_, rate) = tgs::voice_rate_at(self.voice_rate);
                self.running = true;
                self.generating_voiceovers = true;
                self.preview_loading_scene = None;
                self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
                self.error.clear();
                self.stage = crate::i18n::t("threads.generatingVoices");

                crate::host::spawn_long(
                    move || {
                        tgs::generate_voiceovers(&project_id, &pending, voice_id, rate, |stage| {
                            crate::host::on_ui(move |studio, _, _| {
                                studio.threads.stage = stage;
                            });
                        })
                    },
                    |studio, _, _, result| studio.threads.finish_voiceovers(result),
                );
            }
            ThreadsMsg::PreviewSceneVoice(index) => {
                let (idx, frame, project_id, cached) = {
                    let Some(extraction) = self.extraction.as_ref() else {
                        return;
                    };
                    if extraction.frames.is_empty() {
                        return;
                    }
                    let idx = if index < 0 {
                        self.selected.min(extraction.frames.len() - 1)
                    } else {
                        (index as usize).min(extraction.frames.len() - 1)
                    };
                    let cached = self.voiceovers.get(idx).and_then(Option::as_ref).cloned();
                    (
                        idx,
                        extraction.frames[idx].clone(),
                        extraction.project_id.clone(),
                        cached,
                    )
                };
                if self.preview_kind == Some(PreviewKind::Voice(idx)) {
                    self.stop_preview(studio);
                    return;
                }
                if self.running {
                    return;
                }
                if let Some(count) = self.extraction.as_ref().map(|ext| ext.frames.len())
                    && self.voiceovers.len() != count
                {
                    self.voiceovers.resize_with(count, || None);
                }
                if let Some(cached) = cached {
                    self.start_voice_preview(idx, &cached, studio);
                    return;
                }
                if self.preview_loading_scene.is_some() {
                    return;
                }
                if frame.text.trim().is_empty() && frame.author.trim().is_empty() {
                    self.error = crate::i18n::t("threads.emptySceneText");
                    return;
                }
                self.stop_preview(studio);
                self.error.clear();
                self.preview_loading_scene = Some(idx);
                self.preview_req_seq = self.preview_req_seq.wrapping_add(1);
                let req_seq = self.preview_req_seq;
                let (voice_id, _) = tgs::voice_at(self.voice);
                let (_, rate) = tgs::voice_rate_at(self.voice_rate);

                crate::host::spawn_long(
                    move || {
                        let mut out = tgs::generate_voiceovers(
                            &project_id,
                            &[(idx, frame)],
                            voice_id,
                            rate,
                            |_| {},
                        )?;
                        out.pop()
                            .map(|(_, audio)| audio)
                            .ok_or_else(|| "TGS did not return preview audio.".to_owned())
                    },
                    move |studio, _, _, result| {
                        let mut pane = std::mem::take(&mut studio.threads);
                        pane.finish_single_voice_preview(req_seq, idx, result, studio);
                        studio.threads = pane;
                    },
                );
            }
            ThreadsMsg::ChooseSceneMeme(index) => {
                if self.running {
                    return;
                }
                let Some(extraction) = &self.extraction else {
                    return;
                };
                if extraction.frames.is_empty() {
                    return;
                }
                let idx = if index < 0 {
                    self.selected.min(extraction.frames.len() - 1)
                } else {
                    (index as usize).min(extraction.frames.len() - 1)
                };
                let target_id = extraction.frames[idx].id.clone();
                crate::host::pick_files_then(
                    crate::i18n::t("threads.chooseMeme"),
                    Some(("Images".to_owned(), &["png", "jpg", "jpeg", "webp"])),
                    move |studio, _, _, paths| {
                        let Some(path) = paths.into_iter().next() else {
                            return;
                        };
                        let valid_image = match concat_host::media::probe(&path.to_string_lossy()) {
                            Ok(summary) if summary.kind == MediaKind::Image => true,
                            _ => std::fs::read(&path)
                                .ok()
                                .and_then(|bytes| image::load_from_memory(&bytes).ok())
                                .is_some(),
                        };
                        if valid_image {
                            if let Some(ext) = &mut studio.threads.extraction {
                                if let Some(frame) =
                                    ext.frames.iter_mut().find(|frame| frame.id == target_id)
                                {
                                    frame.meme_path = Some(path);
                                } else if let Some(frame) = ext.frames.get_mut(idx) {
                                    frame.meme_path = Some(path);
                                }
                            }
                            studio.threads.error.clear();
                        } else {
                            studio.threads.error = crate::i18n::t("threads.memeUnsupported");
                        }
                    },
                );
            }
            ThreadsMsg::ChooseBgmFile => {
                if self.running {
                    return;
                }
                crate::host::pick_files_then(
                    crate::i18n::t("threads.chooseBgm"),
                    Some((
                        "Audio".to_owned(),
                        &["mp3", "wav", "m4a", "aac", "ogg", "flac"],
                    )),
                    |studio, _, _, paths| {
                        let Some(path) = paths.into_iter().next() else {
                            return;
                        };
                        match concat_host::media::probe(&path.to_string_lossy()) {
                            Ok(summary) if summary.kind == MediaKind::Audio => {
                                let mut pane = std::mem::take(&mut studio.threads);
                                pane.stop_preview(studio);
                                pane.bgm_custom_path = Some(path);
                                pane.bgm_preset = BgmPreset::Custom;
                                pane.error.clear();
                                studio.threads = pane;
                            }
                            Ok(_) => {
                                studio.threads.error = crate::i18n::t("threads.bgmUnsupported");
                            }
                            Err(error) => {
                                studio.threads.error = format!(
                                    "{}: {error}",
                                    crate::i18n::t("threads.bgmUnsupported")
                                );
                            }
                        }
                    },
                );
            }
            ThreadsMsg::PreviewBgm => {
                if self.preview_kind == Some(PreviewKind::Bgm) {
                    self.stop_preview(studio);
                    return;
                }
                if self.bgm_preset == BgmPreset::Off {
                    return;
                }
                let path = match self.bgm_preset {
                    BgmPreset::Off => return,
                    BgmPreset::Custom => {
                        let Some(custom) = self.bgm_custom_path.clone() else {
                            self.error = crate::i18n::t("threads.bgmCustomMissing");
                            return;
                        };
                        custom
                    }
                    preset => match threads_bgm::ensure_preset_wav(&studio.host.dirs.data, preset) {
                        Ok(path) => path,
                        Err(error) => {
                            self.error = error;
                            return;
                        }
                    },
                };
                match concat_host::media::probe(&path.to_string_lossy()) {
                    Ok(summary) if summary.kind == MediaKind::Audio => {
                        let duration = summary.duration.unwrap_or(10.0).clamp(1.0, 20.0);
                        self.start_audio_preview(
                            PreviewKind::Bgm,
                            &path,
                            duration,
                            self.bgm_volume.max(0.05),
                            studio,
                        );
                    }
                    Ok(_) => self.error = crate::i18n::t("threads.bgmUnsupported"),
                    Err(error) => {
                        self.error = format!("{}: {error}", crate::i18n::t("threads.bgmUnsupported"))
                    }
                }
            }
            ThreadsMsg::CreateProject => {
                let Some(extraction) = self.extraction.clone() else {
                    return;
                };
                if extraction.frames.is_empty() {
                    return;
                }
                let ready: Vec<tgs::VoiceoverAudio> =
                    self.voiceovers.iter().flatten().cloned().collect();
                if ready.len() != extraction.frames.len() {
                    self.error = "A voiceover is missing for one or more frames.".to_owned();
                    return;
                }
                if self.bgm_preset == BgmPreset::Custom && self.bgm_custom_path.is_none() {
                    self.error = crate::i18n::t("threads.bgmCustomMissing");
                    return;
                }
                self.stop_preview(studio);
                self.open = false;
                studio.start.begin_threads_creation(ThreadsBuildOptions {
                    extraction,
                    voiceovers: ready,
                    card_theme: self.card_theme,
                    karaoke_style: self.karaoke_style,
                    bgm_preset: self.bgm_preset,
                    bgm_custom_path: self.bgm_custom_path.clone(),
                    bgm_volume: self.bgm_volume,
                });
            }
            _ => {}
        }
    }

    fn start_voice_preview(
        &mut self,
        idx: usize,
        audio: &tgs::VoiceoverAudio,
        studio: &mut Studio,
    ) {
        let preview_dir = studio.host.dirs.data.join("threads-preview");
        if let Err(error) = std::fs::create_dir_all(&preview_dir) {
            self.error = format!("Could not prepare preview cache: {error}");
            return;
        }
        let mut hasher = DefaultHasher::new();
        tgs::voice_at(self.voice).0.hash(&mut hasher);
        tgs::voice_rate_at(self.voice_rate).1.hash(&mut hasher);
        audio.bytes.hash(&mut hasher);
        let file = preview_dir.join(format!("voice-{:016x}.mp3", hasher.finish()));
        if !file.exists() {
            if let Err(error) = std::fs::write(&file, &audio.bytes) {
                self.error = format!("Could not write preview audio: {error}");
                return;
            }
        }
        let probed_dur = concat_host::media::probe(&file.to_string_lossy())
            .ok()
            .and_then(|summary| summary.duration)
            .unwrap_or(0.0);
        let duration = audio.duration_seconds.max(probed_dur).max(0.5);
        self.start_audio_preview(PreviewKind::Voice(idx), &file, duration, 1.0, studio);
    }

    fn start_audio_preview(
        &mut self,
        kind: PreviewKind,
        path: &Path,
        duration: f64,
        volume: f32,
        studio: &mut Studio,
    ) {
        self.preview_timer.stop();
        let preview_dir = studio.host.dirs.data.join("threads-preview");
        let _ = std::fs::create_dir_all(&preview_dir);
        let spec = ClipSpec {
            path: path.to_string_lossy().into_owned(),
            audio_stream: None,
            start: 0.0,
            duration,
            source_start: 0.0,
            volume,
            volume_curve: Vec::new(),
            fade_in: 0.0,
            fade_out: if kind == PreviewKind::Bgm {
                0.35f64.min(duration * 0.25)
            } else {
                0.0
            },
            speed: 1.0,
            tempo: Vec::new(),
            preserve_pitch: true,
            chain: String::new(),
            bus: None,
        };
        studio.host.playback.pause();
        studio.host.playback.set_clips(preview_dir, vec![spec]);
        studio.host.playback.seek(0.0);
        studio.host.playback.play(0.0);

        self.preview_kind = Some(kind);
        self.preview_duration = duration;
        self.error.clear();

        self.preview_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(60),
            || {
                crate::host::Shell::with(|shell, app| {
                    if let Ok(mut studio) = shell.studio.try_borrow_mut() {
                        studio.handle(crate::panes::Msg::Threads(ThreadsMsg::PreviewTick));
                    }
                    if let Ok(studio) = shell.studio.try_borrow() {
                        studio.publish_chrome(&app, &shell.models);
                    }
                });
            },
        );
    }

    fn finish(&mut self, result: Result<Extraction, String>) {
        self.running = false;
        self.generating_voiceovers = false;
        self.preview_loading_scene = None;
        self.stage.clear();
        self.selected = 0;
        self.sort = CommentSort::Likes;
        match result {
            Ok(extraction) => {
                self.error.clear();
                self.voiceovers = vec![None; extraction.frames.len()];
                self.extraction = Some(extraction);
            }
            Err(error) => {
                self.error = error;
                self.extraction = None;
                self.voiceovers.clear();
            }
        }
        self.scene_revision += 1;
    }

    fn finish_single_voice_preview(
        &mut self,
        req_seq: u64,
        idx: usize,
        result: Result<tgs::VoiceoverAudio, String>,
        studio: &mut Studio,
    ) {
        self.preview_loading_scene = None;
        if self.preview_req_seq != req_seq {
            return;
        }
        match result {
            Ok(audio) => {
                let count = self.extraction.as_ref().map_or(0, |item| item.frames.len());
                if idx < count {
                    if self.voiceovers.len() != count {
                        self.voiceovers.resize_with(count, || None);
                    }
                    self.voiceovers[idx] = Some(audio.clone());
                }
                if self.open {
                    self.start_voice_preview(idx, &audio, studio);
                }
            }
            Err(error) => {
                self.error = error;
            }
        }
    }

    fn finish_voiceovers(&mut self, result: Result<Vec<(usize, tgs::VoiceoverAudio)>, String>) {
        self.running = false;
        self.generating_voiceovers = false;
        self.stage.clear();
        match result {
            Ok(generated) => {
                self.error.clear();
                let count = self.extraction.as_ref().map_or(0, |item| item.frames.len());
                if self.voiceovers.len() != count {
                    self.voiceovers.resize_with(count, || None);
                }
                for (index, audio) in generated {
                    if let Some(slot) = self.voiceovers.get_mut(index) {
                        *slot = Some(audio);
                    }
                }
            }
            Err(error) => {
                self.error = error;
            }
        }
    }

    pub fn data(&self) -> ThreadsData {
        let extraction = self.extraction.as_ref();
        let frames_slice = extraction
            .map(|item| item.frames.as_slice())
            .unwrap_or_default();
        let selected_index = if frames_slice.is_empty() {
            0
        } else {
            self.selected.min(frames_slice.len() - 1)
        };
        let selected_frame = frames_slice.get(selected_index);
        let voiceovers_ready = !frames_slice.is_empty()
            && self.voiceovers.len() == frames_slice.len()
            && self.voiceovers.iter().all(Option::is_some);
        let remaining_comments = extraction
            .map(|item| tgs::remaining_pool_count(item, self.sort) as i32)
            .unwrap_or(0);
        let meme_count = frames_slice
            .iter()
            .filter(|frame| frame.meme_path.is_some())
            .count() as i32;
        let bgm_custom_file = self
            .bgm_custom_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        ThreadsData {
            open: self.open,
            url: self.url.as_str().into(),
            running: self.running,
            generating_voiceovers: self.generating_voiceovers,
            voiceovers_ready,
            stage: self.stage.as_str().into(),
            error: self.error.as_str().into(),
            title: extraction
                .map(|item| item.title.as_str())
                .unwrap_or_default()
                .into(),
            source_url: extraction
                .map(|item| item.source_url.as_str())
                .unwrap_or_default()
                .into(),
            sort: match self.sort {
                CommentSort::Likes => 0,
                CommentSort::Replies => 1,
            },
            card_theme: match self.card_theme {
                ThreadsCardTheme::Dark => 0,
                ThreadsCardTheme::Light => 1,
            },
            voice: self.voice as i32,
            voice_rate: self.voice_rate as i32,
            karaoke_style: self.karaoke_style.index(),
            bgm_preset: self.bgm_preset.index(),
            bgm_custom_file: bgm_custom_file.into(),
            bgm_volume: self.bgm_volume,
            bgm_playing: self.preview_kind == Some(PreviewKind::Bgm),
            preview_loading: self.preview_loading_scene.is_some(),
            meme_count,
            selected_index: selected_index as i32,
            selected_label: selected_frame
                .map(|frame| frame.label.as_str())
                .unwrap_or_default()
                .into(),
            selected_author: selected_frame
                .map(|frame| frame.author.as_str())
                .unwrap_or_default()
                .into(),
            selected_username: selected_frame
                .map(|frame| frame.username.as_str())
                .unwrap_or_default()
                .into(),
            selected_timestamp: selected_frame
                .map(|frame| frame.timestamp.as_str())
                .unwrap_or_default()
                .into(),
            selected_verified: selected_frame.is_some_and(|frame| frame.verified),
            selected_has_media: selected_frame.is_some_and(|frame| frame.media_bytes.is_some()),
            selected_media_image: selected_frame
                .and_then(|frame| frame.media_bytes.as_deref())
                .and_then(decode_slint_image)
                .unwrap_or_default(),
            selected_avatar_image: selected_frame
                .and_then(|frame| frame.avatar_bytes.as_deref())
                .and_then(decode_slint_image)
                .unwrap_or_default(),
            selected_has_avatar: selected_frame.is_some_and(|frame| frame.avatar_bytes.is_some()),
            selected_text: selected_frame
                .map(|frame| frame.text.as_str())
                .unwrap_or_default()
                .into(),
            selected_likes: selected_frame
                .map(|frame| frame.likes.as_str())
                .unwrap_or_default()
                .into(),
            selected_replies: selected_frame
                .map(|frame| frame.replies.as_str())
                .unwrap_or_default()
                .into(),
            selected_reposts: selected_frame
                .map(|frame| frame.reposts.as_str())
                .unwrap_or_default()
                .into(),
            selected_is_post: selected_frame.is_some_and(|frame| frame.is_post),
            selected_meme_file: selected_frame
                .and_then(|frame| frame.meme_path.as_deref())
                .map(file_name_string)
                .unwrap_or_default()
                .into(),
            selected_voice_ready: self
                .voiceovers
                .get(selected_index)
                .is_some_and(Option::is_some),
            selected_voice_playing: self.preview_kind == Some(PreviewKind::Voice(selected_index)),
            selected_voice_loading: self.preview_loading_scene == Some(selected_index),
            scene_revision: self.scene_revision,
            remaining_comments,
            frames: slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
                frames_slice
                    .iter()
                    .enumerate()
                    .map(|(idx, frame)| {
                        let voice = self.voiceovers.get(idx).and_then(Option::as_ref);
                        ThreadsFrameData {
                            label: frame.label.as_str().into(),
                            author: frame.author.as_str().into(),
                            username: frame.username.as_str().into(),
                            text: frame.text.as_str().into(),
                            likes: frame.likes.as_str().into(),
                            replies: frame.replies.as_str().into(),
                            reposts: frame.reposts.as_str().into(),
                            is_post: frame.is_post,
                            has_media: frame.media_bytes.is_some(),
                            meme_file: frame
                                .meme_path
                                .as_deref()
                                .map(file_name_string)
                                .unwrap_or_default()
                                .into(),
                            voice_ready: voice.is_some(),
                            voice_playing: self.preview_kind == Some(PreviewKind::Voice(idx)),
                            voice_loading: self.preview_loading_scene == Some(idx),
                            voice_duration: voice
                                .filter(|audio| audio.duration_seconds > 0.0)
                                .map(|audio| format!("{:.1}s", audio.duration_seconds))
                                .unwrap_or_default()
                                .into(),
                        }
                    })
                    .collect::<Vec<_>>(),
            ))),
        }
    }
}

fn file_name_string(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn decode_slint_image(bytes: &[u8]) -> Option<slint::Image> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        rgba.as_raw(),
        w,
        h,
    );
    Some(slint::Image::from_rgba8(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tgs::{Extraction, Frame, VoiceoverAudio};

    fn sample_extraction() -> Extraction {
        let post_frame = Frame {
            id: "post".into(),
            label: "Post".into(),
            author: "OP".into(),
            username: "op.official".into(),
            text: "Bài gốc".into(),
            likes: "100".into(),
            replies: "10".into(),
            reposts: "5".into(),
            is_post: true,
            ..Frame::default()
        };
        let c1 = Frame {
            id: "c1".into(),
            label: "Comment 1".into(),
            author: "A".into(),
            text: "Bình luận 1".into(),
            likes: "50".into(),
            replies: "2".into(),
            ..Frame::default()
        };
        let c2 = Frame {
            id: "c2".into(),
            label: "Comment 2".into(),
            author: "B".into(),
            text: "Bình luận 2".into(),
            likes: "40".into(),
            replies: "25".into(),
            ..Frame::default()
        };
        let c3 = Frame {
            id: "c3".into(),
            label: "Comment 3".into(),
            author: "C".into(),
            text: "Bình luận 3".into(),
            likes: "30".into(),
            replies: "1".into(),
            ..Frame::default()
        };
        Extraction {
            title: "Test".into(),
            source_url: "https://www.threads.com/@op/post/1".into(),
            project_id: "proj-1".into(),
            post_frame,
            pool_by_likes: vec![c1.clone(), c2.clone(), c3.clone()],
            pool_by_replies: vec![c2.clone(), c1.clone(), c3],
            frames: vec![
                Frame {
                    id: "post".into(),
                    label: "Post".into(),
                    author: "OP".into(),
                    username: "op.official".into(),
                    text: "Bài gốc".into(),
                    likes: "100".into(),
                    replies: "10".into(),
                    reposts: "5".into(),
                    is_post: true,
                    ..Frame::default()
                },
                c1,
                c2,
            ],
        }
    }

    fn audio(duration_seconds: f64) -> VoiceoverAudio {
        VoiceoverAudio {
            bytes: vec![1, 2, 3],
            duration_seconds,
            word_timings: Vec::new(),
        }
    }

    #[test]
    fn edit_invalidates_only_changed_scene_while_voice_or_speed_invalidates_all() {
        let mut pane = ThreadsPane::default();
        pane.finish(Ok(sample_extraction()));

        pane.finish_voiceovers(Ok(vec![
            (0, audio(2.0)),
            (1, audio(3.0)),
            (2, audio(4.0)),
        ]));
        assert!(pane.voiceovers.iter().all(Option::is_some));

        // Moving scene 2 up to index 1 keeps its voiceover attached.
        let _ = pane.apply_local(ThreadsMsg::SelectFrame(2));
        let _ = pane.apply_local(ThreadsMsg::MoveFrameUp(2));
        assert_eq!(pane.selected, 1);
        assert_eq!(pane.extraction.as_ref().unwrap().frames[1].id, "c2");
        assert_eq!(pane.voiceovers[1].as_ref().unwrap().duration_seconds, 4.0);
        assert_eq!(pane.voiceovers[2].as_ref().unwrap().duration_seconds, 3.0);

        // Editing text invalidates only the edited scene.
        let _ = pane.apply_local(ThreadsMsg::FrameTextEdited("Đã sửa".into()));
        assert!(pane.voiceovers[1].is_none());
        assert!(pane.voiceovers[0].is_some());
        assert!(pane.voiceovers[2].is_some());

        // Changing voice speed invalidates all generated voiceovers.
        let _ = pane.apply_local(ThreadsMsg::VoiceRateChanged(2));
        assert!(pane.voiceovers.iter().all(Option::is_none));

        // Re-populate and verify switching voice speaker also invalidates all.
        pane.finish_voiceovers(Ok(vec![
            (0, audio(2.0)),
            (1, audio(3.0)),
            (2, audio(4.0)),
        ]));
        let _ = pane.apply_local(ThreadsMsg::VoiceChanged(1));
        assert_eq!(pane.voice, 1);
        assert!(pane.voiceovers.iter().all(Option::is_none));
    }

    #[test]
    fn pool_add_custom_remove_and_sort_work_consistently() {
        let mut pane = ThreadsPane::default();
        pane.finish(Ok(sample_extraction()));

        // Pool still has c3.
        assert_eq!(
            tgs::remaining_pool_count(pane.extraction.as_ref().unwrap(), pane.sort),
            1
        );
        let _ = pane.apply_local(ThreadsMsg::AddNextComment);
        assert_eq!(pane.extraction.as_ref().unwrap().frames.len(), 4);
        assert_eq!(pane.extraction.as_ref().unwrap().frames[3].id, "c3");

        // Adding a custom frame inherits author identity and updates selection.
        let _ = pane.apply_local(ThreadsMsg::AddCustomFrame);
        assert_eq!(pane.extraction.as_ref().unwrap().frames.len(), 5);
        assert_eq!(pane.selected, 4);
        assert_eq!(
            pane.extraction.as_ref().unwrap().frames[4].username,
            "op.official"
        );

        let _ = pane.apply_local(ThreadsMsg::RemoveFrame(1));
        assert_eq!(pane.extraction.as_ref().unwrap().frames.len(), 4);
        assert_eq!(pane.selected, 3);

        // Switching sort order to Replies re-ranks comments from the cached pool.
        let _ = pane.apply_local(ThreadsMsg::SortChanged(1));
        let frames = &pane.extraction.as_ref().unwrap().frames;
        assert_eq!(frames[1].id, "c2");
        assert_eq!(frames[2].id, "c1");
    }

    #[test]
    fn per_scene_meme_theme_and_bgm_controls_update_state() {
        let mut pane = ThreadsPane::default();
        assert_eq!(pane.bgm_preset, BgmPreset::LoFiChill);
        assert_eq!(pane.card_theme, ThreadsCardTheme::Dark);

        pane.finish(Ok(sample_extraction()));
        let meme = PathBuf::from("reaction.png");
        pane.extraction.as_mut().unwrap().frames[1].meme_path = Some(meme.clone());

        // Move scene 1 up -> meme stays attached to scene c1 (now index 0).
        let _ = pane.apply_local(ThreadsMsg::MoveFrameUp(1));
        assert_eq!(
            pane.extraction.as_ref().unwrap().frames[0].meme_path.as_ref(),
            Some(&meme)
        );
        assert!(pane.extraction.as_ref().unwrap().frames[1].meme_path.is_none());

        // Apply to all scenes replicates scene 0's meme to every scene.
        let _ = pane.apply_local(ThreadsMsg::ApplyMemeToAll(0));
        assert!(
            pane.extraction
                .as_ref()
                .unwrap()
                .frames
                .iter()
                .all(|frame| frame.meme_path.as_ref() == Some(&meme))
        );

        // Removing meme on one scene clears only that scene.
        let _ = pane.apply_local(ThreadsMsg::RemoveSceneMeme(1));
        assert!(pane.extraction.as_ref().unwrap().frames[1].meme_path.is_none());
        assert_eq!(
            pane.extraction.as_ref().unwrap().frames[0].meme_path.as_ref(),
            Some(&meme)
        );

        // Theme, karaoke and BGM controls.
        let _ = pane.apply_local(ThreadsMsg::CardThemeChanged(1));
        assert_eq!(pane.card_theme, ThreadsCardTheme::Light);
        let _ = pane.apply_local(ThreadsMsg::KaraokeStyleChanged(2));
        assert_eq!(pane.karaoke_style, KaraokeStyle::NeonCyan);
        let _ = pane.apply_local(ThreadsMsg::BgmPresetChanged(2));
        assert_eq!(pane.bgm_preset, BgmPreset::StoryUpbeat);
        let _ = pane.apply_local(ThreadsMsg::BgmVolumeChanged(0.25));
        assert!((pane.bgm_volume - 0.25).abs() < 1e-6);
    }
}
