// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The Threads quick-video entry on the launch screen.

use crate::studio::Studio;
use crate::tgs::{self, Extraction};
use crate::ui::{ThreadsData, ThreadsFrameData};

#[derive(Clone, Debug)]
pub enum ThreadsMsg {
    Open,
    Close,
    UrlEdited(String),
    Extract,
    GenerateVoiceovers,
    CreateProject,
}

#[derive(Default)]
pub struct ThreadsPane {
    pub open: bool,
    pub url: String,
    pub running: bool,
    pub stage: String,
    pub error: String,
    pub extraction: Option<Extraction>,
    pub voiceovers: Option<Vec<tgs::VoiceoverAudio>>,
    pub generating_voiceovers: bool,
}

impl ThreadsPane {
    pub fn update(&mut self, msg: ThreadsMsg, studio: &mut Studio) {
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
                self.voiceovers = None;
            }
            ThreadsMsg::Extract => {
                let url = self.url.trim().to_owned();
                if !(url.starts_with("https://www.threads.net/")
                    || url.starts_with("https://threads.net/")
                    || url.starts_with("https://www.threads.com/")
                    || url.starts_with("https://threads.com/"))
                {
                    self.error = "Enter a valid Threads post link.".to_owned();
                    return;
                }
                self.running = true;
                self.stage = "Connecting to the TGS extractor".to_owned();
                self.error.clear();
                self.extraction = None;
                self.voiceovers = None;
                self.generating_voiceovers = false;

                crate::host::spawn_long(
                    move || {
                        let mut last = String::new();
                        tgs::extract(&url, move |stage| {
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
                let Some(extraction) = self.extraction.clone() else {
                    return;
                };
                if self.running {
                    return;
                }
                self.running = true;
                self.generating_voiceovers = true;
                self.stage = "Preparing voiceovers".to_owned();
                self.error.clear();
                self.voiceovers = None;
                crate::host::spawn_long(
                    move || {
                        let mut last = String::new();
                        tgs::generate_voiceovers(&extraction, move |stage| {
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
            ThreadsMsg::CreateProject => {
                if let Some(extraction) = self.extraction.clone()
                    && let Some(voiceovers) = self.voiceovers.take()
                {
                    if voiceovers.len() != extraction.frames.len() {
                        self.error = "A voiceover is missing for one or more frames.".to_owned();
                        self.voiceovers = Some(voiceovers);
                        return;
                    }
                    self.open = false;
                    studio.start.begin_threads_creation(extraction, voiceovers);
                }
            }
        }
    }

    fn finish(&mut self, result: Result<Extraction, String>) {
        self.running = false;
        self.generating_voiceovers = false;
        self.stage.clear();
        match result {
            Ok(extraction) => {
                self.error.clear();
                self.extraction = Some(extraction);
                self.voiceovers = None;
            }
            Err(error) => {
                self.error = error;
                self.extraction = None;
            }
        }
    }

    fn finish_voiceovers(&mut self, result: Result<Vec<tgs::VoiceoverAudio>, String>) {
        self.running = false;
        self.generating_voiceovers = false;
        self.stage.clear();
        match result {
            Ok(voiceovers) => {
                self.error.clear();
                self.voiceovers = Some(voiceovers);
            }
            Err(error) => {
                self.error = error;
                self.voiceovers = None;
            }
        }
    }

    pub fn data(&self) -> ThreadsData {
        let extraction = self.extraction.as_ref();
        ThreadsData {
            open: self.open,
            url: self.url.as_str().into(),
            running: self.running,
            generating_voiceovers: self.generating_voiceovers,
            voiceovers_ready: self.voiceovers.is_some(),
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
            frames: slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(
                extraction
                    .map(|item| item.frames.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .map(|frame| ThreadsFrameData {
                        label: frame.label.as_str().into(),
                        author: frame.author.as_str().into(),
                        text: frame.text.as_str().into(),
                        likes: frame.likes.as_str().into(),
                    })
                    .collect::<Vec<_>>(),
            ))),
        }
    }
}
