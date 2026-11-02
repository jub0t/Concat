// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The Gemini engine of the captions sheet.
//!
//! The sheet's other engine is the local transcriber. This one sends the
//! clip's sound, in windows, to Gemini and gets segments back that go
//! through the same path as the transcriber's: [`Segment`]s relative to the
//! clip's window, mapped onto the timeline by the sheet. Everything here is
//! the part that is not the window: the state, the languages, the audio
//! reading and the conversion.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use concat_gemini::job::{self, JobOptions, Mode, Stage};
use concat_gemini::{ApiKey, Client, Cue, GeminiError};
use concat_media::{AudioDecoder, AudioOptions, SampleFormat};
use concat_project::caption_fx::{self, Reveal, TimedWord};
use concat_speech::transcribe::Segment;

use crate::i18n::{t, tf};

/// The model the sheet starts on: the transcription model, which times
/// every word.
pub const DEFAULT_MODEL: &str = "gemini-3.5-transcribe";

/// The speech languages the sheet offers: the name it is shown under, in
/// its own script, and the BCP-47 code sent. `None` is "detect".
pub const LANGUAGES: [(&str, Option<&str>); 8] = [
    ("Auto", None),
    ("فارسی", Some("fa-IR")),
    ("English", Some("en-US")),
    ("العربية", Some("ar-SA")),
    ("Türkçe", Some("tr-TR")),
    ("Español", Some("es-ES")),
    ("Français", Some("fr-FR")),
    ("Deutsch", Some("de-DE")),
];

/// How a caption appears, by the sheet's row: the whole line, building up
/// word by word, one word at a time, typed letter by letter, or the whole
/// line at once with its colour passing from word to word.
pub const REVEALS: [Reveal; 5] = [
    Reveal::Whole,
    Reveal::Build,
    Reveal::WordByWord,
    Reveal::Typewriter { per_second: 14.0 },
    Reveal::Whole,
];

/// The row of [`REVEALS`] that lights the words one by one over the whole
/// line (the sheet's highlight layer) instead of cutting the caption up.
pub const HIGHLIGHT_REVEAL: usize = 4;

/// What the Gemini half of the sheet remembers while the window is open.
#[derive(Default)]
pub struct GeminiState {
    /// A key is saved in the credential store.
    pub key_saved: bool,
    /// A connection test is running.
    pub testing: bool,
    /// The last thing the key's save or test said, as words.
    pub status: String,
    /// The models the key can see, once tested; the default alone before.
    pub models: Vec<String>,
    /// Row in [`GeminiState::model_names`].
    pub model: usize,
    /// Row in [`LANGUAGES`].
    pub language: usize,
    /// Row in [`REVEALS`].
    pub reveal: usize,
    /// Set to stop the run in progress.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl GeminiState {
    /// The models the picker lists.
    pub fn model_names(&self) -> Vec<String> {
        if self.models.is_empty() {
            vec![DEFAULT_MODEL.to_owned()]
        } else {
            self.models.clone()
        }
    }

    /// The model chosen.
    pub fn chosen_model(&self) -> String {
        self.model_names()
            .get(self.model)
            .cloned()
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned())
    }

    /// The language's code, `None` for detect.
    pub fn language_code(&self) -> Option<&'static str> {
        LANGUAGES.get(self.language).and_then(|l| l.1)
    }

    /// Takes the models a test found, keeping the choice where it still
    /// stands, and otherwise the transcription model.
    pub fn set_models(&mut self, found: Vec<String>) {
        let before = self.chosen_model();
        self.models = models_for_picker(found);
        self.model = self
            .models
            .iter()
            .position(|m| *m == before)
            .or_else(|| self.models.iter().position(|m| m == DEFAULT_MODEL))
            .unwrap_or(0);
    }

    /// A row for `id`, as remembered in preferences.
    pub fn choose_model(&mut self, id: &str) {
        let names = self.model_names();
        if let Some(row) = names.iter().position(|m| m == id) {
            self.model = row;
        } else {
            // A remembered model the key has not been tested for yet.
            self.models = vec![id.to_owned()];
            self.model = 0;
        }
    }

    /// A row for the language code remembered in preferences.
    pub fn choose_language(&mut self, code: Option<&str>) {
        self.language = LANGUAGES.iter().position(|l| l.1 == code).unwrap_or(0);
    }
}

/// The models worth offering from what a key can see: Gemini's own, the
/// transcription model first, the rest by name.
pub fn models_for_picker(found: Vec<String>) -> Vec<String> {
    let mut list: Vec<String> = found
        .into_iter()
        .filter(|id| id.starts_with("gemini") && !id.contains("embedding") && !id.contains("live"))
        .collect();
    list.sort();
    list.dedup();
    if let Some(row) = list.iter().position(|m| m == DEFAULT_MODEL) {
        let model = list.remove(row);
        list.insert(0, model);
    }
    list
}

/// How the model is asked for timings: the transcription models time every
/// word; any other is asked for timed cues.
pub fn mode_for(model: &str) -> Mode {
    if model.contains("transcribe") {
        Mode::WordTimestamps
    } else {
        Mode::TimedCues
    }
}

/// Cues as the segments the sheet already knows how to place. With a
/// reveal other than the whole line, each cue is cut into the pieces
/// [`caption_fx::plan_reveal`] makes - from the words' own timings, never
/// from guessed ones - and each piece is a segment.
pub fn segments_of(cues: &[Cue], reveal: Reveal) -> Vec<Segment> {
    let mut out = Vec::with_capacity(cues.len());
    for cue in cues {
        let words: Vec<TimedWord> = cue
            .words
            .iter()
            .map(|word| (word.text.clone(), word.start, word.end))
            .collect();
        for piece in caption_fx::plan_reveal(cue.start, cue.end, &cue.text, &words, reveal) {
            out.push(Segment {
                start: piece.start,
                end: piece.start + piece.duration,
                text: piece.content,
            });
        }
    }
    out
}

/// What a run will send, as words for the sheet: how many requests and how
/// much sound, said before anything is sent.
pub fn notice(window: f64, model: &str) -> String {
    let options = JobOptions::new(model, None, mode_for(model));
    let estimate = job::estimate(window, &options);
    tf(
        "captions.geminiSendNotice",
        &[&estimate.requests, &format!("{:.1}", estimate.megabytes)],
    )
}

/// The clip to caption, as a worker needs it.
pub struct Source {
    /// The media file.
    pub path: String,
    /// Which of its audio streams.
    pub audio_stream: Option<u32>,
    /// Seconds into the file where the clip's window begins.
    pub source_start: f64,
    /// Seconds of source the clip covers.
    pub window: f64,
}

/// Runs a whole transcription. Blocks: run it on a worker.
pub fn transcribe(
    key: &str,
    model: &str,
    language: Option<&str>,
    source: &Source,
    reveal: Reveal,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(Stage),
) -> Result<Vec<Segment>, GeminiError> {
    let client = Client::new(ApiKey::new(key)).with_cancel(cancel);
    let options = JobOptions::new(model, language, mode_for(model));
    let mut read = |start: f64, len: f64| -> Result<Vec<f32>, String> {
        let mut decoder = AudioDecoder::open(
            &source.path,
            &AudioOptions {
                start: Some(source.source_start + start),
                duration: Some(len),
                filters: Vec::new(),
                rate: job::SAMPLE_RATE,
                channels: 1,
                format: SampleFormat::F32,
                stream: source.audio_stream.map(|index| index as usize),
            },
        )
        .map_err(|error| error.to_string())?;
        decoder.collect_f32().map_err(|error| error.to_string())
    };
    let output = job::run(&client, &options, source.window, &mut read, &mut progress)?;
    Ok(segments_of(&output.cues, reveal))
}

/// An error as the sheet says it, in the interface's language. The words
/// carry no key: see `concat_gemini::error`.
pub fn say(error: &GeminiError) -> String {
    match error {
        GeminiError::NoKey => t("captions.geminiNoKey"),
        GeminiError::InvalidKey => t("captions.geminiInvalidKey"),
        GeminiError::Quota { .. } => t("captions.geminiQuota"),
        GeminiError::ModelUnavailable(model) => tf("captions.geminiModelUnavailable", &[model]),
        GeminiError::Offline => t("captions.geminiOffline"),
        GeminiError::TooLarge => t("captions.geminiTooLarge"),
        GeminiError::Cancelled => "cancelled".to_owned(),
        other => tf("captions.geminiFailed", &[&other.to_string()]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_gemini::Word;

    #[test]
    fn the_picker_puts_the_transcription_model_first_and_drops_the_rest() {
        let found = vec![
            "gemini-flash".to_owned(),
            "text-embedding".to_owned(),
            "gemini-embedding-1".to_owned(),
            "gemini-3.5-transcribe".to_owned(),
            "gemini-3.5-transcribe-live".to_owned(),
            "gemini-flash".to_owned(),
        ];
        assert_eq!(
            models_for_picker(found),
            ["gemini-3.5-transcribe", "gemini-flash"]
        );
    }

    #[test]
    fn modes_follow_the_model() {
        assert_eq!(mode_for("gemini-3.5-transcribe"), Mode::WordTimestamps);
        assert_eq!(mode_for("gemini-flash"), Mode::TimedCues);
    }

    #[test]
    fn a_test_keeps_the_choice_when_it_can() {
        let mut state = GeminiState::default();
        state.choose_model("gemini-b");
        state.set_models(vec!["gemini-a".into(), "gemini-b".into(), "other".into()]);
        assert_eq!(state.chosen_model(), "gemini-b");
        state.set_models(vec!["gemini-3.5-transcribe".into(), "gemini-a".into()]);
        assert_eq!(state.chosen_model(), "gemini-3.5-transcribe");
    }

    #[test]
    fn languages_round_trip_through_their_codes() {
        let mut state = GeminiState::default();
        assert_eq!(state.language_code(), None);
        state.choose_language(Some("fa-IR"));
        assert_eq!(state.language_code(), Some("fa-IR"));
        state.choose_language(Some("xx-XX"));
        assert_eq!(state.language_code(), None, "an unknown code is detect");
    }

    #[test]
    fn cues_become_segments_with_their_text_and_times() {
        let cue = Cue {
            start: 1.5,
            end: 3.0,
            text: "سلام\nدنیا".into(),
            words: vec![Word {
                text: "سلام".into(),
                start: 1.5,
                end: 2.0,
            }],
        };
        let segments = segments_of(&[cue], Reveal::Whole);
        assert_eq!(segments.len(), 1);
        assert_eq!(
            (
                segments[0].start,
                segments[0].end,
                segments[0].text.as_str()
            ),
            (1.5, 3.0, "سلام\nدنیا")
        );
    }
}
