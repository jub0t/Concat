// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The Threads quick-video entry on the launch screen.

use std::path::PathBuf;

use crate::studio::Studio;
use crate::tgs::{self, CommentSort, Extraction};
use crate::ui::{ThreadsData, ThreadsFrameData};

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
    VoiceRateChanged(i32),
    GenerateVoiceovers,
    ChooseMeme,
    RemoveMeme,
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
    pub voice_rate: usize,
    pub voiceovers: Vec<Option<tgs::VoiceoverAudio>>,
    pub generating_voiceovers: bool,
    pub meme_path: Option<PathBuf>,
    pub scene_revision: i32,
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
            // Default to 1.15× (+15%) so TikTok narration feels brisk out of the box.
            voice_rate: 1,
            voiceovers: Vec::new(),
            generating_voiceovers: false,
            meme_path: None,
            scene_revision: 0,
        }
    }
}

impl ThreadsPane {
    /// Handles pure in-memory scene editing, selection, sorting, and speed
    /// state transitions that do not touch the host or studio.
    fn apply_local(&mut self, msg: ThreadsMsg) -> Result<(), ThreadsMsg> {
        match msg {
            ThreadsMsg::Open => {
                self.open = true;
                self.error.clear();
            }
            ThreadsMsg::Close if !self.running => self.open = false,
            ThreadsMsg::Close => {}
            ThreadsMsg::UrlEdited(url) => {
                self.url = url;
                self.error.clear();
                self.extraction = None;
                self.voiceovers.clear();
                self.selected = 0;
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
                extraction.frames.push(tgs::Frame {
                    id: format!("custom-{custom_count}"),
                    label: crate::i18n::t("threads.customScene"),
                    author: default_author,
                    text: String::new(),
                    likes: String::new(),
                    replies: String::new(),
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
                    self.scene_revision += 1;
                }
            }
            ThreadsMsg::VoiceRateChanged(index) => {
                if self.running {
                    return Ok(());
                }
                let next = (index.max(0) as usize).min(tgs::VOICE_RATES.len() - 1);
                if next != self.voice_rate {
                    self.voice_rate = next;
                    for slot in &mut self.voiceovers {
                        *slot = None;
                    }
                    self.error.clear();
                }
            }
            ThreadsMsg::RemoveMeme => self.meme_path = None,
            other => return Err(other),
        }
        Ok(())
    }

    pub fn update(&mut self, msg: ThreadsMsg, studio: &mut Studio) {
        let Err(msg) = self.apply_local(msg) else {
            return;
        };
        match msg {
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
                self.stage = "Connecting to the TGS extractor".to_owned();
                self.error.clear();
                self.extraction = None;
                self.voiceovers.clear();
                self.selected = 0;
                self.generating_voiceovers = false;
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
                let missing: Vec<(usize, tgs::Frame)> = extraction
                    .frames
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| {
                        self.voiceovers
                            .get(*idx)
                            .and_then(Option::as_ref)
                            .is_none()
                    })
                    .map(|(idx, frame)| (idx, frame.clone()))
                    .collect();
                if missing.is_empty() {
                    return;
                }
                if missing
                    .iter()
                    .any(|(_, frame)| frame.text.trim().is_empty() && frame.author.trim().is_empty())
                {
                    self.error = crate::i18n::t("threads.emptySceneText");
                    return;
                }

                let project_id = extraction.project_id.clone();
                let (_, rate) = tgs::voice_rate_at(self.voice_rate);
                let rate = rate.to_owned();
                self.running = true;
                self.generating_voiceovers = true;
                self.stage = "Preparing voiceovers".to_owned();
                self.error.clear();
                crate::host::spawn_long(
                    move || {
                        let mut last = String::new();
                        tgs::generate_voiceovers(&project_id, &missing, &rate, move |stage| {
                            if stage == last {
                                return;
                            }
                            last.clone_from(&stage);
                            crate::host::on_ui(move |studio, _, _| {
                                studio.threads.stage = stage;
                            });
                        })
                    },
                    |studio, _, _, result| studio.threads.finish_voiceovers(result),
                );
            }
            ThreadsMsg::ChooseMeme => {
                if self.running {
                    return;
                }
                crate::host::pick_files_then(
                    crate::i18n::t("threads.chooseMeme"),
                    Some(("Images".to_owned(), &["png", "jpg", "jpeg", "webp"])),
                    |studio, _, _, paths| {
                        let Some(path) = paths.into_iter().next() else {
                            return;
                        };
                        match concat_host::media::probe(&path.to_string_lossy()) {
                            Ok(summary)
                                if summary.kind == concat_project::model::MediaKind::Image =>
                            {
                                studio.threads.meme_path = Some(path);
                                studio.threads.error.clear();
                            }
                            Ok(_) => {
                                studio.threads.error = crate::i18n::t("threads.memeUnsupported");
                            }
                            Err(error) => {
                                studio.threads.error = format!(
                                    "{}: {error}",
                                    crate::i18n::t("threads.memeUnsupported")
                                );
                            }
                        }
                    },
                );
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
                self.open = false;
                studio
                    .start
                    .begin_threads_creation(extraction, ready, self.meme_path.clone());
            }
            _ => {}
        }
    }

    fn finish(&mut self, result: Result<Extraction, String>) {
        self.running = false;
        self.generating_voiceovers = false;
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

        ThreadsData {
            open: self.open,
            url: self.url.as_str().into(),
            running: self.running,
            generating_voiceovers: self.generating_voiceovers,
            voiceovers_ready,
            meme_file: self
                .meme_path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
                .into(),
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
            voice_rate: self.voice_rate as i32,
            selected_index: selected_index as i32,
            selected_label: selected_frame
                .map(|frame| frame.label.as_str())
                .unwrap_or_default()
                .into(),
            selected_author: selected_frame
                .map(|frame| frame.author.as_str())
                .unwrap_or_default()
                .into(),
            selected_text: selected_frame
                .map(|frame| frame.text.as_str())
                .unwrap_or_default()
                .into(),
            selected_likes: selected_frame
                .map(|frame| frame.likes.as_str())
                .unwrap_or_default()
                .into(),
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
                            text: frame.text.as_str().into(),
                            likes: frame.likes.as_str().into(),
                            replies: frame.replies.as_str().into(),
                            voice_ready: voice.is_some(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tgs::{Extraction, Frame, VoiceoverAudio};

    fn sample_extraction() -> Extraction {
        let post_frame = Frame {
            id: "post".into(),
            label: "Post".into(),
            author: "OP".into(),
            text: "Bài gốc".into(),
            likes: "100".into(),
            replies: "10".into(),
        };
        let c1 = Frame {
            id: "c1".into(),
            label: "Comment 1".into(),
            author: "A".into(),
            text: "Bình luận 1".into(),
            likes: "50".into(),
            replies: "2".into(),
        };
        let c2 = Frame {
            id: "c2".into(),
            label: "Comment 2".into(),
            author: "B".into(),
            text: "Bình luận 2".into(),
            likes: "40".into(),
            replies: "25".into(),
        };
        let c3 = Frame {
            id: "c3".into(),
            label: "Comment 3".into(),
            author: "C".into(),
            text: "Bình luận 3".into(),
            likes: "30".into(),
            replies: "1".into(),
        };
        Extraction {
            title: "Test".into(),
            source_url: "https://www.threads.com/@op/post/1".into(),
            project_id: "proj-1".into(),
            frames: vec![post_frame.clone(), c1.clone(), c2.clone()],
            post_frame,
            pool_by_likes: vec![c1.clone(), c2.clone(), c3.clone()],
            pool_by_replies: vec![c2, c1, c3],
        }
    }

    fn audio(seconds: f64) -> VoiceoverAudio {
        VoiceoverAudio {
            bytes: vec![1, 2, 3],
            duration_seconds: seconds,
        }
    }

    #[test]
    fn reordering_and_editing_preserve_untouched_voiceovers() {
        let mut pane = ThreadsPane::default();
        pane.finish(Ok(sample_extraction()));
        assert_eq!(pane.voiceovers.len(), 3);

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

        // Editing the selected scene text invalidates only that scene's voiceover.
        let _ = pane.apply_local(ThreadsMsg::FrameTextEdited("Đã sửa".into()));
        assert!(pane.voiceovers[1].is_none());
        assert!(pane.voiceovers[0].is_some());
        assert!(pane.voiceovers[2].is_some());

        // Changing voice speed invalidates all generated voiceovers.
        let _ = pane.apply_local(ThreadsMsg::VoiceRateChanged(2));
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

        // Adding a custom frame and removing an earlier frame keeps selection valid.
        let _ = pane.apply_local(ThreadsMsg::AddCustomFrame);
        assert_eq!(pane.extraction.as_ref().unwrap().frames.len(), 5);
        assert_eq!(pane.selected, 4);

        let _ = pane.apply_local(ThreadsMsg::RemoveFrame(1));
        assert_eq!(pane.extraction.as_ref().unwrap().frames.len(), 4);
        assert_eq!(pane.selected, 3);

        // Switching sort order to Replies re-ranks comments from the cached pool.
        let _ = pane.apply_local(ThreadsMsg::SortChanged(1));
        let frames = &pane.extraction.as_ref().unwrap().frames;
        assert_eq!(frames[1].id, "c2");
        assert_eq!(frames[2].id, "c1");
    }
}
