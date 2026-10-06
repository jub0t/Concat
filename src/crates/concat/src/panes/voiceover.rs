// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The voiceover: words recorded from the microphone over the picture,
//! and laid on the timeline where they were spoken.
//!
//! The tray's button starts a take at the playhead: the picture rolls so
//! the words can follow it, the timeline's own sound held silent so the
//! speakers do not bleed into the microphone, and a meter beside the button
//! shows the level. The same button stops it. The file goes into the
//! project's audio folder and onto the lowest free lane, at the playhead
//! less nothing and plus the moment the microphone actually began.

use std::time::Instant;

use concat_host::media;
use concat_host::record::{self, Recorder};
use concat_project::Command;

use crate::host::spawn_in_project;
use crate::i18n::{t, tf};
use crate::panes::Msg;
use crate::studio::Studio;
use crate::ui::Editor;
use slint::ComponentHandle;

/// Everything that can happen to a take.
pub enum VoiceoverMsg {
    /// The tray's button: start a take, or stop the one running.
    Toggle,
    /// The take is written and probed: onto the timeline with it.
    Finished(Box<Result<media::MediaSummary, String>>),
}

impl std::fmt::Debug for VoiceoverMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Toggle => write!(f, "Toggle"),
            Self::Finished(result) => match result.as_ref() {
                Ok(summary) => write!(f, "Finished({})", summary.path),
                Err(error) => write!(f, "Finished(error {error:?})"),
            },
        }
    }
}

/// The take in progress, if one is.
#[derive(Default)]
pub struct VoiceoverPane {
    recorder: Option<Recorder>,
    /// Where the take began on the timeline.
    playhead: f64,
    /// When the picture started rolling under it.
    rolled: Option<Instant>,
    /// Where the take lands, worked out when it stopped.
    landing: f64,
    /// Feeds the meter while a take runs.
    meter: Option<slint::Timer>,
}

impl VoiceoverPane {
    /// Whether a take is running: the button reads Stop, the meter shows.
    pub fn recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// Applies one message.
    pub fn update(&mut self, msg: VoiceoverMsg, studio: &mut Studio) {
        match msg {
            VoiceoverMsg::Toggle if self.recorder.is_some() => self.stop(studio),
            VoiceoverMsg::Toggle => self.start(studio),
            VoiceoverMsg::Finished(result) => match *result {
                Ok(summary) => {
                    // The user's own media, like an import: shelved under
                    // Audio, not under Generated.
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
                    if let Some(media_id) = media_id {
                        studio.apply(Command::AddClipAtFirstFree {
                            media_id,
                            start: self.landing,
                        });
                        studio.notify(&t("voiceover.added"), false);
                    }
                }
                Err(error) => studio.notify(&tf("voiceover.failed", &[&error]), true),
            },
        }
    }

    /// Opens the microphone and rolls the picture from the playhead.
    fn start(&mut self, studio: &mut Studio) {
        let Some(project) = studio
            .session
            .as_ref()
            .map(|session| std::path::PathBuf::from(session.path()))
        else {
            return;
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or_default();
        let path = project.join("audio").join(format!("voiceover-{stamp}.wav"));
        let recorder = match Recorder::start(path) {
            Ok(recorder) => recorder,
            Err(error) => {
                studio.notify(&tf("voiceover.failed", &[&error]), true);
                return;
            }
        };
        log::info!("voiceover: recording from {:.3}s", studio.playhead);
        self.recorder = Some(recorder);
        self.playhead = f64::from(studio.playhead.max(0.0));
        // The picture rolls under the words, without its sound. At the end
        // of the cut there is nothing to roll, and the take just records.
        studio.host.playback.set_muted(true);
        if !studio.playing && studio.playhead < studio.duration() {
            studio.play_toggle();
        }
        self.rolled = Some(Instant::now());
        self.meter = Some(meter());
    }

    /// Stops the take and the picture, and sends the file to be probed.
    fn stop(&mut self, studio: &mut Studio) {
        self.meter = None;
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        if studio.playing {
            studio.pause();
        }
        studio.host.playback.set_muted(false);
        let take = recorder.stop();
        let rolled = self.rolled.take().unwrap_or_else(Instant::now);
        match take {
            Ok(take) => {
                log::info!("voiceover: {:.2}s to {}", take.seconds, take.path.display());
                self.landing = record::landing(self.playhead, rolled, take.first);
                spawn_in_project(
                    move || media::probe(&take.path.to_string_lossy()),
                    |studio, _, _, result| {
                        studio.handle(Msg::Voiceover(VoiceoverMsg::Finished(Box::new(result))));
                    },
                );
            }
            Err(error) => studio.notify(&tf("voiceover.failed", &[&error]), true),
        }
    }
}

/// A timer that shows the take's level on the tray's meter thirty times a
/// second, straight onto the one property, with no publish around it.
fn meter() -> slint::Timer {
    let timer = slint::Timer::default();
    let mut shown = 0.0f32;
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(33),
        move || {
            crate::host::Shell::with(|shell, app| {
                let studio = shell.studio.borrow();
                let Some(recorder) = studio.voiceover.recorder.as_ref() else {
                    return;
                };
                // Up at once, down gently: a meter that falls as fast as
                // it rises flickers.
                shown = recorder.level().max(shown * 0.85);
                app.global::<Editor>().set_record_level(shown);
            });
        },
    );
    timer
}
