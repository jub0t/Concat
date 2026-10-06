// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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

use concat_project::Command;
use concat_project::commands::{self, IdMint};
use concat_project::model::{MediaKind, Project};

use crate::studio::Studio;
use crate::ui::SourcePreview;
use slint::ComponentHandle;

/// The file lent to the monitor and the speakers.
pub struct Source {
    pub media_id: String,
    pub name: String,
    pub kind: MediaKind,
    /// The bin's row for the file, for its card to show the playing line.
    pub row: i32,
    /// The scratch project flattened, as the monitor draws it.
    pub clips: Arc<Vec<concat_export::ExportClip>>,
    pub duration: f64,
    pub time: f64,
    pub playing: bool,
    /// Follows the speakers' clock while it plays.
    timer: slint::Timer,
}

/// The project as it is, with its active timeline holding only `media_id`
/// from zero: what the source preview draws and plays.
fn scratch(project: &Project, media_id: &str) -> Option<Project> {
    let mut scratch = project.clone();
    let timeline = scratch.active_mut();
    timeline.clips.clear();
    let track_id = timeline.tracks.first()?.id.clone();
    commands::apply(
        &mut scratch,
        &mut IdMint::default(),
        Command::AddClip {
            media_id: media_id.to_owned(),
            track_id,
            start: 0.0,
            ripple: false,
        },
    )
    .ok()?;
    Some(scratch)
}

impl Studio {
    /// A card was clicked. A sound plays, or pauses when it is the one
    /// playing; a picture goes on the monitor, paused at its start.
    pub fn preview_media(&mut self, media_id: &str, row: i32) {
        if let Some(source) = self.source.as_ref()
            && source.media_id == media_id
        {
            if source.kind == MediaKind::Audio {
                self.source_toggle();
            }
            return;
        }
        let Some(media) = self.project().media_by_id(media_id).cloned() else {
            return;
        };
        let Some(project_dir) = self
            .session
            .as_ref()
            .map(|session| std::path::PathBuf::from(session.path()))
        else {
            return;
        };
        let Some(scratch) = scratch(self.project(), media_id) else {
            return;
        };
        // The timeline stops: the speakers and the monitor are the
        // source's now.
        self.pause();
        if let Some(old) = self.source.take() {
            old.timer.stop();
        }
        let clips = Arc::new(concat_export::flatten::flatten_timeline_in(
            &scratch,
            None,
            Some(&project_dir),
        ));
        let duration = scratch
            .active()
            .clips
            .first()
            .map_or(0.0, |clip| clip.duration);
        self.host
            .playback
            .set_clips(project_dir, crate::studio::audio_specs(&clips));
        self.host.playback.seek(0.0);
        self.source = Some(Source {
            media_id: media_id.to_owned(),
            name: media.name.clone(),
            kind: media.kind,
            row,
            clips,
            duration,
            time: 0.0,
            playing: false,
            timer: slint::Timer::default(),
        });
        if media.kind == MediaKind::Audio {
            self.source_toggle();
        }
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
        self.meters.wake();
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
                preview.set_playing(false);
            }
        }
    }
}
