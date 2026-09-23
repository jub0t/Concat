// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The speech sheet: a title's words, or any words, read aloud.
//!
//! It opens on the selected title's words when a title is selected, else
//! on a blank script to be read at the playhead. The voice is the one
//! chosen last time. The synthesiser runs on a worker and reports as
//! [`SpeechMsg::Progress`], ending as [`SpeechMsg::Finished`] with the
//! written file, which lands in the bin and on the timeline.
//!
//! The model comes first and decides the rest: which voices are offered,
//! and which dials. Kokoro reads with built-in speakers at a speed and
//! with a pause of your choosing; Pocket reads in a recording, at a speed,
//! with a pause, and at a quality that is its flow steps; Chatterbox reads
//! at its own pace and knows the expression tags. Either of the two that
//! read in a recording can read in a sample voice instead of a named one:
//! a recording from the bin, chosen by its waveform, from a point in it of
//! the sheet's choosing. The sheet shows each model its own controls rather
//! than the few every model shares, which is why the pane carries every
//! dial and the family says which are read.

use std::sync::Arc;

use concat_host::media::{self, MediaSummary};
use concat_project::Command;
use concat_project::model::{ClipKind, MediaItem, MediaOrigin};
use concat_speech::tts::{
    CHATTERBOX_CLONE, Family, POCKET_CLONE, Reference, VoiceInfo, family_of, is_clone,
};
use slint::SharedString;

use crate::format::wave_path;
use crate::host::{on_ui_in_project, spawn_in_project};
use crate::i18n::{t, tf};
use crate::panes::Msg;
use crate::panes::captions::CHARS_PER_SECOND;
use crate::panes::settings::installed;
use crate::studio::Studio;
use crate::ui::SpeechSheetData;

/// The default Kokoro speaker: `af_heart`.
const DEFAULT_VOICE: i32 = 3;
/// The default Pocket voice: Bria, the bundle's first recording.
const DEFAULT_POCKET_VOICE: i32 = 1000;
/// The default Chatterbox voice: the same recording, read by Chatterbox.
const DEFAULT_CHATTERBOX_VOICE: i32 = 2000;
/// The speeds a voice can be asked for, as the rate multiplier the sheet's
/// slider runs over; 1 is the voice's own.
const SPEEDS: (f32, f32) = (0.5, 2.0);
/// Pocket's flow steps behind the sheet's three qualities: quick, the
/// engine's own, and fine.
const STEPS: [i32; 3] = [3, 5, 8];
/// The breath between sentences the sheet opens on: sherpa's own.
const DEFAULT_PAUSES: f32 = 0.2;

/// Everything that can happen to the speech sheet.
#[derive(Clone, Debug)]
pub enum SpeechMsg {
    /// The tray's Speak tool, or the menu.
    Open,
    Close,
    TextEdited(String),
    VoiceChanged(i32),
    ModelChanged(i32),
    /// The rate, `0.5..=2`.
    SpeedChanged(f32),
    /// Pocket's quality: a row of [`STEPS`].
    QualityChanged(i32),
    /// The breath between sentences, `0..=1`.
    PausesChanged(f32),
    /// Seconds into the sample the voice is taken from.
    ReferenceStartChanged(f32),
    /// Read in a sample from the bin rather than a named voice.
    UseSampleChanged(bool),
    /// Which sample: a row of [`SpeechPane::samples`].
    SampleChanged(i32),
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
    /// Row in the chosen model's voices; see [`SpeechPane::offered`].
    pub voice: usize,
    /// Row in the installed voice model list.
    pub model: usize,
    /// The family of the chosen model, which is which voices are offered
    /// and which of the dials below are read.
    pub family: Family,
    /// The rate, `0.5..=2`; Kokoro and Pocket.
    pub speed: f32,
    /// Row of [`STEPS`]; Pocket.
    pub quality: usize,
    /// The breath between sentences, `0..=1`; Kokoro and Pocket.
    pub pauses: f32,
    /// Reading in a sample from the bin rather than a named voice; Pocket
    /// and Chatterbox, the two that read in a recording.
    pub use_sample: bool,
    /// Row of [`SpeechPane::samples`]: which recording.
    pub sample: usize,
    /// Seconds into the sample the voice is taken from: the first ten
    /// seconds from there are what the model listens to, and the start of
    /// a recording is not always where the speaking is.
    pub reference_start: f64,
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
                let family = installed
                    .get(model)
                    .map(|model| family_of(&model.id))
                    .unwrap_or(Family::Kokoro);
                *self = SpeechPane {
                    open: true,
                    clip: title.as_ref().map(|clip| clip.id.clone()),
                    text: title
                        .and_then(|clip| clip.text.map(|text| text.content))
                        .unwrap_or_default(),
                    voice: 0,
                    model,
                    family,
                    speed: 1.0,
                    quality: 1,
                    pauses: DEFAULT_PAUSES,
                    use_sample: false,
                    sample: 0,
                    reference_start: 0.0,
                    speakers: std::mem::take(&mut self.speakers),
                    ..SpeechPane::default()
                };
                // The voice chosen last time, if the model reads with it;
                // and the selected clip's recording as the sample to start
                // from, since that is the voice most likely wanted.
                let wanted = studio.prefs.tts_voice;
                self.voice = self.voice_row(wanted);
                self.sample = self.default_sample(studio);
            }
            SpeechMsg::Close => self.open = false,
            SpeechMsg::TextEdited(text) => self.text = text,
            SpeechMsg::VoiceChanged(index) => self.voice = index.max(0) as usize,
            SpeechMsg::ModelChanged(index) => {
                self.model = index.max(0) as usize;
                // Another model may read with other voices: the list is
                // rebuilt, and the row points at a voice on it.
                let chosen = self.offered().get(self.voice).map(|voice| voice.id);
                self.family = installed(&studio.settings.voices)
                    .get(self.model)
                    .map(|model| family_of(&model.id))
                    .unwrap_or(Family::Kokoro);
                self.voice = self.voice_row(chosen);
            }
            SpeechMsg::SpeedChanged(speed) => {
                self.speed = if speed.is_finite() {
                    speed.clamp(SPEEDS.0, SPEEDS.1)
                } else {
                    1.0
                }
            }
            SpeechMsg::QualityChanged(index) => {
                self.quality = (index.max(0) as usize).min(STEPS.len() - 1)
            }
            SpeechMsg::PausesChanged(pauses) => {
                self.pauses = if pauses.is_finite() {
                    pauses.clamp(0.0, 1.0)
                } else {
                    DEFAULT_PAUSES
                }
            }
            SpeechMsg::ReferenceStartChanged(seconds) => {
                self.reference_start = if seconds.is_finite() {
                    f64::from(seconds.max(0.0))
                } else {
                    0.0
                }
            }
            SpeechMsg::UseSampleChanged(on) => self.use_sample = on,
            SpeechMsg::SampleChanged(index) => {
                self.sample = index.max(0) as usize;
                self.reference_start = 0.0;
            }
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
                        // Marked as read aloud: the bin shelves it under
                        // Generated › Speech, apart from the imports.
                        let mut item = summary.to_new_media();
                        item.origin = Some(MediaOrigin::Speech);
                        let created = studio.apply(Command::AddMedia { item });
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
        // A sample voice is a recording of the sheet's choosing, handed over
        // as the request's reference under the family's own id for one.
        // Otherwise the voice is one the model names - and Chatterbox may
        // name none, since its stock voices are Pocket's recordings and
        // are only there while Pocket is.
        let (voice, reference) = if self.use_sample {
            let Some(voice) = self.sample_voice() else {
                self.message = t("This model reads only with its own speakers");
                return;
            };
            let Some(sample) = self.sample_media(studio).cloned() else {
                self.message = t("Pick a sample with a voice in it");
                return;
            };
            let start = self.reference_offset(sample.duration.unwrap_or(0.0));
            (
                voice,
                Some(Reference {
                    path: sample.path,
                    start,
                }),
            )
        } else {
            let Some(voice) = self.offered().get(self.voice).map(|speaker| speaker.id) else {
                self.message = if self.family == Family::Chatterbox {
                    t(
                        "Chatterbox has no voice of its own: turn on Use sample voice, or \
                         download Pocket TTS for its two recordings",
                    )
                } else {
                    t("No voice to read with")
                };
                return;
            };
            (voice, None)
        };
        let Some(project) = studio
            .session
            .as_ref()
            .map(|session| session.path().to_owned())
        else {
            return;
        };
        // Remembered: the voice chosen is the voice wanted next time. A
        // sample is a recording in one project, and is not.
        if !self.use_sample {
            studio.prefs.tts_voice = Some(voice);
            studio.prefs.save(&studio.host.dirs);
        }
        self.landing = self
            .clip
            .as_ref()
            .and_then(|id| studio.clip(id))
            .map(|clip| clip.start)
            .unwrap_or(f64::from(studio.playhead));
        let request = concat_speech::tts::SpeakRequest {
            model_id: model,
            voice,
            reference,
            text,
            speed: self.rate(),
            pauses: self.reads_at_a_pace().then_some(self.pauses),
            steps: (self.family == Family::Pocket)
                .then(|| STEPS[self.quality.min(STEPS.len() - 1)]),
            project,
        };
        let dirs = studio.host.dirs.clone();
        let speech = Arc::clone(&studio.host.speech);
        self.running = true;
        self.progress = 0.0;
        self.message.clear();
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                let spoken = speech.speak(&dirs, &request, move |fraction| {
                    on_ui_in_project(epoch, move |studio, _, _| {
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
    fn estimate(&self, studio: &Studio) -> String {
        let chars = self.text.trim().chars().count();
        if chars == 0 {
            return t("Nothing to read yet");
        }
        let seconds = chars as f32 / CHARS_PER_SECOND / self.rate();
        let whole = seconds.round() as i32;
        let voice = if self.use_sample {
            self.sample_media(studio)
                .map(|sample| sample.name.clone())
                .unwrap_or_default()
        } else {
            self.offered()
                .get(self.voice)
                .map(|speaker| voice_label(&speaker.name).0)
                .unwrap_or_default()
        };
        tf(
            "About {0}:{1} in {2} · {3} characters",
            &[&(whole / 60), &format!("{:02}", whole % 60), &voice, &chars],
        )
    }

    /// Whether the chosen model reads at a speed and with a pause of the
    /// sheet's choosing. Chatterbox does neither: it reads at its own
    /// pace, and the sheet does not offer what the engine would ignore.
    pub fn reads_at_a_pace(&self) -> bool {
        self.family != Family::Chatterbox
    }

    /// The rate the request carries: the slider's, or 1 for a model that
    /// reads at its own.
    fn rate(&self) -> f32 {
        if self.reads_at_a_pace() {
            self.speed
        } else {
            1.0
        }
    }

    /// The id under which the chosen model reads in a recording of the
    /// sheet's choosing; None for a model that reads only with its own
    /// speakers.
    fn sample_voice(&self) -> Option<i32> {
        match self.family {
            Family::Kokoro => None,
            Family::Pocket => Some(POCKET_CLONE),
            Family::Chatterbox => Some(CHATTERBOX_CLONE),
        }
    }

    /// The recordings a voice can be taken from: everything in the bin with
    /// sound in it, in the bin's order.
    pub fn samples<'a>(&self, studio: &'a Studio) -> Vec<&'a MediaItem> {
        studio
            .project()
            .media
            .iter()
            .filter(|item| item.has_audio)
            .collect()
    }

    /// The chosen sample.
    fn sample_media<'a>(&self, studio: &'a Studio) -> Option<&'a MediaItem> {
        self.samples(studio).get(self.sample).copied()
    }

    /// The row to open the sample list on: the selected clip's recording,
    /// when one clip with sound in it is selected, else the first.
    fn default_sample(&self, studio: &Studio) -> usize {
        studio
            .sole_selection()
            .and_then(|id| studio.clip(&id))
            .filter(|clip| clip.kind == ClipKind::Video || clip.kind == ClipKind::Audio)
            .and_then(|clip| {
                self.samples(studio)
                    .iter()
                    .position(|item| item.id == clip.media_id)
            })
            .unwrap_or(0)
    }

    /// How far into a recording of `duration` the voice is taken from: what
    /// the sheet asked for, held short of the end so there is always at
    /// least a second of sound left to listen to.
    fn reference_offset(&self, duration: f64) -> f64 {
        self.reference_start.clamp(0.0, (duration - 1.0).max(0.0))
    }

    /// The voices the chosen model reads with, in table order.
    pub fn offered(&self) -> Vec<&VoiceInfo> {
        // The family's clone id is not a voice to pick from a list: it is
        // what a sample is sent under, and the sample switch offers it.
        self.speakers
            .iter()
            .filter(|speaker| speaker.family == self.family && !is_clone(speaker.id))
            .collect()
    }

    /// The row of `wanted` among the offered voices, or of the family's
    /// default, or the first.
    fn voice_row(&self, wanted: Option<i32>) -> usize {
        let offered = self.offered();
        let default = match self.family {
            Family::Kokoro => DEFAULT_VOICE,
            Family::Pocket => DEFAULT_POCKET_VOICE,
            Family::Chatterbox => DEFAULT_CHATTERBOX_VOICE,
        };
        wanted
            .and_then(|id| offered.iter().position(|voice| voice.id == id))
            .or_else(|| offered.iter().position(|voice| voice.id == default))
            .unwrap_or(0)
    }

    /// The voice list's rows: each offered voice's name.
    pub fn speaker_rows(&self) -> Vec<SharedString> {
        self.offered()
            .iter()
            .map(|speaker| voice_label(&speaker.name).0.into())
            .collect()
    }

    /// The voice list's second lines: each voice's accent and gender, or
    /// where a Pocket voice comes from.
    pub fn speaker_detail_rows(&self) -> Vec<SharedString> {
        self.offered()
            .iter()
            .map(|speaker| voice_label(&speaker.name).1.into())
            .collect()
    }

    /// The model list's second lines: what each model is for, in a few
    /// words, since which one is chosen decides everything under it.
    pub fn model_detail_rows(&self, studio: &Studio) -> Vec<SharedString> {
        installed(&studio.settings.voices)
            .iter()
            .map(|model| match family_of(&model.id) {
                Family::Kokoro => t("Built-in speakers · fast"),
                Family::Pocket => t("Any voice, from a recording"),
                Family::Chatterbox => t("Studio voice · slow"),
            })
            .map(SharedString::from)
            .collect()
    }

    /// The sample list's rows: each recording's name.
    pub fn sample_rows(&self, studio: &Studio) -> Vec<SharedString> {
        self.samples(studio)
            .iter()
            .map(|item| item.name.as_str().into())
            .collect()
    }

    /// The sample list's pictures: each recording's envelope as path
    /// commands, the same drawing the bin's card and the timeline's clip
    /// make of it, or empty while the peaks have not been read yet.
    pub fn sample_wave_rows(&self, studio: &Studio) -> Vec<SharedString> {
        self.samples(studio)
            .iter()
            .map(|item| match studio.peaks.get(&item.id) {
                Some(peaks) => {
                    wave_path(peaks, 0.0, item.duration.unwrap_or(0.0) as f32, 64, 0.75).into()
                }
                None => SharedString::new(),
            })
            .collect()
    }

    /// The sample list's second column: how long each recording is.
    pub fn sample_detail_rows(&self, studio: &Studio) -> Vec<SharedString> {
        self.samples(studio)
            .iter()
            .map(|item| {
                let whole = item.duration.unwrap_or(0.0).round() as i64;
                format!("{}:{:02}", whole / 60, whole % 60).into()
            })
            .collect()
    }

    /// The sheet as Slint shows it.
    pub fn data(&self, studio: &Studio) -> SpeechSheetData {
        let sample = self.sample_media(studio);
        let length = sample.and_then(|item| item.duration).unwrap_or(0.0);
        SpeechSheetData {
            open: self.open,
            text: self.text.as_str().into(),
            voice: self.voice as i32,
            model: self.model as i32,
            family: match self.family {
                Family::Kokoro => 0,
                Family::Pocket => 1,
                Family::Chatterbox => 2,
            },
            speed: self.speed,
            quality: self.quality as i32,
            pauses: self.pauses,
            samples_offered: self.sample_voice().is_some(),
            use_sample: self.use_sample,
            sample: self.sample as i32,
            sample_name: sample
                .map(|item| item.name.as_str())
                .unwrap_or_default()
                .into(),
            reference_length: length as f32,
            reference_start: self.reference_offset(length) as f32,
            running: self.running,
            progress: self.progress,
            ready: !installed(&studio.settings.voices).is_empty(),
            placement: if self.clip.is_some() {
                "at the title"
            } else {
                "at the playhead"
            }
            .into(),
            estimate: self.estimate(studio).into(),
            message: self.message.as_str().into(),
        }
    }
}

/// `tag` written into `text` at byte offset `at` - the caret, as the script
/// box reports it - with a space on whichever side is against a word, so
/// `[laugh]` lands as a word of its own and never as the tail of one. An
/// offset inside a character steps back to the character's start; one past
/// the end is the end. A trailing space is always written when the tag is
/// not already followed by one, so the caller can put the caret after it.
pub fn insert_tag(text: &str, tag: &str, at: usize) -> String {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let (before, after) = text.split_at(at);
    let lead = if before.is_empty() || before.ends_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    let trail = if after.starts_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    format!("{before}{lead}{tag}{trail}{after}")
}

/// "af_heart" as a person would say it: the name, and the accent and
/// gender its prefix encodes. A Pocket voice says where it comes from.
fn voice_label(name: &str) -> (String, String) {
    if name == "pocket_clone" || name == "chatterbox_clone" {
        return (
            t("The selected clip's voice"),
            t("Reads in the voice heard in the clip selected on the timeline"),
        );
    }
    if let Some(rest) = name.strip_prefix("pocket_") {
        let mut chars = rest.chars();
        let title = match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        };
        return (title, t("A recording that comes with Pocket TTS"));
    }
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
    use super::{insert_tag, voice_label};

    /// A tag lands as a word of its own wherever the caret is: spaced off a
    /// word on either side, not doubled against a space already there, and
    /// snapped to a character when the caret is reported inside one.
    #[test]
    fn a_tag_lands_as_its_own_word() {
        assert_eq!(insert_tag("", "[laugh]", 0), "[laugh] ");
        assert_eq!(insert_tag("hello", "[laugh]", 5), "hello [laugh] ");
        assert_eq!(insert_tag("hello world", "[sigh]", 5), "hello [sigh] world");
        assert_eq!(insert_tag("hello world", "[sigh]", 6), "hello [sigh] world");
        assert_eq!(
            insert_tag("hello world", "[sigh]", 3),
            "hel [sigh] lo world"
        );
        assert_eq!(insert_tag("hi", "[chuckle]", 0), "[chuckle] hi");
        // Past the end is the end; inside "é" (two bytes) steps back to it.
        assert_eq!(insert_tag("hi", "[laugh]", 40), "hi [laugh] ");
        assert_eq!(insert_tag("café", "[laugh]", 4), "caf [laugh] é");
        assert_eq!(insert_tag("café ", "[laugh]", 6), "café [laugh] ");
    }

    #[test]
    fn a_voice_name_reads_as_a_person_would_say_it() {
        let (title, detail) = voice_label("af_heart");
        assert_eq!(title, "Heart");
        assert!(detail.contains("American"));
        assert!(detail.contains("female"));
        assert_eq!(voice_label("nova").0, "Nova");
        assert_eq!(voice_label("pocket_bria").0, "Bria");
        assert!(voice_label("pocket_bria").1.contains("Pocket"));
        assert!(voice_label("pocket_clone").0.contains("clip"));
        assert!(voice_label("chatterbox_clone").0.contains("clip"));
    }
}
