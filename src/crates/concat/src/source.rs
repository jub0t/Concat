// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The source preview: a file from the library seen and heard on its own,
//! before it is anywhere near the timeline (#197).
//!
//! A click on a video or a still in the bin shows it on the monitor,
//! paused at its start, with a chip that names it; Play and Space play it,
//! the monitor's timecode moves through it. A click on a sound plays it at
//! once, with a line moving across its card; a second click pauses it.
//! The timeline is not touched: the monitor and the speakers are lent a
//! scratch copy of the project holding the one clip, and given back the
//! moment the person clicks the timeline or the chip's cross.

use std::sync::Arc;

use concat_doc::commands::{self, Command};
use concat_doc::model::{Media, MediaKind, Model};
use concat_doc::{Document, Origin, Tick};

use crate::doc_view;
use crate::studio::Studio;
use crate::ui::SourcePreview;
use slint::{ComponentHandle, SharedString};

/// The file lent to the monitor and the speakers.
pub struct Source {
    pub media_id: String,
    pub name: String,
    pub kind: MediaKind,
    /// The bin's row for the file, for its card to show the playing line;
    /// -1 for a sound from the Audio page.
    pub row: i32,
    /// The Audio page's card for the file, or empty for one in the bin.
    pub card: String,
    /// The scratch project flattened, as the monitor draws it.
    pub clips: Arc<Vec<concat_export::ExportClip>>,
    pub duration: f64,
    pub time: f64,
    pub playing: bool,
    /// Follows the speakers' clock while it plays.
    timer: slint::Timer,
}

/// A document of its own: one timeline in the frame of the one showing in
/// `doc`, `media` in its bin and one clip of it from zero, and how long that
/// clip runs. What the source preview draws and plays; nothing of it
/// reaches the project.
fn scratch(doc: &Model, media: &Media) -> Option<(Document<Model>, f64)> {
    fn run(scratch: &mut Document<Model>, command: Command) -> Option<commands::Applied> {
        let outcome = commands::run(scratch, &command, Origin(0)).ok()?;
        if let Some(commit) = outcome.commit {
            *scratch = commit.into_document();
        }
        Some(outcome.applied)
    }
    let mut scratch: Document<Model> = Document::new();
    run(&mut scratch, Command::AddTimeline)?;
    let sequence = doc_view::showing(&scratch)?;
    if let Some(shown) = doc_view::sequence(doc) {
        run(
            &mut scratch,
            Command::SetTimelineVideo {
                sequence,
                frame: shown.frame.clone(),
            },
        )?;
    }
    let added = run(
        &mut scratch,
        Command::AddMedia {
            media: media.clone(),
        },
    )?;
    let media = added.created?.typed()?;
    let lane = *doc_view::lanes(&scratch).first()?;
    run(
        &mut scratch,
        Command::AddClip {
            media,
            lane,
            start: Tick::ZERO,
            ripple: false,
        },
    )?;
    let duration = doc_view::items(&scratch)
        .first()
        .map_or(0.0, |(_, item)| doc_view::len_s(item));
    Some((scratch, duration))
}

impl Studio {
    /// A card was clicked. A sound plays, or pauses when it is the one
    /// playing; a picture goes on the monitor, paused at its start.
    pub fn preview_media(&mut self, media_id: &str, row: i32) {
        if let Some(source) = self.source.as_ref()
            && source.media_id == media_id
            && source.card.is_empty()
        {
            if source.kind == MediaKind::Audio {
                self.source_toggle();
            }
            return;
        }
        let Some(media) = doc_view::parse(media_id)
            .and_then(|id| doc_view::media(self.committed(), id))
            .cloned()
        else {
            return;
        };
        let Some(project_dir) = self
            .session
            .as_ref()
            .map(|session| std::path::PathBuf::from(session.path()))
        else {
            return;
        };
        let Some((scratch, duration)) = scratch(self.committed(), &media) else {
            return;
        };
        // The timeline stops: the speakers and the monitor are the
        // source's now.
        self.pause();
        if let Some(old) = self.source.take() {
            old.timer.stop();
        }
        let clips = Arc::new(doc_view::flattened(&scratch, Some(&project_dir)));
        self.host
            .playback
            .set_clips(project_dir, crate::studio::audio_specs(&clips));
        self.host.playback.seek(0.0);
        let sound = media.kind == MediaKind::Audio;
        self.source = Some(Source {
            media_id: media_id.to_owned(),
            name: media.name.clone(),
            kind: media.kind.clone(),
            row,
            card: String::new(),
            clips,
            duration,
            time: 0.0,
            playing: false,
            timer: slint::Timer::default(),
        });
        if sound {
            self.source_toggle();
        }
        self.request_preview();
    }

    /// A card on the Audio page was clicked: its sound plays through the
    /// speakers as a sound from the bin does, without being in the bin, or
    /// pauses when it is the one playing. `card` names it for its card's
    /// playing line.
    pub fn preview_file(&mut self, card: &str, name: &str, path: &std::path::Path) {
        if let Some(source) = self.source.as_ref()
            && source.card == card
        {
            self.source_toggle();
            return;
        }
        let Some(project_dir) = self
            .session
            .as_ref()
            .map(|session| std::path::PathBuf::from(session.path()))
        else {
            return;
        };
        let summary = match concat_host::media::probe(&path.to_string_lossy()) {
            Ok(summary) => summary,
            Err(error) => {
                self.notify(&crate::host::probe_error(&error), true);
                return;
            }
        };
        // A document of its own with the file in its bin and the one clip
        // cut from it: the project's bin is not touched.
        let Some((scratch, duration)) = self::scratch(self.committed(), &summary.to_new_media())
        else {
            return;
        };
        self.pause();
        if let Some(old) = self.source.take() {
            old.timer.stop();
        }
        let clips = Arc::new(doc_view::flattened(&scratch, Some(&project_dir)));
        self.host
            .playback
            .set_clips(project_dir, crate::studio::audio_specs(&clips));
        self.host.playback.seek(0.0);
        // No id of the bin: the file is not in it, and the card names it.
        self.source = Some(Source {
            media_id: String::new(),
            name: name.to_owned(),
            kind: MediaKind::Audio,
            row: -1,
            card: card.to_owned(),
            clips,
            duration,
            time: 0.0,
            playing: false,
            timer: slint::Timer::default(),
        });
        self.source_toggle();
        self.request_preview();
    }

    /// Back to the timeline: its picture on the monitor, its sound in the
    /// speakers, the transport where the playhead was.
    pub fn close_source(&mut self) {
        let Some(source) = self.source.take() else {
            return;
        };
        source.timer.stop();
        self.host.playback.pause();
        self.sync_audio();
        self.host.playback.seek(f64::from(self.playhead));
        self.request_preview();
    }

    /// Play or pause the source, from where it stands; from its start
    /// again once it has played to the end.
    pub fn source_toggle(&mut self) {
        let Some(source) = self.source.as_mut() else {
            return;
        };
        if source.playing {
            source.playing = false;
            source.timer.stop();
            self.host.playback.pause();
            return;
        }
        if source.time >= source.duration - 1e-3 {
            source.time = 0.0;
        }
        source.playing = true;
        self.host.playback.play(source.time);
        source.timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(16),
            || {
                crate::host::Shell::with(|shell, app| {
                    let mut studio = shell.studio.borrow_mut();
                    let position = studio.host.playback.position_now();
                    let Some(source) = studio.source.as_mut() else {
                        return;
                    };
                    source.time = position.min(source.duration);
                    let ended = source.time >= source.duration;
                    if ended {
                        source.playing = false;
                        source.timer.stop();
                    }
                    if ended {
                        studio.host.playback.pause();
                    }
                    studio.request_preview();
                    studio.publish_source(&app);
                });
            },
        );
        self.wake_meters();
    }

    /// The monitor's timecode moved through the source.
    pub fn source_seek(&mut self, seconds: f64) {
        let Some(source) = self.source.as_mut() else {
            return;
        };
        source.time = seconds.clamp(0.0, source.duration);
        let time = source.time;
        if source.playing {
            self.host.playback.play(time);
        } else {
            self.host.playback.seek(time);
        }
        self.request_preview();
    }

    /// What the window shows of the source: the chip, the card's line, the
    /// monitor's readout.
    pub fn publish_source(&self, app: &crate::ui::App) {
        let preview = app.global::<SourcePreview>();
        match self.source.as_ref() {
            Some(source) => {
                preview.set_open(true);
                preview.set_name(source.name.as_str().into());
                preview.set_row(source.row);
                preview.set_card(source.card.as_str().into());
                preview.set_time(source.time as f32);
                preview.set_duration(source.duration as f32);
                preview.set_playing(source.playing);
                preview.set_progress(if source.duration > 0.0 {
                    (source.time / source.duration) as f32
                } else {
                    0.0
                });
            }
            None => {
                preview.set_open(false);
                preview.set_row(-1);
                preview.set_card(SharedString::new());
                preview.set_playing(false);
            }
        }
    }
}
