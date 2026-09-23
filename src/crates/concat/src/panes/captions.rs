// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The captions sheet: the tray's Captions tool, as a form and then as a
//! progress report.
//!
//! What it captions is decided when it opens, not asked: the selected
//! clip's sound when one clip with sound is selected, and a script
//! otherwise. Either lands as one batch of title clips, one undo step.
//! The transcriber runs on a worker and reports as
//! [`CaptionsMsg::Progress`], ending as [`CaptionsMsg::Finished`].

use std::sync::Arc;

use concat_project::Command;
use concat_project::model::TextStyle;
use concat_speech::transcribe::Segment;
use slint::SharedString;

use crate::host::{on_ui_in_project, spawn_in_project};
use crate::i18n::{t, tf};
use crate::panes::Msg;
use crate::panes::settings::installed;
use crate::studio::Studio;
use crate::ui::CaptionsSheetData;

/// Where a caption sits, by the sheet's row: a frame-height fraction from
/// the centre, positive down. Bottom, centre, top.
const CAPTION_OFFSETS: [f64; 3] = [0.35, 0.0, -0.35];
/// A caption's cap height by the sheet's row, as a fraction of the frame.
const CAPTION_SIZES: [f64; 3] = [0.04, 0.05, 0.065];
/// A rough speaking rate, for a script's timing and the speech sheet's
/// estimate.
pub const CHARS_PER_SECOND: f32 = 14.0;

/// Everything that can happen to the captions sheet.
#[derive(Clone, Debug)]
pub enum CaptionsMsg {
    /// The tray's Captions tool.
    Open,
    Close,
    TextEdited(String),
    ModelChanged(i32),
    PlacementChanged(i32),
    SizeChanged(i32),
    /// Run the pass the sheet describes.
    Begin,
    Cancel,
    /// The transcriber's worker reporting where it is, in percent.
    Progress(i32),
    /// The transcriber's worker is done: what was said, or why not.
    Finished(Result<Vec<Segment>, String>),
}

/// The clip a transcription is running over, as the finished segments
/// need it: where it starts on the timeline and how fast it plays.
#[derive(Clone, Copy, Debug)]
struct Subject {
    start: f64,
    speed: f64,
    /// The look chosen when the run began.
    look: (f64, f64),
}

/// The captions sheet's state.
#[derive(Default)]
pub struct CaptionsPane {
    pub open: bool,
    /// The clip being transcribed: the one selected when the sheet opened,
    /// when it had sound. None, and the sheet is a script instead.
    pub clip: Option<String>,
    /// The script's words.
    pub text: String,
    /// Row in the installed transcriber list.
    pub model: usize,
    /// 0 bottom, 1 centre, 2 top.
    pub placement: usize,
    /// 0 small, 1 medium, 2 large.
    pub size: usize,
    pub running: bool,
    pub progress: f32,
    /// Why the last run failed, when it did.
    pub message: String,
    /// The clip under the running transcription.
    subject: Option<Subject>,
}

impl CaptionsPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: CaptionsMsg, studio: &mut Studio) {
        match msg {
            CaptionsMsg::Open => {
                let installed = installed(&studio.settings.transcribers);
                let model = installed.iter().position(|model| model.active).unwrap_or(0);
                let clip = studio
                    .sole_selection()
                    .and_then(|id| studio.clip(&id))
                    .filter(|clip| studio.clip_has_sound(clip))
                    .map(|clip| clip.id.clone());
                *self = CaptionsPane {
                    open: true,
                    clip,
                    model,
                    placement: 0,
                    size: 1,
                    ..CaptionsPane::default()
                };
            }
            CaptionsMsg::Close => self.open = false,
            CaptionsMsg::TextEdited(text) => self.text = text,
            CaptionsMsg::ModelChanged(index) => self.model = index.max(0) as usize,
            CaptionsMsg::PlacementChanged(index) => {
                self.placement = (index.max(0) as usize).min(2);
            }
            CaptionsMsg::SizeChanged(index) => self.size = (index.max(0) as usize).min(2),
            CaptionsMsg::Begin => {
                if self.clip.is_some() {
                    self.run_sound(studio);
                } else {
                    self.run_script(studio);
                }
            }
            CaptionsMsg::Cancel => {
                studio.host.transcriber.cancel();
                self.running = false;
                self.open = false;
            }
            CaptionsMsg::Progress(percent) => {
                self.progress = (percent as f32 / 100.0).clamp(0.0, 1.0);
            }
            CaptionsMsg::Finished(result) => {
                self.running = false;
                let subject = self.subject.take();
                match result {
                    Ok(segments) => {
                        let Some(subject) = subject else {
                            return;
                        };
                        let look = subject.look;
                        let commands: Vec<Command> = segments
                            .into_iter()
                            .filter_map(|segment| {
                                let text = segment.text.trim().to_owned();
                                (!text.is_empty()).then(|| {
                                    caption_clip(
                                        text,
                                        subject.start + segment.start / subject.speed,
                                        ((segment.end - segment.start) / subject.speed).max(0.2),
                                        look,
                                    )
                                })
                            })
                            .collect();
                        let count = commands.len();
                        self.open = false;
                        if count == 0 {
                            studio.notify(&t("Nothing was said in that clip"), true);
                        } else {
                            studio.apply(Command::Batch { commands });
                            studio.notify(&tf("Added {0} captions", &[&count]), false);
                        }
                    }
                    // Asked for: the sheet is already on its way down.
                    Err(error) if error.contains("cancel") => self.open = false,
                    Err(error) => self.message = error,
                }
            }
        }
    }

    /// A caption's look, by the sheet's rows: where it sits and its size.
    fn look(&self) -> (f64, f64) {
        (
            CAPTION_OFFSETS[self.placement.min(2)],
            CAPTION_SIZES[self.size.min(2)],
        )
    }

    /// The script as titles, one after another from the playhead.
    fn run_script(&mut self, studio: &mut Studio) {
        let lines = script_captions(&self.text);
        if lines.is_empty() {
            self.message = t("Nothing to caption yet");
            return;
        }
        let look = self.look();
        let mut at = f64::from(studio.playhead);
        let commands: Vec<Command> = lines
            .into_iter()
            .map(|(text, seconds)| {
                let command = caption_clip(text, at, seconds, look);
                at += seconds;
                command
            })
            .collect();
        let count = commands.len();
        self.open = false;
        studio.apply(Command::Batch { commands });
        studio.notify(&tf("Added {0} captions", &[&count]), false);
    }

    /// The sheet's clip through the transcriber on a worker, reporting into
    /// the sheet as it goes.
    fn run_sound(&mut self, studio: &mut Studio) {
        let Some(clip) = self.clip.as_ref().and_then(|id| studio.clip(id)).cloned() else {
            self.message = t("The clip is no longer on the timeline");
            return;
        };
        let Some(media) = studio.project().media_by_id(&clip.media_id).cloned() else {
            self.message = t("This clip has no file to transcribe");
            return;
        };
        let Some(model) = installed(&studio.settings.transcribers)
            .get(self.model)
            .map(|model| model.id.clone())
        else {
            self.message = t("Download a transcriber model in Settings › Transcriber first");
            return;
        };
        let request = concat_speech::transcribe::TranscribeRequest {
            path: media.path.clone(),
            audio_stream: clip.audio_stream,
            source_start: clip.source_start,
            window: clip.duration * clip.speed,
            model_id: model,
        };
        let dirs = studio.host.dirs.clone();
        let transcriber = Arc::clone(&studio.host.transcriber);
        self.subject = Some(Subject {
            start: clip.start,
            speed: clip.speed,
            look: self.look(),
        });
        self.running = true;
        self.progress = 0.0;
        self.message.clear();
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                transcriber.transcribe(&dirs, &request, move |percent| {
                    on_ui_in_project(epoch, move |studio, _, _| {
                        studio.handle(Msg::Captions(CaptionsMsg::Progress(percent)));
                    });
                })
            },
            |studio, _, _, result| studio.handle(Msg::Captions(CaptionsMsg::Finished(result))),
        );
    }

    /// The sheet as Slint shows it.
    pub fn data(&self, studio: &Studio) -> CaptionsSheetData {
        CaptionsSheetData {
            open: self.open,
            from_sound: self.clip.is_some(),
            subject: self
                .clip
                .as_ref()
                .and_then(|id| studio.clip(id))
                .map(|clip| SharedString::from(clip.name.as_str()))
                .unwrap_or_else(|| t("at the playhead").into()),
            text: self.text.as_str().into(),
            model: self.model as i32,
            placement: self.placement as i32,
            size: self.size as i32,
            running: self.running,
            progress: self.progress,
            ready: !installed(&studio.settings.transcribers).is_empty(),
            message: self.message.as_str().into(),
        }
    }
}

/// One caption as a title clip: its words, when and for how long, and
/// its look.
fn caption_clip(text: String, start: f64, duration: f64, look: (f64, f64)) -> Command {
    let (offset_y, font_size) = look;
    Command::AddTextClip {
        track_id: None,
        above: true,
        start,
        style: Some(TextStyle {
            content: text,
            font_family: "Hanken Grotesk".to_owned(),
            font_size,
            font_weight: 600.0,
            ..TextStyle::default()
        }),
        duration: Some(duration),
        offset_y: Some(offset_y),
    }
}

/// Longest a caption line gets before it is wrapped: about what two lines
/// of broadcast subtitle hold, and what a reader takes in at a glance.
const CAPTION_CHARS: usize = 42;

/// A script as caption lines, each with how long it stays up: a line's
/// reading time at [`CHARS_PER_SECOND`], held to one second at least so
/// a short word is not a flicker, and seven at most so a long line does
/// not hang. A line break in the script is a break the author asked for;
/// within a paragraph a sentence is a caption, and a long sentence wraps
/// at its words.
fn script_captions(text: &str) -> Vec<(String, f64)> {
    text.lines()
        .flat_map(sentences)
        .flat_map(|sentence| wrap_caption(&sentence))
        .map(|line| {
            let seconds =
                (line.chars().count() as f64 / f64::from(CHARS_PER_SECOND)).clamp(1.0, 7.0);
            (line, seconds)
        })
        .collect()
}

/// A paragraph's sentences. A full stop, question or exclamation mark ends
/// one when it is followed by space or by the end - so "3.5" and "e.g." hold
/// together - and the CJK marks end one on their own.
fn sentences(paragraph: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = paragraph.chars().peekable();
    while let Some(ch) = chars.next() {
        current.push(ch);
        let ends = match ch {
            '。' | '！' | '？' => true,
            '.' | '!' | '?' => chars.peek().is_none_or(|next| next.is_whitespace()),
            _ => false,
        };
        if ends {
            let sentence = current.trim();
            if !sentence.is_empty() {
                out.push(sentence.to_owned());
            }
            current.clear();
        }
    }
    let rest = current.trim();
    if !rest.is_empty() {
        out.push(rest.to_owned());
    }
    out
}

/// A sentence in lines of at most [`CAPTION_CHARS`], broken between words;
/// a word longer than a line, or a run of CJK with no spaces, is broken
/// where it must be.
fn wrap_caption(sentence: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_chars = 0;
    for word in sentence.split_whitespace() {
        let word_chars = word.chars().count();
        if line_chars > 0 && line_chars + 1 + word_chars > CAPTION_CHARS {
            lines.push(std::mem::take(&mut line));
            line_chars = 0;
        }
        if word_chars > CAPTION_CHARS {
            let mut piece = String::new();
            for ch in word.chars() {
                piece.push(ch);
                if piece.chars().count() == CAPTION_CHARS {
                    lines.push(std::mem::take(&mut piece));
                }
            }
            line = piece;
            line_chars = line.chars().count();
            continue;
        }
        if line_chars > 0 {
            line.push(' ');
            line_chars += 1;
        }
        line.push_str(word);
        line_chars += word_chars;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::script_captions;

    /// A script becomes one caption per sentence, a hand line break is
    /// kept, a long sentence wraps at its words, and each line is held for
    /// its reading time within one to seven seconds.
    #[test]
    fn a_script_is_cut_into_readable_lines() {
        let lines = script_captions(
            "Hello there. This is version 3.5, mind!\n\nA sentence that runs on for far \
             longer than a caption line has any business running on for. Ok?",
        );
        let text: Vec<&str> = lines.iter().map(|(line, _)| line.as_str()).collect();
        assert_eq!(
            text,
            [
                "Hello there.",
                "This is version 3.5, mind!",
                "A sentence that runs on for far longer",
                "than a caption line has any business",
                "running on for.",
                "Ok?",
            ]
        );
        assert!(
            lines
                .iter()
                .all(|(_, seconds)| (1.0..=7.0).contains(seconds))
        );
        assert_eq!(lines[0].1, 1.0);
        assert!(lines[2].1 > lines[0].1);
        assert!(script_captions("  \n ").is_empty());
        assert_eq!(script_captions("你好。再见！").len(), 2);
    }
}
