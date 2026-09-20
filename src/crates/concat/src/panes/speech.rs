// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The speech sheet: a title's words, or any words, read aloud.
//!
//! It opens on the selected title's words when a title is selected, else
//! on a blank script to be read at the playhead. The voice is the one
//! chosen last time. The synthesiser runs on a worker and reports as
//! [`SpeechMsg::Progress`], ending as [`SpeechMsg::Finished`] with the
//! written file, which lands in the bin and on the timeline.

use std::sync::Arc;

use concat_host::media::{self, MediaSummary};
use concat_project::Command;
use concat_project::model::ClipKind;
use concat_speech::tts::VoiceInfo;
use slint::SharedString;

use crate::host::{on_ui, spawn};
use crate::i18n::{t, tf};
use crate::panes::Msg;
use crate::panes::captions::CHARS_PER_SECOND;
use crate::panes::settings::installed;
use crate::studio::Studio;
use crate::ui::SpeechSheetData;

/// The default Kokoro speaker: `af_heart`.
const DEFAULT_VOICE: i32 = 3;
/// The paces the speech sheet offers, as the voice's rate multiplier.
const PACES: [f32; 3] = [0.85, 1.0, 1.15];

/// Everything that can happen to the speech sheet.
#[derive(Clone, Debug)]
pub enum SpeechMsg {
    /// The tray's Speak tool, or the menu.
    Open,
    Close,
    TextEdited(String),
    VoiceChanged(i32),
    ModelChanged(i32),
    PaceChanged(i32),
    /// Read the script.
    Begin,
    Cancel,
    /// The synthesiser's worker reporting where it is, `0..=1`.
    Progress(f32),
    /// The synthesiser's worker is done: the file it wrote, probed, or
    /// why not.
    Finished(Box<Result<MediaSummary, String>>),
}

/// The speech sheet's state.
#[derive(Default)]
pub struct SpeechPane {
    pub open: bool,
    /// The title the words came from, and where the sound lands. None
    /// reads a script of its own at the playhead.
    pub clip: Option<String>,
    pub text: String,
    /// Row in `speakers`.
    pub voice: usize,
    /// Row in the installed voice model list.
    pub model: usize,
    /// 0 slower, 1 natural, 2 faster.
    pub pace: usize,
    pub running: bool,
    pub progress: f32,
    pub message: String,
    /// Where the running read will land on the timeline, in seconds.
    landing: f64,
    /// Who can speak, from the voice model on disk.
    pub speakers: Vec<VoiceInfo>,
}

impl SpeechPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: SpeechMsg, studio: &mut Studio) {
        match msg {
            SpeechMsg::Open => {
                let title = studio
                    .sole_selection()
                    .and_then(|id| studio.clip(&id))
                    .filter(|clip| clip.kind == ClipKind::Text)
                    .cloned();
                let installed = installed(&studio.settings.voices);
                let model = installed.iter().position(|model| model.active).unwrap_or(0);
                let wanted = studio.prefs.tts_voice.unwrap_or(DEFAULT_VOICE);
                let voice = self
                    .speakers
                    .iter()
                    .position(|speaker| speaker.id == wanted)
                    .unwrap_or(0);
                *self = SpeechPane {
                    open: true,
                    clip: title.as_ref().map(|clip| clip.id.clone()),
                    text: title
                        .and_then(|clip| clip.text.map(|text| text.content))
                        .unwrap_or_default(),
                    voice,
                    model,
                    pace: 1,
                    speakers: std::mem::take(&mut self.speakers),
                    ..SpeechPane::default()
                };
            }
            SpeechMsg::Close => self.open = false,
            SpeechMsg::TextEdited(text) => self.text = text,
            SpeechMsg::VoiceChanged(index) => self.voice = index.max(0) as usize,
            SpeechMsg::ModelChanged(index) => self.model = index.max(0) as usize,
            SpeechMsg::PaceChanged(index) => self.pace = (index.max(0) as usize).min(2),
            SpeechMsg::Begin => self.run(studio),
            SpeechMsg::Cancel => {
                studio.host.speech.cancel();
                self.running = false;
                self.open = false;
            }
            SpeechMsg::Progress(fraction) => self.progress = fraction.clamp(0.0, 1.0),
            SpeechMsg::Finished(result) => {
                self.running = false;
                match *result {
                    Ok(summary) => {
                        let created = studio.apply(Command::AddMedia {
                            item: summary.to_new_media(),
                        });
                        let media_id = created.or_else(|| {
                            studio
                                .project()
                                .media
                                .iter()
                                .find(|item| item.path == summary.path)
                                .map(|item| item.id.clone())
                        });
                        self.open = false;
                        if let Some(media_id) = media_id {
                            studio.apply(Command::AddClipAtFirstFree {
                                media_id,
                                start: self.landing,
                            });
                            studio.notify(&t("Voice added to the timeline"), false);
                        }
                    }
                    // Asked for: the sheet is already on its way down.
                    Err(error) if error.contains("cancel") => self.open = false,
                    Err(error) => self.message = error,
                }
            }
        }
    }

    /// Reads the script on a worker: the WAV lands in the bin and on the
    /// timeline, at the title's start or at the playhead.
    fn run(&mut self, studio: &mut Studio) {
        let text = self.text.trim().to_owned();
        if text.is_empty() {
            self.message = t("Nothing to read yet");
            return;
        }
        let Some(model) = installed(&studio.settings.voices)
            .get(self.model)
            .map(|model| model.id.clone())
        else {
            self.message = t("Download a voice model in Settings › Speech first");
            return;
        };
        let Some(voice) = self.speakers.get(self.voice).map(|speaker| speaker.id) else {
            self.message = t("No voice to read with");
            return;
        };
        let Some(project) = studio
            .session
            .as_ref()
            .map(|session| session.path().to_owned())
        else {
            return;
        };
        // Remembered: the voice chosen is the voice wanted next time.
        studio.prefs.tts_voice = Some(voice);
        studio.prefs.save(&studio.host.dirs);
        self.landing = self
            .clip
            .as_ref()
            .and_then(|id| studio.clip(id))
            .map(|clip| clip.start)
            .unwrap_or(f64::from(studio.playhead));
        let request = concat_speech::tts::SpeakRequest {
            model_id: model,
            voice,
            text,
            speed: PACES[self.pace.min(2)],
            project,
        };
        let dirs = studio.host.dirs.clone();
        let speech = Arc::clone(&studio.host.speech);
        self.running = true;
        self.progress = 0.0;
        self.message.clear();
        spawn(
            move || {
                let spoken = speech.speak(&dirs, &request, |fraction| {
                    on_ui(move |studio, _, _| {
                        studio.handle(Msg::Speech(SpeechMsg::Progress(fraction)));
                    });
                })?;
                let summary = media::probe(&spoken.path)?;
                Ok::<_, String>(summary)
            },
            |studio, _, _, result| {
                studio.handle(Msg::Speech(SpeechMsg::Finished(Box::new(result))))
            },
        );
    }

    /// What the sheet's script would take to say, for the line under it.
    fn estimate(&self) -> String {
        let chars = self.text.trim().chars().count();
        if chars == 0 {
            return t("Nothing to read yet");
        }
        let seconds = chars as f32 / CHARS_PER_SECOND / PACES[self.pace.min(2)];
        let whole = seconds.round() as i32;
        let voice = self
            .speakers
            .get(self.voice)
            .map(|speaker| voice_label(&speaker.name).0)
            .unwrap_or_default();
        tf(
            "About {0}:{1} in {2} · {3} characters",
            &[&(whole / 60), &format!("{:02}", whole % 60), &voice, &chars],
        )
    }

    /// The voice list's rows: each speaker's name.
    pub fn speaker_rows(&self) -> Vec<SharedString> {
        self.speakers
            .iter()
            .map(|speaker| voice_label(&speaker.name).0.into())
            .collect()
    }

    /// The voice list's second lines: each speaker's accent and gender.
    pub fn speaker_detail_rows(&self) -> Vec<SharedString> {
        self.speakers
            .iter()
            .map(|speaker| voice_label(&speaker.name).1.into())
            .collect()
    }

    /// The sheet as Slint shows it.
    pub fn data(&self, studio: &Studio) -> SpeechSheetData {
        SpeechSheetData {
            open: self.open,
            text: self.text.as_str().into(),
            voice: self.voice as i32,
            model: self.model as i32,
            pace: self.pace as i32,
            running: self.running,
            progress: self.progress,
            ready: !installed(&studio.settings.voices).is_empty(),
            placement: if self.clip.is_some() {
                "at the title"
            } else {
                "at the playhead"
            }
            .into(),
            estimate: self.estimate().into(),
            message: self.message.as_str().into(),
        }
    }
}

/// "af_heart" as a person would say it: the name, and the accent and
/// gender its prefix encodes.
fn voice_label(name: &str) -> (String, String) {
    let (prefix, rest) = name.split_once('_').unwrap_or(("", name));
    let mut chars = rest.chars();
    let title = match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    };
    let accent = match prefix.chars().next() {
        Some('a') => t("American"),
        Some('b') => t("British"),
        Some('e') => t("Spanish"),
        Some('f') => t("French"),
        Some('h') => t("Hindi"),
        Some('i') => t("Italian"),
        Some('j') => t("Japanese"),
        Some('p') => t("Portuguese"),
        Some('z') => t("Chinese"),
        _ => String::new(),
    };
    let gender = match prefix.chars().nth(1) {
        Some('f') => t("female"),
        Some('m') => t("male"),
        _ => String::new(),
    };
    let detail = [accent, gender]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    (title, detail)
}

#[cfg(test)]
mod tests {
    use super::voice_label;

    #[test]
    fn a_voice_name_reads_as_a_person_would_say_it() {
        let (title, detail) = voice_label("af_heart");
        assert_eq!(title, "Heart");
        assert!(detail.contains("American"));
        assert!(detail.contains("female"));
        assert_eq!(voice_label("nova").0, "Nova");
    }
}
