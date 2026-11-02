// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The captions sheet: the tray's Captions tool, as a form and then as a
//! progress report.
//!
//! One switch says what it captions: "Use Sample Voice", on, takes
//! the words from a clip with sound, chosen in the sheet from the clips on
//! the timeline that have any; off, from a script typed in it. It opens on
//! when a clip with sound is selected, or when a clip's menu opened it on
//! that clip, and off otherwise. Either lands as one batch of title clips,
//! one undo step.
//! The transcriber runs on a worker and reports as
//! [`CaptionsMsg::Progress`], ending as [`CaptionsMsg::Finished`].
//!
//! The sheet also reads a SubRip file in as captions, with the same look,
//! and writes the timeline's captions out as one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use concat_gemini::job::{self, JobOptions, Stage as GeminiStage};
use concat_gemini::{ApiKey, Client as GeminiClient, GeminiError};
use concat_project::Command;
use concat_project::caption_fx::{self, Direction, Effect};
use concat_project::model::{ClipKind, TextStyle};
use concat_project::srt;
use concat_speech::JobStage;
use concat_speech::transcribe::Segment;

use crate::host::{on_ui_in_project, spawn_in_project};
use crate::i18n::{t, tf};
use crate::panes::Msg;
use crate::panes::gemini_captions::{self, GeminiState};
use crate::panes::pace::{Pace, Stage, Timings};
use crate::panes::settings::installed;
use crate::secret::{KeyStore, OsKeyStore};
use crate::studio::Studio;
use crate::ui::CaptionsSheetData;

/// Where a caption sits, by the sheet's row: a frame-height fraction from
/// the centre, positive down. Bottom, centre, top.
const CAPTION_OFFSETS: [f64; 3] = [0.35, 0.0, -0.35];
/// A caption's cap height by the sheet's row, as a fraction of the frame.
const CAPTION_SIZES: [f64; 3] = [0.04, 0.05, 0.065];
/// How many motions the sheet lists: none, fade, pop, slide, scale.
const MOTIONS: usize = 5;
/// The colours the highlight layer can light a word in; the sheet's list
/// is "none" and then these.
const HIGHLIGHTS: [&str; 5] = ["#ffe600", "#3dff6e", "#5ef2ff", "#ff4fd8", "#ff9f1c"];

/// A rough speaking rate, for a script's timing and the speech sheet's
/// estimate.
pub const CHARS_PER_SECOND: f32 = 14.0;

/// Everything that can happen to the captions sheet.
#[derive(Clone, Debug)]
pub enum CaptionsMsg {
    /// The tray's Captions tool: the selected clip's voice when it has
    /// sound, the script otherwise.
    Open,
    /// A clip's menu: the sheet on that clip's voice.
    OpenOnClip(String),
    Close,
    /// The "Use Sample Voice" switch.
    VoiceToggled(bool),
    /// Row in the list of clips with sound.
    SourceChanged(i32),
    TextEdited(String),
    ModelChanged(i32),
    /// 0 the local transcriber, 1 Gemini.
    EngineChanged(i32),
    /// A key typed in the sheet, to be kept in the credential store.
    GeminiKeySaved(String),
    /// Forget the saved key.
    GeminiKeyCleared,
    /// Try the key and list the models it can see.
    GeminiTest,
    /// The connection test finished: the model ids, or why not.
    GeminiTested(Result<Vec<String>, GeminiError>),
    GeminiModelChanged(i32),
    GeminiLanguageChanged(i32),
    /// How the captions appear: the whole line, building, word by word, typed.
    GeminiRevealChanged(i32),
    /// The motion row of the sheet.
    MotionChanged(i32),
    /// Give the captions lane that motion.
    ApplyMotion,
    /// The style row of the sheet.
    StyleChanged(i32),
    /// Dress the captions lane in that style.
    ApplyStyle,
    /// The highlight row of the sheet.
    HighlightChanged(i32),
    /// Light the captions' words one at a time, in the sheet's colour.
    ApplyHighlight,
    PlacementChanged(i32),
    SizeChanged(i32),
    /// Run the pass the sheet describes.
    Begin,
    Cancel,
    /// The transcriber's worker reporting the stage it is in.
    Progress(JobStage),
    /// The transcriber's worker is done: what was said, or why not.
    Finished(Result<Vec<Segment>, String>),
    /// Asks for a SubRip file to read in.
    ImportSrt,
    /// The SubRip file chosen: its cues as captions.
    ImportSrtFrom(PathBuf),
    /// Asks where to write the captions as SubRip.
    ExportSrt,
    /// Where they go.
    ExportSrtTo(PathBuf),
    /// Asks where to write the captions as ASS, with their look.
    ExportAss,
    /// Where they go.
    ExportAssTo(PathBuf),
}

/// The clip a transcription is running over, as the finished segments
/// need it: where it starts on the timeline and how fast it plays.
#[derive(Clone, Debug)]
struct Subject {
    start: f64,
    speed: f64,
    /// The look chosen when the run began.
    look: (f64, f64),
    /// The transcriber model, and the thirty-second windows whisper reads
    /// the sound in: what the run's timing is kept by.
    model: String,
    windows: f32,
}

/// Seconds whisper takes over one thirty-second window, per megabyte of
/// model, before a run on the model has been timed: about a second for
/// base and ten for medium on eight cores of a recent CPU.
const WINDOW_SECONDS_PER_MB: f32 = 1.0 / 160.0;

/// The captions sheet's state.
#[derive(Default)]
pub struct CaptionsPane {
    pub open: bool,
    /// "Use Sample Voice": the words come from [`Self::clip`]'s sound
    /// when on, from [`Self::text`] when off.
    pub from_voice: bool,
    /// The clip whose voice is transcribed: one with sound, chosen from the
    /// timeline's. Kept while the switch is off, so turning it back on
    /// returns to the same clip. The panel floats over a live editor, so
    /// a clip with sound selected since is the one shown and run instead;
    /// see [`Self::subject_clip`].
    pub clip: Option<String>,
    /// The sole selection when [`Self::clip`] was chosen - opened on, or
    /// picked from the list - so a later selection can be told from it.
    chosen_with: Option<String>,
    /// The script's words.
    pub text: String,
    /// Row in the installed transcriber list.
    pub model: usize,
    /// 0 the local transcriber, 1 Gemini.
    pub engine: usize,
    /// The Gemini engine's state.
    pub gemini: GeminiState,
    /// 0 bottom, 1 centre, 2 top.
    pub placement: usize,
    /// 0 small, 1 medium, 2 large.
    pub size: usize,
    /// Row of the motion list: none, fade, pop, slide, scale.
    pub motion: usize,
    /// Row of the style list: the kept look, then the Text page's presets.
    pub style: usize,
    /// Row of the highlight list: none, then `HIGHLIGHTS`.
    pub highlight: usize,
    pub running: bool,
    /// How far the running transcription looks.
    pub pace: Pace,
    /// Why the last run failed, when it did.
    pub message: String,
    /// The clip under the running transcription.
    subject: Option<Subject>,
    /// How long each model took this session, for the next run's bar.
    timings: Timings,
}

impl CaptionsPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: CaptionsMsg, studio: &mut Studio) {
        match msg {
            // A run in progress is shown as it is, however the panel is
            // asked for again: closing it never stopped the run.
            CaptionsMsg::Open | CaptionsMsg::OpenOnClip(_) if self.running => self.open = true,
            CaptionsMsg::Open => {
                let selected = studio.sole_selection();
                self.open_on(studio, selected, false);
            }
            CaptionsMsg::OpenOnClip(id) => self.open_on(studio, Some(id), true),
            CaptionsMsg::Close => self.open = false,
            CaptionsMsg::VoiceToggled(on) => {
                self.from_voice = on;
                self.message.clear();
                let sources = sources(studio);
                if on
                    && !self
                        .clip
                        .as_ref()
                        .is_some_and(|id| sources.iter().any(|s| &s.id == id))
                {
                    self.clip = sources.first().map(|source| source.id.clone());
                    self.chosen_with = studio.sole_selection();
                }
            }
            CaptionsMsg::SourceChanged(index) => {
                if let Some(source) = usize::try_from(index)
                    .ok()
                    .and_then(|index| sources(studio).into_iter().nth(index))
                {
                    self.clip = Some(source.id);
                    self.chosen_with = studio.sole_selection();
                    self.message.clear();
                }
            }
            CaptionsMsg::TextEdited(text) => self.text = text,
            CaptionsMsg::ModelChanged(index) => self.model = index.max(0) as usize,
            CaptionsMsg::EngineChanged(index) => {
                self.engine = (index.max(0) as usize).min(1);
                self.message.clear();
                studio.prefs.captions_engine = (self.engine == 1).then(|| "gemini".to_owned());
                studio.prefs.save(&studio.host.dirs);
                self.refresh_key();
            }
            CaptionsMsg::GeminiKeySaved(text) => {
                let key = text.trim();
                if key.is_empty() {
                    self.gemini.status = t("captions.geminiKeyEmpty");
                } else {
                    match OsKeyStore.save(key) {
                        Ok(()) => {
                            self.gemini.key_saved = true;
                            self.gemini.status = t("captions.geminiKeySaved");
                            self.message.clear();
                        }
                        Err(error) => {
                            self.message = tf("captions.geminiKeyNotSaved", &[&error]);
                        }
                    }
                }
            }
            CaptionsMsg::GeminiKeyCleared => match OsKeyStore.clear() {
                Ok(()) => {
                    self.gemini.key_saved = false;
                    self.gemini.status = t("captions.geminiKeyRemoved");
                }
                Err(error) => self.message = tf("captions.geminiKeyNotSaved", &[&error]),
            },
            CaptionsMsg::GeminiTest => self.test_gemini(),
            CaptionsMsg::GeminiTested(result) => {
                self.gemini.testing = false;
                match result {
                    Ok(ids) => {
                        let wanted = self.gemini.chosen_model();
                        let seen = ids.contains(&wanted);
                        self.gemini.set_models(ids);
                        self.message = if seen {
                            String::new()
                        } else {
                            gemini_captions::say(&GeminiError::ModelUnavailable(wanted))
                        };
                        self.gemini.status = tf(
                            "captions.geminiConnected",
                            &[&self.gemini.model_names().len()],
                        );
                    }
                    Err(error) => {
                        self.gemini.status.clear();
                        self.message = gemini_captions::say(&error);
                    }
                }
            }
            CaptionsMsg::GeminiModelChanged(index) => {
                self.gemini.model = index.max(0) as usize;
                studio.prefs.gemini_model = Some(self.gemini.chosen_model());
                studio.prefs.save(&studio.host.dirs);
            }
            CaptionsMsg::GeminiLanguageChanged(index) => {
                self.gemini.language = index.max(0) as usize;
                studio.prefs.gemini_language = self.gemini.language_code().map(str::to_owned);
                studio.prefs.save(&studio.host.dirs);
            }
            CaptionsMsg::GeminiRevealChanged(index) => {
                self.gemini.reveal =
                    (index.max(0) as usize).min(gemini_captions::REVEALS.len() - 1);
            }
            CaptionsMsg::MotionChanged(index) => {
                self.motion = (index.max(0) as usize).min(MOTIONS - 1);
            }
            CaptionsMsg::ApplyMotion => self.apply_motion(studio),
            CaptionsMsg::StyleChanged(index) => {
                self.style = (index.max(0) as usize).min(style_ids(studio).len());
            }
            CaptionsMsg::ApplyStyle => self.apply_style(studio),
            CaptionsMsg::HighlightChanged(index) => {
                self.highlight = (index.max(0) as usize).min(HIGHLIGHTS.len());
            }
            CaptionsMsg::ApplyHighlight => self.apply_highlight(studio),
            CaptionsMsg::PlacementChanged(index) => {
                self.placement = (index.max(0) as usize).min(2);
            }
            CaptionsMsg::SizeChanged(index) => self.size = (index.max(0) as usize).min(2),
            CaptionsMsg::Begin => {
                if self.from_voice && self.engine == 1 {
                    self.run_gemini(studio);
                } else if self.from_voice {
                    self.run_sound(studio);
                } else {
                    self.run_script(studio);
                }
            }
            CaptionsMsg::Cancel => {
                studio.host.transcriber.cancel();
                if let Some(flag) = &self.gemini.cancel {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.running = false;
                self.open = false;
            }
            CaptionsMsg::Progress(stage) => self.pace.report(stage),
            CaptionsMsg::Finished(result) => {
                self.running = false;
                let subject = self.subject.take();
                if let (Ok(_), Some(subject)) = (&result, &subject) {
                    self.timings
                        .keep(&subject.model, self.pace.learned(subject.windows));
                }
                match result {
                    Ok(segments) => {
                        let Some(subject) = subject else {
                            return;
                        };
                        let look = subject.look;
                        let base = self.base_style(studio);
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
                                        base.as_ref(),
                                    )
                                })
                            })
                            .collect();
                        let count = commands.len();
                        self.open = false;
                        if count == 0 {
                            studio.notify(&t("captions.nothingSaidClip"), true);
                        } else {
                            studio.apply(Command::Batch { commands });
                            studio.notify(&tf("captions.addedCaptions", &[&count]), false);
                        }
                    }
                    // Asked for: the sheet is already on its way down.
                    Err(error) if error.contains("cancel") => self.open = false,
                    // Closed while it ran: the panel is not there to say
                    // it, so the notice does, and the panel too next time.
                    Err(error) => {
                        if !self.open {
                            studio.notify(&error, true);
                        }
                        self.message = error;
                    }
                }
            }
            // Asked with the studio unborrowed, on a turn of its own; see
            // `host::after_dialog`.
            CaptionsMsg::ImportSrt => crate::host::pick_files_then(
                t("captions.importSrt"),
                Some((t("captions.subRipSubtitles"), &["srt"])),
                |studio, _, _, paths| {
                    if let Some(path) = paths.into_iter().next() {
                        studio.handle(Msg::Captions(CaptionsMsg::ImportSrtFrom(path)));
                    }
                },
            ),
            CaptionsMsg::ImportSrtFrom(path) => self.import_srt(studio, &path),
            CaptionsMsg::ExportSrt => self.ask_export_srt(studio),
            CaptionsMsg::ExportSrtTo(path) => self.export_srt(studio, &path),
            CaptionsMsg::ExportAss => self.ask_export_ass(studio),
            CaptionsMsg::ExportAssTo(path) => self.export_ass(studio, &path),
        }
    }

    /// A SubRip file's cues as captions where the file times them, with
    /// the sheet's look: one batch, one undo step. A file that is not
    /// UTF-8 is read as best it can be.
    fn import_srt(&mut self, studio: &mut Studio, path: &Path) {
        let cues = std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                srt::parse(&String::from_utf8_lossy(&bytes)).map_err(|error| error.to_string())
            });
        let cues = match cues {
            Ok(cues) => cues,
            Err(error) => {
                log::warn!("captions: could not read {}: {error}", path.display());
                self.message = tf("captions.couldNotReadSubtitles", &[&error]);
                return;
            }
        };
        let look = self.look();
        let base = self.base_style(studio);
        let commands = srt_commands(cues, look, base.as_ref());
        if commands.is_empty() {
            self.message = t("captions.noSubtitlesInFile");
            return;
        }
        let count = commands.len();
        self.open = false;
        studio.apply(Command::Batch { commands });
        studio.notify(&tf("captions.addedCaptions", &[&count]), false);
    }

    /// Asks where to write the captions, offering the project's folder
    /// and its name. A phone has no such dialog: the file is written
    /// beside the project under a name nothing there has, and handed to
    /// the phone to put where its file manager shows it.
    fn ask_export_srt(&mut self, studio: &mut Studio) {
        if export_lane(studio).is_none() {
            self.message = t("captions.noCaptionsToExport");
            return;
        }
        let name = format!("{}.srt", file_stem(studio));
        // No project, no project folder: Documents, as anywhere else a
        // file is written with nothing better to offer.
        let folder = studio
            .session
            .as_ref()
            .map(|session| PathBuf::from(session.path()))
            .unwrap_or_else(|| PathBuf::from(crate::studio::home_folder("Documents")));
        if cfg!(any(target_os = "android", target_os = "ios")) {
            let path = unique_path(&folder, &name);
            self.export_srt(studio, &path);
            return;
        }
        let title = t("captions.exportSrt");
        let family = t("captions.subRipSubtitles");
        crate::host::after_dialog(
            move || {
                let chosen = crate::platform::save_file(
                    &title,
                    &folder,
                    &name,
                    (family.as_str(), &["srt"]),
                )?;
                Some(
                    if chosen
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("srt"))
                    {
                        chosen
                    } else {
                        chosen.with_extension("srt")
                    },
                )
            },
            |studio, _, _, path| studio.handle(Msg::Captions(CaptionsMsg::ExportSrtTo(path))),
        );
    }

    /// Writes the captions to `path` as SubRip, and says how many and
    /// where - on a phone, once the phone has put the file where it shows
    /// it.
    fn export_srt(&mut self, studio: &mut Studio, path: &Path) {
        let Some(lane) = export_lane(studio) else {
            self.message = t("captions.noCaptionsToExport");
            return;
        };
        let cues = srt::title_cues(studio.timeline(), Some(&lane.track_id));
        if let Err(error) = std::fs::write(path, srt::write(&cues)) {
            log::warn!("captions: could not write {}: {error}", path.display());
            self.message = tf("captions.couldNotWriteSubtitles", &[&error]);
            return;
        }
        log::info!(
            "captions: wrote {} cue(s) to {}",
            cues.len(),
            path.display()
        );
        self.open = false;
        let count = cues.len();
        // Which lane was written goes without saying when it was the
        // selected title's, or the only lane with titles; otherwise the
        // notice says it was the busiest, and how to pick another.
        let busiest = lane.busiest && srt::title_lanes(studio.timeline()) > 1;
        let say = move |studio: &mut Studio, place: &str| {
            // Each key spelled out where it is used: scripts/locales.py
            // finds the strings the source asks for by reading it.
            let said = if busiest {
                tf("captions.exportedBusiestLane", &[&count, &place])
            } else {
                tf("captions.exportedCaptions", &[&count, &place])
            };
            studio.notify(&said, false);
        };
        let written = path.to_path_buf();
        let handed = crate::platform::publish_document(
            path.to_path_buf(),
            "application/x-subrip",
            move |result| {
                crate::host::on_ui(move |studio, _, _| match result {
                    Ok(place) => say(studio, &place),
                    Err(error) => {
                        log::warn!("captions: could not publish {}: {error}", written.display());
                        studio.notify(
                            &tf(
                                "captions.couldNotPublishSubtitles",
                                &[&written.display(), &error],
                            ),
                            true,
                        );
                    }
                });
            },
        );
        if !handed {
            say(studio, &path.display().to_string());
        }
    }

    /// Asks where to write the captions as ASS: the same lane the SubRip
    /// export would write, with each title's look.
    fn ask_export_ass(&mut self, studio: &mut Studio) {
        if export_lane(studio).is_none() {
            self.message = t("captions.noCaptionsToExport");
            return;
        }
        let name = format!("{}.ass", file_stem(studio));
        let folder = studio
            .session
            .as_ref()
            .map(|session| PathBuf::from(session.path()))
            .unwrap_or_else(|| PathBuf::from(crate::studio::home_folder("Documents")));
        if cfg!(any(target_os = "android", target_os = "ios")) {
            let path = unique_path(&folder, &name);
            self.export_ass(studio, &path);
            return;
        }
        let title = t("captions.exportAss");
        let family = t("captions.assSubtitles");
        crate::host::after_dialog(
            move || {
                let chosen = crate::platform::save_file(
                    &title,
                    &folder,
                    &name,
                    (family.as_str(), &["ass"]),
                )?;
                Some(
                    if chosen
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("ass"))
                    {
                        chosen
                    } else {
                        chosen.with_extension("ass")
                    },
                )
            },
            |studio, _, _, path| studio.handle(Msg::Captions(CaptionsMsg::ExportAssTo(path))),
        );
    }

    /// Writes the captions to `path` as ASS, and says what the format
    /// could not carry rather than leaving it out silently.
    fn export_ass(&mut self, studio: &mut Studio, path: &Path) {
        let Some(lane) = export_lane(studio) else {
            self.message = t("captions.noCaptionsToExport");
            return;
        };
        let (width, height) = studio.output_size();
        let out =
            concat_project::ass::write(studio.timeline(), Some(&lane.track_id), width, height);
        if let Err(error) = std::fs::write(path, out.text.as_bytes()) {
            log::warn!("captions: could not write {}: {error}", path.display());
            self.message = tf("captions.couldNotWriteSubtitles", &[&error]);
            return;
        }
        log::info!(
            "captions: wrote {} event(s) to {}",
            out.events,
            path.display()
        );
        self.open = false;
        let said = tf(
            "captions.exportedCaptions",
            &[&out.events, &path.display().to_string()],
        );
        studio.notify(&said, false);
        if !out.warnings.is_empty() {
            studio.notify(
                &tf("captions.assNotCarried", &[&out.warnings.join(" ")]),
                true,
            );
        }
    }

    /// Gives every caption on the lane the sheet's motion: the selected
    /// title's lane, or the busiest. Keys on the clips' own opacity, scale
    /// and offset, one undo step, editable afterwards on the timeline.
    fn apply_motion(&mut self, studio: &mut Studio) {
        let Some(lane) = export_lane(studio) else {
            self.message = t("captions.noCaptionsToExport");
            return;
        };
        let effect = match self.motion {
            1 => Effect::Fade {
                fade_in: 0.25,
                fade_out: 0.25,
            },
            2 => Effect::Pop {
                seconds: 0.35,
                overshoot: 0.1,
            },
            3 => Effect::Slide {
                seconds: 0.35,
                from: Direction::Down,
                distance: 0.06,
            },
            4 => Effect::Scale {
                seconds: 0.3,
                from: 0.7,
            },
            _ => Effect::None,
        };
        let mut commands: Vec<Command> = Vec::new();
        let mut count = 0;
        for clip in studio.timeline().clips.iter() {
            if clip.kind == ClipKind::Text && clip.track_id == lane.track_id {
                commands.extend(caption_fx::commands(clip, &effect));
                count += 1;
            }
        }
        if commands.is_empty() {
            return;
        }
        studio.apply(Command::Batch { commands });
        studio.notify(&tf("captions.motionApplied", &[&count]), false);
    }

    /// The look new captions are made in: the style the sheet names, or
    /// the look the person kept, or nothing (the bundled face).
    fn base_style(&self, studio: &mut Studio) -> Option<TextStyle> {
        let ids = style_ids(studio);
        if let Some(id) = self.style.checked_sub(1).and_then(|row| ids.get(row))
            && let Some(preset) = studio.text_presets.iter().find(|held| &held.id == id)
        {
            let style = preset.style.clone();
            studio.ensure_font_for(&style.font_family);
            return Some(style);
        }
        studio.prefs.title_style.clone()
    }

    /// The highlight layer: over each caption, the same words again in the
    /// sheet's colour, one word at a time, each clip masked to its word.
    /// Made of what the editor already has - a title, a rectangle mask,
    /// a lane of its own - so the preview and the export draw it alike.
    /// Any earlier layer is taken off first; "none" only does that.
    fn apply_highlight(&mut self, studio: &mut Studio) {
        let Some(lane) = export_lane(studio) else {
            self.message = t("captions.noCaptionsToExport");
            return;
        };
        let old: Vec<String> = studio
            .timeline()
            .clips
            .iter()
            .filter(|clip| clip.kind == ClipKind::Text && clip.name == caption_fx::HIGHLIGHT_NAME)
            .map(|clip| clip.id.clone())
            .collect();
        if !old.is_empty() {
            studio.apply(Command::RemoveClips {
                clip_ids: old,
                ripple: false,
            });
        }
        let Some(colour) = self
            .highlight
            .checked_sub(1)
            .and_then(|index| HIGHLIGHTS.get(index))
            .copied()
        else {
            studio.notify(&t("captions.highlightRemoved"), false);
            return;
        };
        let (width, height) = studio.output_size();
        let captions: Vec<concat_project::model::Clip> = studio
            .timeline()
            .clips
            .iter()
            .filter(|clip| {
                clip.kind == ClipKind::Text
                    && clip.track_id == lane.track_id
                    && clip.name != caption_fx::HIGHLIGHT_NAME
            })
            .map(|clip| (**clip).clone())
            .collect();
        let mut layer: Option<String> = None;
        let (mut words_lit, mut skipped) = (0usize, 0usize);
        for clip in captions {
            let Some(text) = clip.text.clone() else {
                continue;
            };
            let count = text.content.split_whitespace().count();
            let rtl = text
                .content
                .chars()
                .any(|c| ('\u{0590}'..='\u{08FF}').contains(&c));
            let boxes = studio
                .host
                .titles
                .word_boxes(studio.project(), &text, width, height);
            let ordered = caption_fx::spoken_order(&boxes, rtl);
            if count == 0 || ordered.len() != count {
                skipped += 1;
                continue;
            }
            let overlay = TextStyle {
                color: colour.to_owned(),
                background: String::new(),
                shadow: false,
                ..text.clone()
            };
            let slots = caption_fx::word_slots(clip.start, clip.duration, &text.content);
            for (index, (start, duration)) in slots.into_iter().enumerate() {
                let created = studio.apply(Command::AddTextClip {
                    track_id: layer.clone(),
                    above: true,
                    start,
                    style: Some(overlay.clone()),
                    duration: Some(duration),
                    offset_y: Some(clip.offset_y),
                });
                let Some(id) = created else {
                    continue;
                };
                if layer.is_none() {
                    layer = studio.clip(&id).map(|made| made.track_id.clone());
                }
                studio.apply(Command::UpdateClip {
                    clip_id: id,
                    patch: concat_project::commands::ClipPatch {
                        name: Some(caption_fx::HIGHLIGHT_NAME.to_owned()),
                        mask: Some(Some(caption_fx::word_mask(
                            &ordered,
                            index,
                            (width, height),
                        ))),
                        ..concat_project::commands::ClipPatch::default()
                    },
                });
                words_lit += 1;
            }
        }
        if words_lit == 0 {
            self.message = t("captions.highlightNothing");
        } else if skipped > 0 {
            studio.notify(
                &tf("captions.highlightSkipped", &[&words_lit, &skipped]),
                false,
            );
        } else {
            studio.notify(&tf("captions.highlightApplied", &[&words_lit]), false);
        }
    }

    /// Dresses every caption on the lane in the sheet's style, keeping
    /// each one's words and size.
    fn apply_style(&mut self, studio: &mut Studio) {
        let Some(lane) = export_lane(studio) else {
            self.message = t("captions.noCaptionsToExport");
            return;
        };
        let Some(base) = self.base_style(studio) else {
            return;
        };
        let mut commands: Vec<Command> = Vec::new();
        for clip in studio.timeline().clips.iter() {
            if clip.kind != ClipKind::Text || clip.track_id != lane.track_id {
                continue;
            }
            let Some(own) = clip.text.as_ref() else {
                continue;
            };
            let style = TextStyle {
                content: own.content.clone(),
                font_size: own.font_size,
                max_width: own.max_width,
                max_height: own.max_height,
                ..base.clone()
            };
            commands.push(Command::UpdateClip {
                clip_id: clip.id.clone(),
                patch: concat_project::commands::ClipPatch {
                    text: Some(Some(style)),
                    ..concat_project::commands::ClipPatch::default()
                },
            });
        }
        if commands.is_empty() {
            return;
        }
        let count = commands.len();
        studio.apply(Command::Batch { commands });
        studio.notify(&tf("captions.styleApplied", &[&count]), false);
    }

    /// The sheet opened afresh. `clip` is the clip it was opened on, if
    /// any: the switch starts on when that clip has sound, or when a clip's
    /// menu asked for its voice (`voice`), in which case a clip without
    /// sound falls to the first clip that has some.
    fn open_on(&mut self, studio: &Studio, clip: Option<String>, voice: bool) {
        let installed = installed(&studio.settings.transcribers);
        let model = installed.iter().position(|model| model.active).unwrap_or(0);
        let sources = sources(studio);
        let sounding = clip.filter(|id| sources.iter().any(|source| &source.id == id));
        let from_voice = voice || sounding.is_some();
        let clip = sounding.or_else(|| {
            from_voice
                .then(|| sources.first().map(|source| source.id.clone()))
                .flatten()
        });
        let mut gemini = std::mem::take(&mut self.gemini);
        if let Some(id) = studio.prefs.gemini_model.as_deref() {
            gemini.choose_model(id);
        }
        gemini.choose_language(studio.prefs.gemini_language.as_deref());
        gemini.key_saved = OsKeyStore.load().is_ok_and(|key| key.is_some());
        gemini.status.clear();
        *self = CaptionsPane {
            open: true,
            from_voice,
            clip,
            chosen_with: studio.sole_selection(),
            engine: usize::from(studio.prefs.captions_engine.as_deref() == Some("gemini")),
            gemini,
            model,
            placement: 0,
            size: 1,
            timings: std::mem::take(&mut self.timings),
            ..CaptionsPane::default()
        };
    }

    /// The clip the panel transcribes, and the one its list shows - the
    /// same answer for both, so it never runs a clip it is not showing.
    /// The editor stays live under the panel: a clip with sound selected
    /// since the panel's clip was chosen is the one meant now, and is
    /// taken. Otherwise the chosen clip while it is still on the timeline
    /// with sound, and none - nothing to run - when it is not.
    fn subject_clip(&self, studio: &Studio, sources: &[Source]) -> Option<String> {
        follow_selection(
            studio.sole_selection(),
            self.chosen_with.as_deref(),
            self.clip.as_deref(),
            |id| sources.iter().any(|source| source.id == id),
        )
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
            self.message = t("captions.nothingToCaptionYet");
            return;
        }
        let look = self.look();
        let mut at = f64::from(studio.playhead);
        let base = self.base_style(studio);
        let commands: Vec<Command> = lines
            .into_iter()
            .map(|(text, seconds)| {
                let command = caption_clip(text, at, seconds, look, base.as_ref());
                at += seconds;
                command
            })
            .collect();
        let count = commands.len();
        self.open = false;
        studio.apply(Command::Batch { commands });
        studio.notify(&tf("captions.addedCaptions", &[&count]), false);
    }

    /// Whether a key is saved, asked of the credential store.
    fn refresh_key(&mut self) {
        self.gemini.key_saved = OsKeyStore.load().is_ok_and(|key| key.is_some());
    }

    /// The saved key against the service, off the UI thread: it says which
    /// models the key can see, and whether the one chosen is among them.
    fn test_gemini(&mut self) {
        if self.gemini.testing {
            return;
        }
        let key = match OsKeyStore.load() {
            Ok(Some(key)) => key,
            Ok(None) => {
                self.message = gemini_captions::say(&GeminiError::NoKey);
                return;
            }
            Err(error) => {
                self.message = tf("captions.geminiKeyNotSaved", &[&error]);
                return;
            }
        };
        self.gemini.testing = true;
        self.gemini.status.clear();
        self.message.clear();
        crate::host::spawn(
            move || {
                GeminiClient::new(ApiKey::new(&key))
                    .test_connection(None)
                    .map(|models| models.into_iter().map(|model| model.id).collect::<Vec<_>>())
            },
            |studio, _, _, result| studio.handle(Msg::Captions(CaptionsMsg::GeminiTested(result))),
        );
    }

    /// The sheet's clip through Gemini on a worker. The sound goes out in
    /// windows, and the segments that come back are the ones the local
    /// transcriber makes, so they land on the timeline the same way.
    fn run_gemini(&mut self, studio: &mut Studio) {
        let Some(clip) = self
            .subject_clip(studio, &sources(studio))
            .and_then(|id| studio.clip(&id))
            .cloned()
        else {
            self.message = t("captions.clipNoLongerTimeline");
            return;
        };
        let Some(media) = studio.project().media_by_id(&clip.media_id).cloned() else {
            self.message = t("captions.clipHasNoFile");
            return;
        };
        let key = match OsKeyStore.load() {
            Ok(Some(key)) => key,
            Ok(None) => {
                self.message = gemini_captions::say(&GeminiError::NoKey);
                return;
            }
            Err(error) => {
                self.message = tf("captions.geminiKeyNotSaved", &[&error]);
                return;
            }
        };
        let source = gemini_captions::Source {
            path: media.path.clone(),
            audio_stream: clip.audio_stream,
            source_start: clip.source_start,
            window: clip.duration * clip.speed,
        };
        let model = self.gemini.chosen_model();
        let language = self.gemini.language_code();
        let reveal =
            gemini_captions::REVEALS[self.gemini.reveal.min(gemini_captions::REVEALS.len() - 1)];
        let options = JobOptions::new(&model, language, gemini_captions::mode_for(&model));
        let requests = job::estimate(source.window, &options).requests;
        // A request takes a few seconds to a minute; the bar is told what
        // window it is on, and is a guess in between.
        self.pace = Pace::new(vec![
            (Stage::Reading, 1.0),
            (Stage::Working, requests as f32 * 20.0),
        ]);
        self.subject = Some(Subject {
            start: clip.start,
            speed: clip.speed,
            look: self.look(),
            model: model.clone(),
            windows: requests as f32,
        });
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.gemini.cancel = Some(Arc::clone(&cancel));
        self.running = true;
        self.message.clear();
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                gemini_captions::transcribe(
                    &key,
                    &model,
                    language,
                    &source,
                    reveal,
                    cancel,
                    move |stage| {
                        let stage = match stage {
                            GeminiStage::Reading { .. } => JobStage::Reading,
                            GeminiStage::Waiting { n, of } => JobStage::Working {
                                done: (n - 1) as f32 / of as f32,
                                next: n as f32 / of as f32,
                            },
                            GeminiStage::Finishing => JobStage::Working {
                                done: 1.0,
                                next: 1.0,
                            },
                        };
                        on_ui_in_project(epoch, move |studio, _, _| {
                            studio.handle(Msg::Captions(CaptionsMsg::Progress(stage)));
                        });
                    },
                )
            },
            |studio, _, _, result| {
                studio.handle(Msg::Captions(CaptionsMsg::Finished(
                    result.map_err(|error| gemini_captions::say(&error)),
                )));
            },
        );
    }

    /// The sheet's clip through the transcriber on a worker, reporting into
    /// the sheet as it goes.
    fn run_sound(&mut self, studio: &mut Studio) {
        // The clip the panel shows, and no other: see `subject_clip`.
        let Some(clip) = self
            .subject_clip(studio, &sources(studio))
            .and_then(|id| studio.clip(&id))
            .cloned()
        else {
            self.message = t("captions.clipNoLongerTimeline");
            return;
        };
        let Some(media) = studio.project().media_by_id(&clip.media_id).cloned() else {
            self.message = t("captions.clipHasNoFile");
            return;
        };
        let Some((model, megabytes)) = installed(&studio.settings.transcribers)
            .get(self.model)
            .map(|model| (model.id.clone(), model.megabytes))
        else {
            self.message = t("captions.downloadTranscriberModelFirst");
            return;
        };
        let request = concat_speech::transcribe::TranscribeRequest {
            path: media.path.clone(),
            audio_stream: clip.audio_stream,
            source_start: clip.source_start,
            window: clip.duration * clip.speed,
            model_id: model.clone(),
        };
        let dirs = studio.host.dirs.clone();
        let transcriber = Arc::clone(&studio.host.transcriber);
        // The run as the bar lays it out: the sound read in at a few
        // hundred times its length, the model loaded unless it still is,
        // and whisper a window at a time - at the pace the last run on
        // this model measured, or the estimate by its size.
        let windows = (request.window / 30.0).ceil().max(1.0) as f32;
        let learned = self.timings.get(&model);
        let loaded = studio.host.transcriber.loaded_model().as_deref() == Some(model.as_str());
        let load = if loaded {
            0.0
        } else {
            learned.load.unwrap_or(0.5 + megabytes / 400.0)
        };
        let per_window = learned
            .per_unit
            .unwrap_or(0.3 + megabytes * WINDOW_SECONDS_PER_MB);
        self.pace = Pace::new(vec![
            (Stage::Reading, 0.2 + request.window as f32 / 400.0),
            (Stage::Loading, load),
            (Stage::Working, windows * per_window),
        ]);
        self.subject = Some(Subject {
            start: clip.start,
            speed: clip.speed,
            look: self.look(),
            model,
            windows,
        });
        self.running = true;
        self.message.clear();
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                transcriber.transcribe(&dirs, &request, move |stage| {
                    on_ui_in_project(epoch, move |studio, _, _| {
                        studio.handle(Msg::Captions(CaptionsMsg::Progress(stage)));
                    });
                })
            },
            |studio, _, _, result| studio.handle(Msg::Captions(CaptionsMsg::Finished(result))),
        );
    }

    /// The sheet as Slint shows it.
    pub fn data(&self, studio: &Studio) -> CaptionsSheetData {
        let sources = if self.open {
            sources(studio)
        } else {
            Vec::new()
        };
        let source = self
            .subject_clip(studio, &sources)
            .and_then(|id| sources.iter().position(|source| source.id == id))
            .map_or(-1, |index| index as i32);
        let notice = if self.open && self.from_voice && self.engine == 1 {
            self.subject_clip(studio, &sources)
                .and_then(|id| studio.clip(&id))
                .map(|clip| {
                    gemini_captions::notice(clip.duration * clip.speed, &self.gemini.chosen_model())
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        let labels: Vec<slint::SharedString> = sources
            .into_iter()
            .map(|source| source.label.into())
            .collect();
        CaptionsSheetData {
            open: self.open,
            from_voice: self.from_voice,
            sources: std::rc::Rc::new(slint::VecModel::from(labels)).into(),
            source,
            text: self.text.as_str().into(),
            model: self.model as i32,
            placement: self.placement as i32,
            size: self.size as i32,
            running: self.running,
            progress: self.pace.floor,
            ceiling: self.pace.ceiling,
            step: self.pace.step,
            left: self.pace.left,
            stage: match self.pace.stage() {
                Some(Stage::Loading) => t("captions.loadingTheModel"),
                Some(Stage::Working) => t("captions.transcribing"),
                _ => t("captions.readingTheSound"),
            }
            .into(),
            ready: if self.engine == 1 {
                self.gemini.key_saved
            } else {
                !installed(&studio.settings.transcribers).is_empty()
            },
            message: self.message.as_str().into(),
            engine: self.engine as i32,
            gemini_key: self.gemini.key_saved,
            gemini_testing: self.gemini.testing,
            gemini_status: self.gemini.status.as_str().into(),
            gemini_models: std::rc::Rc::new(slint::VecModel::from(
                self.gemini
                    .model_names()
                    .into_iter()
                    .map(slint::SharedString::from)
                    .collect::<Vec<_>>(),
            ))
            .into(),
            gemini_model: self.gemini.model as i32,
            gemini_languages: std::rc::Rc::new(slint::VecModel::from(
                gemini_captions::LANGUAGES
                    .iter()
                    .map(|language| slint::SharedString::from(language.0))
                    .collect::<Vec<_>>(),
            ))
            .into(),
            gemini_language: self.gemini.language as i32,
            gemini_notice: notice.into(),
            gemini_reveal: self.gemini.reveal as i32,
            gemini_reveals: std::rc::Rc::new(slint::VecModel::from(vec![
                slint::SharedString::from(t("captions.revealWhole")),
                slint::SharedString::from(t("captions.revealBuild")),
                slint::SharedString::from(t("captions.revealWordByWord")),
                slint::SharedString::from(t("captions.revealTypewriter")),
            ]))
            .into(),
            motion: self.motion as i32,
            style: self.style as i32,
            highlight: self.highlight as i32,
            highlights: std::rc::Rc::new(slint::VecModel::from(vec![
                slint::SharedString::from(t("captions.highlightNone")),
                slint::SharedString::from(t("captions.highlightYellow")),
                slint::SharedString::from(t("captions.highlightGreen")),
                slint::SharedString::from(t("captions.highlightCyan")),
                slint::SharedString::from(t("captions.highlightPink")),
                slint::SharedString::from(t("captions.highlightOrange")),
            ]))
            .into(),
            styles: std::rc::Rc::new(slint::VecModel::from({
                let mut names = vec![slint::SharedString::from(t("captions.styleKept"))];
                names.extend(
                    style_names(studio)
                        .into_iter()
                        .map(slint::SharedString::from),
                );
                names
            }))
            .into(),
            motions: std::rc::Rc::new(slint::VecModel::from(vec![
                slint::SharedString::from(t("captions.motionNone")),
                slint::SharedString::from(t("captions.motionFade")),
                slint::SharedString::from(t("captions.motionPop")),
                slint::SharedString::from(t("captions.motionSlide")),
                slint::SharedString::from(t("captions.motionScale")),
            ]))
            .into(),
        }
    }
}

/// Imported cues keep the file's timing, including captions shorter than
/// generated speech segments. The title command applies the engine's minimum
/// clip duration where needed.
fn srt_commands(cues: Vec<srt::Cue>, look: (f64, f64), base: Option<&TextStyle>) -> Vec<Command> {
    cues.into_iter()
        .filter(|cue| !cue.text.trim().is_empty())
        .map(|cue| {
            caption_clip(
                cue.text.trim().to_owned(),
                cue.start,
                cue.end - cue.start,
                look,
                base,
            )
        })
        .collect()
}

/// The lane an SRT export writes out.
struct ExportLane {
    track_id: String,
    /// Chosen as the lane with the most titles, not by a selected title.
    busiest: bool,
}

/// The lane whose titles are written out as captions: the selected
/// title's; or with no title selected, the lane with the most titles -
/// captions come many to a lane, a title card one or two, and nothing
/// else in the document tells them apart (see `srt::busiest_title_lane`).
/// None when the timeline has no title.
fn export_lane(studio: &Studio) -> Option<ExportLane> {
    let selected = studio
        .sole_selection()
        .and_then(|id| studio.clip(&id))
        .filter(|clip| clip.kind == ClipKind::Text)
        .map(|clip| clip.track_id.clone());
    match selected {
        Some(track_id) => Some(ExportLane {
            track_id,
            busiest: false,
        }),
        None => srt::busiest_title_lane(studio.timeline()).map(|track_id| ExportLane {
            track_id,
            busiest: true,
        }),
    }
}

/// The name an export is offered under: the project's, as one file name,
/// or "captions" with no project or no usable name.
fn file_stem(studio: &Studio) -> String {
    let name = studio
        .session
        .as_ref()
        .map(|session| session.settings().name)
        .unwrap_or_default();
    let stem: String = name
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let stem = stem.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if stem.is_empty() {
        "captions".to_owned()
    } else {
        stem.to_owned()
    }
}

/// `name` in `dir`, or `name (2)`, `name (3)`... until nothing there is
/// written over.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|path| !path.exists())
        .unwrap_or(first)
}

/// [`CaptionsPane::subject_clip`] with the studio taken out: `selected`
/// the sole selection, `chosen_with` the selection when `clip` was chosen,
/// and `sounds` whether a clip is on the timeline with sound.
fn follow_selection(
    selected: Option<String>,
    chosen_with: Option<&str>,
    clip: Option<&str>,
    sounds: impl Fn(&str) -> bool,
) -> Option<String> {
    if let Some(selected) = selected.filter(|id| sounds(id))
        && Some(selected.as_str()) != chosen_with
    {
        return Some(selected);
    }
    clip.filter(|id| sounds(id)).map(str::to_owned)
}

/// A clip on the timeline whose voice can be captioned, as the sheet's
/// list shows it.
struct Source {
    id: String,
    label: String,
}

/// The clips on the current timeline that have sound, in the order they
/// play: an audio clip, or a video whose file has an audio stream.
fn sources(studio: &Studio) -> Vec<Source> {
    let project = studio.project();
    let mut clips: Vec<_> = studio
        .timeline()
        .clips
        .iter()
        .filter(|clip| studio.clip_has_sound(clip))
        .map(|clip| {
            let name = project
                .media_by_id(&clip.media_id)
                .map_or(clip.name.as_str(), |media| media.name.as_str());
            (clip.start, clip.id.clone(), name.to_owned())
        })
        .collect();
    clips.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    clips
        .into_iter()
        .map(|(start, id, name)| Source {
            id,
            label: source_label(&name, start),
        })
        .collect()
}

/// A clip as the list names it: its file and where it starts on the
/// timeline, so two clips of one file are told apart.
fn source_label(name: &str, start: f64) -> String {
    let seconds = start.max(0.0).floor() as u64;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{name} · {hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{name} · {minutes}:{seconds:02}")
    }
}

/// One caption as a title clip: its words, when and for how long, and
/// its look.
fn caption_clip(
    text: String,
    start: f64,
    duration: f64,
    look: (f64, f64),
    base: Option<&TextStyle>,
) -> Command {
    let (offset_y, font_size) = look;
    let arabic = text.chars().any(|c| ('\u{0600}'..='\u{06FF}').contains(&c));
    // The kept look where there is one - family, weight, colours, stroke,
    // shadow, background - at the caption's own size and place; the
    // bundled face otherwise.
    let style = match base {
        Some(base) => TextStyle {
            content: text,
            font_size,
            max_width: 0.0,
            max_height: 0.0,
            ..base.clone()
        },
        None => TextStyle {
            content: text,
            font_family: if arabic {
                "Noto Sans Arabic UI".to_owned()
            } else {
                "Hanken Grotesk".to_owned()
            },
            font_size,
            font_weight: 600.0,
            ..TextStyle::default()
        },
    };
    Command::AddTextClip {
        track_id: None,
        above: true,
        start,
        style: Some(style),
        duration: Some(duration),
        offset_y: Some(offset_y),
    }
}

/// The ids of the presets the sheet offers as styles, Persian ones first:
/// that is the writing the subtitle tool is for.
fn style_ids(studio: &Studio) -> Vec<String> {
    let mut persian: Vec<String> = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    for preset in &studio.text_presets {
        if preset.id == "default" {
            continue;
        }
        if preset.language == "Persian" {
            persian.push(preset.id.clone());
        } else {
            rest.push(preset.id.clone());
        }
    }
    persian.extend(rest);
    persian
}

/// Their names, in the same order.
fn style_names(studio: &Studio) -> Vec<String> {
    style_ids(studio)
        .iter()
        .filter_map(|id| studio.text_presets.iter().find(|held| &held.id == id))
        .map(|preset| preset.name.clone())
        .collect()
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
    use super::{script_captions, source_label};

    #[test]
    fn imported_srt_preserves_short_cues_and_multiline_text() {
        let cues = concat_project::srt::parse(
            "1\n00:00:01,000 --> 00:00:01,100\nOlá\nAção\n\n2\n00:00:02,000 --> 00:00:03,000\nFim\n",
        ).unwrap();
        let commands = super::srt_commands(cues.clone(), (0.35, 0.04), None);
        let mut editor = concat_project::Editor::new();
        editor
            .apply(concat_project::Command::Batch { commands })
            .unwrap();
        let exported = concat_project::srt::write(&concat_project::srt::title_cues(
            editor.project().active(),
            None,
        ));
        assert_eq!(concat_project::srt::parse(&exported).unwrap(), cues);
        assert!(editor.undo());
        assert!(editor.project().active().clips.is_empty());
        assert!(editor.redo());
        assert_eq!(
            concat_project::srt::title_cues(editor.project().active(), None).len(),
            2
        );
    }

    /// The panel floats over a live editor: a clip with sound selected
    /// since the panel's clip was chosen is the one it shows and runs; a
    /// pick from its list holds until the selection moves again; and a
    /// chosen clip gone from the timeline is not run unseen.
    #[test]
    fn the_captioned_clip_follows_a_new_selection_of_a_clip_with_sound() {
        let sounds = |id: &str| ["a", "b", "c"].contains(&id);
        let follow = |selected: Option<&str>, chosen_with: Option<&str>, clip: Option<&str>| {
            super::follow_selection(selected.map(str::to_owned), chosen_with, clip, sounds)
        };
        // Opened on A, A still selected: A.
        assert_eq!(
            follow(Some("a"), Some("a"), Some("a")).as_deref(),
            Some("a")
        );
        // B selected on the timeline since: B.
        assert_eq!(
            follow(Some("b"), Some("a"), Some("a")).as_deref(),
            Some("b")
        );
        // C picked from the list with B selected: C, until the selection moves.
        assert_eq!(
            follow(Some("b"), Some("b"), Some("c")).as_deref(),
            Some("c")
        );
        // A title selected - no sound - leaves the choice alone.
        assert_eq!(
            follow(Some("t"), Some("b"), Some("c")).as_deref(),
            Some("c")
        );
        // The chosen clip deleted: nothing to run, rather than a clip unseen.
        assert_eq!(follow(None, None, Some("gone")), None);
    }

    #[test]
    fn imported_srt_ignores_empty_captions() {
        let cues = vec![concat_project::srt::Cue {
            start: 0.0,
            end: 1.0,
            text: "  ".into(),
        }];
        assert!(super::srt_commands(cues, (0.35, 0.04), None).is_empty());
    }

    /// A clip in the voice list is its file and where it starts, in
    /// minutes and seconds, with hours once there are any.
    #[test]
    fn a_voice_is_named_by_its_file_and_where_it_starts() {
        assert_eq!(source_label("Interview.mp4", 0.0), "Interview.mp4 · 0:00");
        assert_eq!(source_label("Interview.mp4", 72.9), "Interview.mp4 · 1:12");
        assert_eq!(source_label("Talk.wav", 3_725.0), "Talk.wav · 1:02:05");
        assert_eq!(source_label("Talk.wav", -1.0), "Talk.wav · 0:00");
    }

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
