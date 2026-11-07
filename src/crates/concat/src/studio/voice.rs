// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Voice isolation's jobs: each media file a clip asks to have its voice
//! set apart is split once, in the background, into its voice and the
//! rest, and every clip cut from it - a split, a copy - plays from the two.
//! See `concat_speech::isolate` for the engine and `VoiceMix` for what a
//! clip asks.

use super::*;

/// What isolating a file needs to know: where its sound is and where its
/// two parts go.
struct VoiceRequest {
    /// The parts' folder; also the job's name.
    dir: std::path::PathBuf,
    source: std::path::PathBuf,
    stream: Option<usize>,
    kind: concat_project::model::VoiceKind,
}

impl Studio {
    /// The parts of `clip`'s sound, as the flattener looks for them.
    fn voice_dir(&self, clip: &Clip) -> Option<std::path::PathBuf> {
        let session = self.session.as_ref()?;
        let media = self.project().media_by_id(&clip.media_id)?;
        Some(concat_export::voice_stems_dir(
            std::path::Path::new(session.path()),
            &media.path,
            clip.audio_stream,
            clip.voice?.kind,
        ))
    }

    /// Whether `clip`'s sound has been split.
    fn voice_ready(dir: &std::path::Path) -> bool {
        dir.join("voice.wav").is_file() && dir.join("rest.wav").is_file()
    }

    /// How far splitting `clip`'s sound has got, 0 to 1, or -1 when nothing
    /// is splitting it: done, failed, or never asked; and whether that is
    /// the model still downloading.
    pub(super) fn voice_progress(&self, clip: &Clip) -> (f32, bool) {
        self.voice_dir(clip)
            .and_then(|dir| self.voice_jobs.get(&dir).copied())
            .map_or((-1.0, false), |(fetching, fraction)| (fraction, fetching))
    }

    /// Starts splitting the next file a clip asks to have split, if nothing
    /// is splitting already. One at a time: the engine spreads one file
    /// over the cores itself.
    pub fn ensure_voices(&mut self) {
        if !self.voice_jobs.is_empty() {
            return;
        }
        let mut next = None;
        for clip in &self.timeline().clips {
            if clip.voice.is_none() {
                continue;
            }
            let Some(dir) = self.voice_dir(clip) else {
                continue;
            };
            if Self::voice_ready(&dir) || self.voice_failed.contains(&dir) {
                continue;
            }
            let Some(media) = self.project().media_by_id(&clip.media_id) else {
                continue;
            };
            if !media.has_audio {
                continue;
            }
            next = Some(VoiceRequest {
                dir,
                source: std::path::PathBuf::from(&media.path),
                stream: clip.audio_stream.map(|index| index as usize),
                kind: clip.voice.unwrap_or_default().kind,
            });
            break;
        }
        let Some(request) = next else {
            return;
        };
        let vocals = request.kind == concat_project::model::VoiceKind::Vocals;
        self.voice_jobs.insert(request.dir.clone(), (vocals, 0.0));
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.voice_cancel = Some(Arc::clone(&cancel));
        let epoch = crate::host::project_epoch();
        let memory = concat_host::memory::total();
        let workers = if vocals {
            concat_speech::vocals::workers_for(memory)
        } else {
            concat_speech::isolate::workers_for(memory)
        };
        let data = self.host.dirs.data.clone();
        log::info!(
            "voice: splitting {} on {workers} worker(s)",
            request.source.display()
        );
        let started = std::time::Instant::now();
        spawn_in_project(
            move || {
                let stems = concat_speech::isolate::Stems {
                    voice: request.dir.join("voice.wav"),
                    rest: request.dir.join("rest.wav"),
                };
                let reporting = request.dir.clone();
                let report = move |fetching: bool, fraction: f32| {
                    let dir = reporting.clone();
                    on_ui_in_project(epoch, move |studio, _, _| {
                        if let Some(held) = studio.voice_jobs.get_mut(&dir) {
                            *held = (fetching, fraction);
                        }
                    });
                };
                let mut last = -1.0f32;
                let mut step = move |fetching: bool, fraction: f32| {
                    // Every percent, not every chunk's tick.
                    if (fraction - last).abs() >= 0.01 || fraction == 0.0 {
                        last = fraction;
                        report(fetching, fraction);
                    }
                };
                let result = std::fs::create_dir_all(&request.dir)
                    .map_err(|error| format!("could not create {}: {error}", request.dir.display()))
                    .and_then(|()| match request.kind {
                        concat_project::model::VoiceKind::Speech => {
                            concat_speech::isolate::isolate(
                                &request.source,
                                request.stream,
                                &stems,
                                workers,
                                &cancel,
                                &mut |fraction| step(false, fraction),
                            )
                        }
                        concat_project::model::VoiceKind::Vocals => {
                            // The two networks first, the first time: 39 MB.
                            let mut fetch = |id| {
                                concat_host::cutout::fetch_model(
                                    &data,
                                    id,
                                    &cancel,
                                    &mut |got, total| {
                                        step(true, got as f32 / total.max(1) as f32);
                                    },
                                )
                            };
                            let networks = concat_speech::vocals::Networks {
                                vocals: fetch(concat_vision::models::ModelId::SpleeterVocals)?,
                                accompaniment: fetch(
                                    concat_vision::models::ModelId::SpleeterAccompaniment,
                                )?,
                            };
                            step(false, 0.0);
                            concat_speech::vocals::separate(
                                &request.source,
                                request.stream,
                                &networks,
                                &stems,
                                workers,
                                &cancel,
                                &mut |fraction| step(false, fraction),
                            )
                        }
                    });
                (request.dir, result)
            },
            move |studio, _, _, (dir, result)| {
                studio.voice_jobs.remove(&dir);
                studio.voice_cancel = None;
                match result {
                    Ok(()) => {
                        log::info!("voice: split in {:.1} s", started.elapsed().as_secs_f64());
                        // The clips now play from the parts: the mix is
                        // handed over again, and the waveforms redrawn.
                        studio.flat = None;
                        studio.sync_audio();
                        studio.request_media_art();
                        studio.ensure_voices();
                    }
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => {
                        log::error!("voice: {error}");
                        studio.voice_failed.insert(dir);
                        studio.notify(&tf("studio.voiceFailed", &[&error]), true);
                        studio.ensure_voices();
                    }
                }
            },
        );
    }

    /// Sets the voice levels of every selected clip that has sound, or
    /// takes isolation off them: one undo step. A file that failed to
    /// split before is tried again, since asking again is asking.
    pub fn set_voice(&mut self, mix: Option<concat_project::model::VoiceMix>) {
        let ids: Vec<String> = self
            .selection
            .iter()
            .filter(|id| {
                self.clip(id).is_some_and(|clip| {
                    matches!(
                        clip.kind,
                        concat_project::model::ClipKind::Video
                            | concat_project::model::ClipKind::Audio
                    )
                })
            })
            .cloned()
            .collect();
        if ids.is_empty() {
            return;
        }
        for id in &ids {
            if let Some(dir) = self.clip(id).and_then(|clip| self.voice_dir(clip)) {
                self.voice_failed.remove(&dir);
            }
        }
        let commands = ids
            .into_iter()
            .map(|clip_id| Command::SetClipVoice {
                clip_id,
                voice: mix,
            })
            .collect();
        self.apply(Command::Batch { commands });
    }

    /// Stops a split in progress, for a project closing.
    pub(super) fn cancel_voices(&mut self) {
        if let Some(cancel) = self.voice_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.voice_jobs.clear();
        self.voice_failed.clear();
    }
}
