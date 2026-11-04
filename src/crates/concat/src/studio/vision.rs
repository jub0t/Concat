// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah
//! Cutouts, stabilization and Enhance.

use super::*;

impl Studio {
    // ── cutouts ──
    //
    // A cutout is a mask per source instant, found by the host's model and
    // cached in the project folder. The window's part is to notice which
    // media need masks they do not have, run one analysis at a time, and
    // turn a brush on the stage into strokes on the document.

    /// Starts the analysis for the first media whose cutout wants masks
    /// that are not there, unless one is running. Called after every
    /// change; the finished job calls it again for whatever is next.
    pub fn ensure_cutouts(&mut self) {
        if !self.cutout_jobs.is_empty() || self.host.cutouts.is_busy() {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        // One analysis per media and subject, keyed the way the job map is.
        let wanted: Vec<(String, AnalyseRequest)> = Cutouts::requests(self.project(), &project)
            .into_iter()
            .map(|(media_id, request)| (Self::analysis_key(&media_id, request.subject), request))
            .collect();
        // One the app died analysing last time is skipped, and said once:
        // starting it again by itself closed the app on every open.
        let mut next = None;
        for (id, request) in wanted {
            if Cutouts::outstanding(&request) == 0 {
                continue;
            }
            if Cutouts::tripped(&request) {
                let dir =
                    concat_vision::mask_dir(&request.project, &request.media_path, request.subject);
                if self.tripped_strokes.insert(dir) {
                    log::warn!("cutout: media the app closed while analysing is skipped");
                    self.notify(&t("studio.removeBackgroundSkipped"), true);
                }
                continue;
            }
            next = Some((id, request));
            break;
        }
        let Some((id, request)) = next else {
            return;
        };
        self.cutout_jobs.insert(id.clone(), (false, 0.0));
        let cutouts = Arc::clone(&self.host.cutouts);
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                let mut last = (false, -1.0f32);
                let reporting = id.clone();
                let result = cutouts.analyse(&request, &mut |progress| {
                    // Every percent, not every frame: the readout cannot
                    // use more and the event loop has other work.
                    let now = match progress {
                        concat_host::cutout::Progress::Fetching { received, total } => {
                            (true, received as f32 / total.max(1) as f32)
                        }
                        concat_host::cutout::Progress::Analysing(fraction) => (false, fraction),
                    };
                    if now.0 != last.0 || now.1 - last.1 >= 0.01 {
                        last = now;
                        let id = reporting.clone();
                        on_ui_in_project(epoch, move |studio, _, _| {
                            if let Some(held) = studio.cutout_jobs.get_mut(&id) {
                                *held = now;
                            }
                        });
                    }
                });
                (id, result)
            },
            |studio, _, _, (id, result)| {
                studio.cutout_jobs.remove(&id);
                match result {
                    Ok(_) => {
                        // The monitor shows the cut, and whatever else is
                        // waiting gets its turn.
                        studio.request_preview();
                        studio.ensure_cutouts();
                    }
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => studio.notify(&tf("studio.removeBackground", &[&error]), true),
                }
            },
        );
    }

    // ── stabilization ──
    //
    // A stabilized clip's camera movement is measured once per stretch of
    // its media, by the host, and cached in the project folder; the
    // correction is worked out from it whenever the timeline is built. The
    // window's part is to notice which media have stretches not measured,
    // run one analysis at a time, and show the clip steadied when it lands.

    /// Starts the analysis for the first media a stabilized clip needs
    /// measured, unless one is running. Called after every change; the
    /// finished job calls it again for whatever is next.
    pub fn ensure_stabilizers(&mut self) {
        if !self.stabilize_jobs.is_empty() || self.host.stabilizers.is_busy() {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        let mut next = None;
        for (media_id, request) in concat_host::Stabilizers::requests(self.project(), &project) {
            if concat_host::Stabilizers::outstanding(&request).is_empty() {
                continue;
            }
            let dir = concat_host::Stabilizers::dir(&request);
            if concat_host::Stabilizers::tripped(&request) {
                if self.stabilize_retry.remove(&media_id) {
                    concat_host::jobs::Tripwire::clear(&dir);
                } else {
                    // Skipped, and said once: starting it again by itself
                    // would close the app on every open.
                    if self.tripped_strokes.insert(dir) {
                        log::warn!("stabilize: media the app closed while measuring is skipped");
                        self.notify(&t("studio.stabilizeSkipped"), true);
                    }
                    continue;
                }
            }
            next = Some((media_id, request));
            break;
        }
        let Some((media_id, request)) = next else {
            return;
        };
        self.stabilize_jobs.insert(media_id.clone(), 0.0);
        let stabilizers = Arc::clone(&self.host.stabilizers);
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                let mut last = -1.0f32;
                let reporting = media_id.clone();
                let result = stabilizers.analyse(&request, &mut |fraction| {
                    // Every percent, not every frame.
                    if fraction - last >= 0.01 {
                        last = fraction;
                        let id = reporting.clone();
                        on_ui_in_project(epoch, move |studio, _, _| {
                            if let Some(held) = studio.stabilize_jobs.get_mut(&id) {
                                *held = fraction;
                            }
                        });
                    }
                });
                (media_id, result)
            },
            |studio, _, _, (media_id, result)| {
                studio.stabilize_jobs.remove(&media_id);
                match result {
                    Ok(_) => {
                        // The kept plan was built before the measurement
                        // and would go on showing the clip as shot.
                        studio.host.monitor.forget_plan();
                        studio.request_preview();
                        studio.ensure_stabilizers();
                    }
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => studio.notify(&tf("studio.stabilize", &[&error]), true),
                }
            },
        );
    }

    /// The zoom a stabilized clip is drawn at and the zoom it would need
    /// to steady every frame, once its media's movement is measured.
    pub(super) fn stabilize_readout(&self, clip: &Clip) -> Option<(f64, f64)> {
        let (correction, _) = self.stabilize_correction(clip)?;
        Some((correction.zoom, correction.needed))
    }

    /// A stabilized clip's correction, worked out as the renderer works it
    /// out, with the picture's aspect it is measured in; `None` for a clip
    /// not stabilized or not measured yet.
    fn stabilize_correction(
        &self,
        clip: &Clip,
    ) -> Option<(concat_vision::stabilize::Correction, f64)> {
        let settings = clip.stabilize?;
        let session = self.session.as_ref()?;
        let media = self.project().media_by_id(&clip.media_id)?;
        let dir =
            concat_vision::stabilize::motion_dir(std::path::Path::new(session.path()), &media.path);
        let motion = concat_vision::stabilize::Motion::read(&dir)?;
        let speed = clip.speed.max(0.0625);
        let correction = concat_vision::stabilize::path::correct(
            &motion,
            (clip.source_start, clip.source_start + clip.duration * speed),
            speed,
            &concat_vision::stabilize::Settings {
                smoothness: settings.smoothness,
                max_zoom: settings.max_zoom,
                tripod: settings.tripod,
                keep_rotation: settings.keep_rotation,
            },
        );
        (!correction.is_empty()).then_some((correction, motion.aspect))
    }

    // ── enhance ──
    //
    // The restoration model reads a clip's file frame by frame and writes
    // an enhanced copy into the project's cache; the clip is then pointed
    // at the copy, as one undo step, and shows it wherever it showed the
    // original. The window's part is to start the job, keep the readout
    // moving, and re-point the clip when the copy is whole. Before that,
    // it holds the section's Size and Strength and runs a preview of the
    // frame under the playhead, so the person can judge both - and see
    // what the whole run will cost - before committing minutes to it.

    /// Writes one clip's sound as it plays - trimmed, at its speed, with its
    /// level, fades and effects baked in - to a WAV in the project's audio
    /// folder, and puts the file in the bin under Generated › Processed. A
    /// copy, not a replacement: the clip keeps its media and its settings,
    /// and the file is independent of both, so it can be cut, moved and
    /// exported like any import.
    ///
    /// The export's own audio path does the work, on the clip alone: the
    /// project is flattened with just this clip on its timeline, the one
    /// export clip that comes out is moved to the origin, and the mixer
    /// writes it the way it writes an export's soundtrack.
    /// The clip's menu: its sound as it plays - trimmed, at its speed, with
    /// its level, fades and effects baked in - written to a WAV the user
    /// names, and brought into the bin as a file of its own, selected,
    /// with the bin showing all its media so the new file is in view.
    ///
    /// A plain import, not a Generated › Processed one: the user picked
    /// where the file goes and what it is called, which makes it theirs
    /// the way a file dragged in is. On a phone, which has no save dialog,
    /// it goes in the project's audio folder under the clip's name.
    pub fn export_clip_audio(&mut self, id: &str) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if !self.clip_has_sound(&clip) {
            self.notify(&t("studio.clipHasNoSound"), true);
            return;
        }
        let Some(project_dir) = self
            .session
            .as_ref()
            .map(|session| std::path::PathBuf::from(session.path()))
        else {
            return;
        };
        let mut alone = self.project().clone();
        alone.active_mut().clips.retain(|held| held.id == clip.id);
        let Some(mut piece) =
            concat_export::flatten::flatten_timeline_in(&alone, None, Some(&project_dir))
                .into_iter()
                .next()
        else {
            self.notify(&t("studio.clipHasNoSound"), true);
            return;
        };
        piece.start = 0.0;
        piece.muted = false;
        let duration = piece.duration;
        let pieces = concat_export::audio_pieces(&piece);

        // Where it goes: the user's choice, offered under the clip's name
        // in the project's audio folder; a phone takes the offer.
        let out_dir = project_dir.join("audio");
        let slug: String = clip
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' {
                    c
                } else {
                    '-'
                }
            })
            .take(60)
            .collect();
        let suggested = format!("{}.wav", slug.trim());
        // The dialog on the next turn of the event loop, never from here:
        // this is reached from a right-click menu's row, which reports
        // through a timer, and a blocking dialog opened inside a timer's
        // callback spins the loop back into Slint's timers - "Recursion in
        // timer code", and the app aborts (on macOS; Windows and Linux
        // were spared only by their dialogs not pumping the loop). And
        // with the studio unborrowed, for the timers that borrow it; see
        // `host::after_dialog`.
        let name = clip.name.clone();
        let title = t("lib.exportAudio");
        let family = t("lib.wavAudio");
        let ask_in = out_dir.clone();
        crate::host::after_dialog(
            move || {
                if cfg!(any(target_os = "android", target_os = "ios")) {
                    return Some(ask_in.join(&suggested));
                }
                let chosen = crate::platform::save_file(
                    &title,
                    &ask_in,
                    &suggested,
                    (family.as_str(), &["wav"]),
                )?;
                Some(
                    if chosen
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
                    {
                        chosen
                    } else {
                        chosen.with_extension("wav")
                    },
                )
            },
            move |studio, _, _, file| {
                studio.write_clip_audio(&name, pieces, duration, file, out_dir);
            },
        );
    }

    /// Mixes `pieces` - one clip's sound, `duration` seconds of it - into
    /// `file`, and puts the file in the bin, selected. The second half of
    /// [`Studio::export_clip_audio`], after the person has said where.
    fn write_clip_audio(
        &mut self,
        name: &str,
        pieces: Vec<concat_media::audio::AudioClip>,
        duration: f64,
        file: std::path::PathBuf,
        out_dir: std::path::PathBuf,
    ) {
        let out_dir = file
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or(out_dir);
        let written = file.to_string_lossy().into_owned();
        log::info!(
            "export audio: {name} ({duration:.2}s, {} piece(s)) to {written}",
            pieces.len(),
        );
        self.notify(&t("studio.exportingAudio"), false);
        spawn_in_project(
            move || -> Result<concat_host::media::MediaSummary, String> {
                std::fs::create_dir_all(&out_dir)
                    .map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
                concat_media::audio::mix_to_file(&pieces, duration, &file)
                    .map_err(|error| error.to_string())?;
                concat_host::media::probe(&file.to_string_lossy())
            },
            move |studio, _, _, result| match result {
                Ok(summary) => {
                    let item = summary.to_new_media();
                    let path = item.path.clone();
                    let minted = studio.apply(Command::AddMedia { item }).or_else(|| {
                        studio
                            .project()
                            .media
                            .iter()
                            .find(|item| item.path == path)
                            .map(|item| item.id.clone())
                    });
                    // In view and in hand: the bin on all its media, with
                    // the new file the one thing selected.
                    studio.media.filter = MediaFilter::All;
                    studio.media.selected.clear();
                    if let Some(id) = minted {
                        studio.media.selected.insert(id);
                    }
                    studio.media_jump += 1;
                    studio.notify(&tf("studio.audioExported", &[&written]), false);
                }
                Err(error) => {
                    log::warn!("export audio: {error}");
                    studio.notify(&tf("studio.couldNotExportAudio", &[&error]), true);
                }
            },
        );
    }

    /// The Enhance tick in the inspector, for the one selected clip. It
    /// never starts a run - that is minutes of the machine, and the
    /// button's to ask for. Ticked, the section's settings and button come
    /// alive. On a clip with a copy it also switches the clip between the
    /// copy and its original, one undo step, the copy kept in the bin and
    /// the cache so ticking again shows it at once.
    ///
    /// Unticked means the clip shows its original, so a run still going
    /// for it is stopped: left alone, it would put the copy on screen when
    /// it finished, under a tick that says it is off.
    pub fn clip_enhance_set(&mut self, on: bool) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(clip) = self.clip(&id) else {
            return;
        };
        if self.locked(&clip.track_id) {
            return;
        }
        let linked = clip.enhanced.is_some();
        if !on && self.enhance_jobs.contains_key(&id) {
            self.host.enhancers.cancel();
        }
        if on {
            self.enhance_ticked.insert(id.clone());
        } else {
            self.enhance_ticked.remove(&id);
        }
        if linked {
            self.apply(Command::SetClipEnhanced { clip_id: id, on });
            self.request_preview();
        }
    }

    /// Whether clip `clip`'s Enhance section is ticked: its copy shown, or,
    /// without a copy, the tick put on in this window.
    pub(super) fn enhance_ticked(&self, clip: &Clip) -> bool {
        match clip.enhanced.as_ref() {
            Some(link) => link.on,
            None => self.enhance_ticked.contains(&clip.id),
        }
    }

    /// The Scale row: zero or less is Auto, otherwise times larger. One
    /// carried here from a smaller clip is held to what this one may take;
    /// see `concat_vision::enhance::scale_of`.
    pub fn clip_enhance_scale(&mut self, scale: f32) {
        self.enhance_scale = if scale > 0.0 {
            scale.clamp(1.0, 4.0)
        } else {
            0.0
        };
    }

    /// The Strength row, 0 to 1: how much of the model's answer the copy
    /// keeps. A preview on screen is re-blended at once, which is the
    /// point of keeping both its halves.
    pub fn clip_enhance_strength(&mut self, strength: f32) {
        self.enhance_strength = strength.clamp(0.0, 1.0);
        let strength = self.enhance_strength;
        if let Some(preview) = self.enhance_preview.as_mut() {
            preview.blend_at(strength);
        }
    }

    /// The section's one button: Enhance, or Enhance again on a clip that
    /// has a copy - a run from the original with the settings as they are
    /// now, a copy those settings already made picked up at once rather
    /// than redone - and Terminate while this clip's run is going. Only on
    /// a ticked section, as the button is.
    pub fn clip_enhance_run(&mut self) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        // Terminate: the run under way for this clip stops after the frame
        // in hand, and its partial files go; the clip is re-pointed only on
        // a copy that is whole, so it stays as it was.
        if self.enhance_jobs.contains_key(&id) {
            self.host.enhancers.cancel();
            return;
        }
        if self
            .clip(&id)
            .is_some_and(|clip| !self.locked(&clip.track_id) && self.enhance_ticked(clip))
        {
            self.enhance_clip(&id);
        }
    }

    /// The file Enhance reads for `clip`: its original when it is showing,
    /// or holding, a copy already - a second run restores the source, not
    /// the first run's answer.
    pub(super) fn enhance_source(&self, clip: &Clip) -> Option<model::MediaItem> {
        let id = clip
            .enhanced
            .as_ref()
            .map_or(&clip.media_id, |link| &link.original);
        self.project()
            .media
            .iter()
            .find(|item| &item.id == id)
            .cloned()
    }

    /// How many times larger `media` comes out at the section's Scale.
    pub(super) fn enhance_scale_of(&self, media: &model::MediaItem) -> f32 {
        concat_vision::enhance::scale_of(
            self.enhance_scale,
            media.width.unwrap_or(0),
            media.height.unwrap_or(0),
        )
    }

    /// "Preview this frame": the middle of the frame of the selected clip
    /// under the playhead, through the model at the section's Size, shown
    /// before and after in the inspector. Seconds rather than minutes - a
    /// few tiles, not a file - and nothing is written or re-pointed, so the
    /// clip is as it was whatever comes of it.
    pub fn clip_enhance_preview(&mut self) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        if self.enhance_previewing.is_some()
            || !self.enhance_jobs.is_empty()
            || self.host.enhancers.is_busy()
        {
            self.notify(&t("studio.enhanceAlreadyWorkOne"), true);
            return;
        }
        let Some(clip) = self.clip(&id).cloned() else {
            return;
        };
        if clip.kind != model::ClipKind::Video && clip.kind != model::ClipKind::Image {
            return;
        }
        let Some(media) = self.enhance_source(&clip) else {
            return;
        };
        if media.width.unwrap_or(0) == 0 || media.height.unwrap_or(0) == 0 {
            self.notify(&t("studio.fileHasNoPicture"), true);
            return;
        }
        let scale = self.enhance_scale_of(&media);
        // The moment under the playhead in the original's seconds, which is
        // the file the preview reads, the clip may be showing a copy of a
        // span that starts partway into it.
        let seconds = if clip.kind == model::ClipKind::Image {
            0.0
        } else {
            self.source_at_playhead(&clip) + Self::original_start(&clip) - clip.source_start
        };
        let request = concat_host::enhance::PreviewRequest {
            media_path: media.path.clone(),
            seconds,
            scale,
        };
        self.enhance_previewing = Some((id.clone(), false, 0.0));
        let enhancers = Arc::clone(&self.host.enhancers);
        let epoch = crate::host::project_epoch();
        let media_id = media.id.clone();
        spawn_in_project(
            move || {
                let mut last = -1.0f32;
                let result = enhancers.preview(&request, &mut |progress| {
                    // The model's download is the one slow part a preview
                    // can have; the tiles themselves take seconds.
                    if let concat_host::enhance::Progress::Fetching { received, total } = progress {
                        let fraction = received as f32 / total.max(1) as f32;
                        if fraction - last >= 0.01 {
                            last = fraction;
                            on_ui_in_project(epoch, move |studio, _, _| {
                                if let Some(held) = studio.enhance_previewing.as_mut() {
                                    held.1 = true;
                                    held.2 = fraction;
                                }
                            });
                        }
                    }
                });
                (id, media_id, scale, result)
            },
            |studio, _, _, (clip_id, media_id, scale, result)| {
                studio.enhance_previewing = None;
                match result {
                    Ok(preview) => {
                        let mut made = EnhancePreview {
                            clip_id,
                            media_id,
                            scale,
                            before: image_of(&preview.plain),
                            after: image_of(&preview.enhanced),
                            after_percent: 100,
                            plain: preview.plain,
                            enhanced: preview.enhanced,
                            tile_seconds: preview.tile_seconds,
                            tiles_per_frame: preview.tiles_per_frame,
                        };
                        made.blend_at(studio.enhance_strength);
                        studio.enhance_preview = Some(made);
                    }
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => studio.notify(&tf("studio.enhance", &[&error]), true),
                }
            },
        );
    }

    /// The preview on screen for `clip` read from `media` at `scale`, if
    /// the last one made is that.
    pub(super) fn enhance_preview_for(
        &self,
        clip: &Clip,
        media: &model::MediaItem,
        scale: f32,
    ) -> Option<&EnhancePreview> {
        self.enhance_preview.as_ref().filter(|preview| {
            preview.clip_id == clip.id
                && preview.media_id == media.id
                && (preview.scale - scale).abs() < 1e-4
        })
    }

    pub fn enhance_clip(&mut self, id: &str) {
        if !self.enhance_jobs.is_empty()
            || self.enhance_previewing.is_some()
            || self.host.enhancers.is_busy()
        {
            self.notify(&t("studio.enhanceAlreadyWorkOne"), true);
            return;
        }
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if clip.kind != model::ClipKind::Video && clip.kind != model::ClipKind::Image {
            return;
        }
        let Some(media) = self.enhance_source(&clip) else {
            return;
        };
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        let (width, height) = (media.width.unwrap_or(0), media.height.unwrap_or(0));
        if width == 0 || height == 0 {
            self.notify(&t("studio.fileHasNoPicture"), true);
            return;
        }
        let scale = self.enhance_scale_of(&media);
        let strength = self.enhance_strength;
        let still = clip.kind == model::ClipKind::Image;
        // The span the clip shows of the original, and a second either side
        // so a small trim after is still inside the copy: the work is the
        // clip's, not the whole file's.
        let span = (!still).then(|| {
            let start = Self::original_start(&clip);
            let covered = clip.duration * clip.speed;
            let from = (start - ENHANCE_MARGIN).max(0.0);
            let to = (start + covered + ENHANCE_MARGIN).min(media.duration.unwrap_or(f64::MAX));
            (from, (to - from).max(covered))
        });
        let Some(target) =
            concat_host::enhance::target_for(&project, &media.path, scale, strength, still, span)
        else {
            self.notify(&tf("studio.couldNotRead", &[&media.path]), true);
            return;
        };
        let request = EnhanceRequest {
            media_path: media.path.clone(),
            still,
            scale,
            strength,
            target,
            span,
        };
        let percent = concat_host::enhance::strength_percent(strength);
        let clip_id = clip.id.clone();
        self.enhance_jobs.insert(clip_id.clone(), (false, 0.0));
        self.notify(
            &tf(
                "studio.enhancingWayFrameFrame",
                &[&media.name, &times(scale)],
            ),
            false,
        );
        let enhancers = Arc::clone(&self.host.enhancers);
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                let mut last = (false, -1.0f32);
                let reporting = clip_id.clone();
                let result = enhancers.enhance(&request, &mut |progress| {
                    let now = match progress {
                        concat_host::enhance::Progress::Fetching { received, total } => {
                            (true, received as f32 / total.max(1) as f32)
                        }
                        concat_host::enhance::Progress::Analysing(fraction) => (false, fraction),
                    };
                    if now.0 != last.0 || now.1 - last.1 >= 0.01 {
                        last = now;
                        let id = reporting.clone();
                        on_ui_in_project(epoch, move |studio, _, _| {
                            if let Some(held) = studio.enhance_jobs.get_mut(&id) {
                                *held = now;
                            }
                        });
                    }
                });
                (clip_id, result)
            },
            move |studio, _, _, (clip_id, result)| {
                studio.enhance_jobs.remove(&clip_id);
                // A cancelled or failed run leaves the clip as it was: the
                // job writes under partial names and the clip is only
                // re-pointed here, on a copy that is whole. Nor is a clip
                // whose tick went off as the run finished: the copy stays
                // in the cache, and ticking and Enhance again find it.
                let ticked = studio
                    .clip(&clip_id)
                    .is_some_and(|clip| studio.enhance_ticked(clip));
                match result {
                    Ok(path) if ticked => {
                        studio.enhance_ticked.remove(&clip_id);
                        let offset = span.map_or(0.0, |(start, _)| start);
                        studio.adopt_enhanced(&clip_id, &path, scale, percent, offset);
                    }
                    Ok(_) => {}
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => studio.notify(&tf("studio.enhance", &[&error]), true),
                }
            },
        );
    }

    /// Writes the span clip `id` covers of its file backwards into the
    /// project's cache, and points the clip at the copy when it is there;
    /// see `concat_host::reverse`. The import is never touched.
    pub fn reverse_clip(&mut self, id: &str) {
        if !self.reverse_jobs.is_empty() || self.host.reversers.is_busy() {
            self.notify(&t("studio.reverseAlreadyWorkOne"), true);
            return;
        }
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        let audio_only = match clip.kind {
            model::ClipKind::Video => false,
            model::ClipKind::Audio => true,
            _ => return,
        };
        if self.locked(&clip.track_id) {
            return;
        }
        let Some(media) = self
            .project()
            .media
            .iter()
            .find(|item| item.id == clip.media_id)
            .cloned()
        else {
            return;
        };
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        // The span the clip covers of the file: its in-point, and its
        // length at its rate - the curve's mean when it has one, which is
        // what `speed` holds then.
        let start = clip.source_start;
        let covered = clip.duration * clip.speed;
        let Some(target) =
            concat_host::reverse::target_for(&project, &media.path, start, covered, audio_only)
        else {
            self.notify(&tf("studio.couldNotRead", &[&media.path]), true);
            return;
        };
        let request = concat_host::ReverseRequest {
            media_path: media.path.clone(),
            audio_only,
            start,
            duration: covered,
            target,
        };
        let clip_id = clip.id.clone();
        self.reverse_jobs.insert(clip_id.clone(), 0.0);
        self.notify(&tf("studio.reversing", &[&media.name]), false);
        let reversers = Arc::clone(&self.host.reversers);
        let epoch = crate::host::project_epoch();
        spawn_in_project(
            move || {
                let mut last = -1.0f32;
                let reporting = clip_id.clone();
                let result = reversers.reverse(&request, &mut |fraction| {
                    if fraction - last >= 0.01 {
                        last = fraction;
                        let id = reporting.clone();
                        on_ui_in_project(epoch, move |studio, _, _| {
                            if let Some(held) = studio.reverse_jobs.get_mut(&id) {
                                *held = fraction;
                            }
                        });
                    }
                });
                (clip_id, start, covered, result)
            },
            |studio, _, _, (clip_id, start, covered, result)| {
                studio.reverse_jobs.remove(&clip_id);
                match result {
                    Ok(path) => studio.adopt_reversed(&clip_id, &path, start, covered),
                    Err(error) if error.contains("cancelled") => {}
                    Err(error) => studio.notify(&tf("studio.reverse", &[&error]), true),
                }
            },
        );
    }

    /// Points clip `id` at the reversed copy at `path`, its in-point at
    /// the copy's own zero, and shows it - unless the clip has moved on
    /// its file meanwhile, in which case the copy covers a span the clip
    /// no longer shows, and the ask has to be made again.
    fn adopt_reversed(&mut self, id: &str, path: &std::path::Path, start: f64, covered: f64) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if (clip.source_start - start).abs() > 1e-6
            || (clip.duration * clip.speed - covered).abs() > 1e-6
        {
            self.notify(&t("studio.clipChangedWhileBeing"), true);
            return;
        }
        let summary = match media::probe(&path.to_string_lossy()) {
            Ok(summary) => summary,
            Err(error) => {
                self.notify(&tf("studio.reverse", &[&error]), true);
                return;
            }
        };
        let mut item = summary.to_new_media();
        if let Some(original) = self.project().media.iter().find(|m| m.id == clip.media_id) {
            item.name = format!("{} (reversed)", original.name);
        }
        item.origin = Some(model::MediaOrigin::Processed);
        self.apply(Command::ReplaceClipMedia {
            clip_id: id.to_owned(),
            item,
            source_start: Some(0.0),
            enhanced: false,
            offset: 0.0,
        });
        self.request_preview();
        self.notify(&t("studio.reversedClipNowShows"), false);
    }

    /// Clip `clip`'s in-point in its original's seconds: its own, or, while
    /// it shows an enhanced copy of a span, its own moved by where that
    /// copy starts.
    fn original_start(clip: &Clip) -> f64 {
        match &clip.enhanced {
            Some(link) if link.on => clip.source_start + link.offset,
            _ => clip.source_start,
        }
    }

    /// Points clip `id` at the enhanced copy at `path`, probed the way an
    /// import is, and shows it.
    fn adopt_enhanced(
        &mut self,
        id: &str,
        path: &std::path::Path,
        scale: f32,
        percent: u32,
        offset: f64,
    ) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        // The clip's in-point in the copy, which starts `offset` into the
        // original; a clip moved off the copy's span while it was made is
        // left as it is rather than pointed past the copy's ends.
        let start = Self::original_start(&clip) - offset;
        if start < -1e-6 {
            self.notify(&t("studio.clipChangedWhileBeing"), true);
            return;
        }
        let summary = match media::probe(&path.to_string_lossy()) {
            Ok(summary) => summary,
            Err(error) => {
                self.notify(&tf("studio.enhance", &[&error]), true);
                return;
            }
        };
        let mut item = summary.to_new_media();
        // The bin names the copy after the original, marked with what it
        // was made with - one file can have a copy for each setting -
        // rather than after its cache file.
        if let Some(original) = self.clip(id).and_then(|clip| self.enhance_source(clip)) {
            item.name = format!("{} (enhanced {}x, {percent}%)", original.name, times(scale));
        }
        if let Some(length) = item.duration
            && start + clip.duration * clip.speed > length + 1e-3
        {
            self.notify(&t("studio.clipChangedWhileBeing"), true);
            return;
        }
        self.apply(Command::ReplaceClipMedia {
            clip_id: id.to_owned(),
            item,
            source_start: Some(start.max(0.0)),
            enhanced: true,
            offset,
        });
        self.request_preview();
        self.notify(&t("studio.enhancedClipNowShows"), false);
    }

    /// The Mode row: 0 off, 1 automatic, 2 custom. The chroma rows are the
    /// inspector's own business, on the chain.
    pub fn cutout_mode(&mut self, mode: i32) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(clip) = self.clip(&id).cloned() else {
            return;
        };
        let held = clip.cutout.clone().unwrap_or_else(model::Cutout::auto);
        let cutout = match mode {
            1 => Some(model::Cutout {
                mode: model::CutoutMode::Auto,
                ..held
            }),
            2 => Some(model::Cutout {
                mode: model::CutoutMode::Custom,
                ..held
            }),
            _ => None,
        };
        if mode == 2 {
            self.painting = true;
        }
        if cutout != clip.cutout {
            self.apply(Command::SetClipCutout {
                clip_id: id,
                cutout,
            });
        }
    }

    /// The name an analysis runs under: the media and what it keeps.
    pub(super) fn analysis_key(media_id: &str, subject: model::Subject) -> String {
        format!("{media_id}:{}", subject.key())
    }

    /// The Subject row: 0 automatic, 1 person, 2 object. A change means
    /// other masks, which the analysis notices on its own.
    pub fn cutout_subject(&mut self, index: i32) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(cutout) = self.clip(&id).and_then(|clip| clip.cutout.clone()) else {
            return;
        };
        let subject = match index {
            1 => model::Subject::Person,
            2 => model::Subject::Object,
            _ => model::Subject::Auto,
        };
        if cutout.subject == subject {
            return;
        }
        self.apply(Command::SetClipCutout {
            clip_id: id,
            cutout: Some(model::Cutout { subject, ..cutout }),
        });
    }

    /// Takes every stroke off the selected clip's cutout, keeping it custom.
    pub fn cutout_clear(&mut self) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(cutout) = self.clip(&id).and_then(|clip| clip.cutout.clone()) else {
            return;
        };
        if cutout.strokes.is_empty() {
            return;
        }
        self.apply(Command::SetClipCutout {
            clip_id: id,
            cutout: Some(model::Cutout {
                strokes: Vec::new(),
                ..cutout
            }),
        });
    }

    pub fn cutout_tool(&mut self, index: i32) {
        self.brush = index.clamp(0, BRUSHES.len() as i32 - 1) as usize;
    }

    pub fn cutout_size(&mut self, size: f32) {
        self.brush_size = f64::from(size).clamp(model::MIN_BRUSH, model::MAX_BRUSH);
    }

    pub fn cutout_painting(&mut self, on: bool) {
        self.painting = on;
        if !on {
            self.pending_stroke = None;
        }
        // The monitor's view changes with the brushes: tinted while they
        // are out, cut when they are put away.
        self.request_preview();
    }

    /// The picture a press on the stage would paint: the one selected clip,
    /// under the playhead, with a custom cutout, while painting is on.
    pub(super) fn paint_target(&self) -> Option<Clip> {
        if !self.painting {
            return None;
        }
        let id = self.sole_selection()?;
        let clip = self.clip(&id)?;
        let custom = clip
            .cutout
            .as_ref()
            .is_some_and(|cutout| cutout.mode == model::CutoutMode::Custom);
        if !custom || !clip.kind.is_visual() || self.locked(&clip.track_id) {
            return None;
        }
        self.stage_clips()
            .into_iter()
            .any(|shown| shown.id == clip.id)
            .then(|| clip.clone())
    }

    /// Where a stage point lands in the source picture, in the fractions a
    /// stroke is stored in: the footprint's turn undone, then the crop and
    /// the flips, the same way a decoded pixel finds its mask, then a
    /// stabilized clip's correction at the playhead.
    pub(super) fn stage_to_source(&self, clip: &Clip, x: f64, y: f64) -> [f64; 2] {
        let footprint = self.footprint(clip);
        let (width, height) = self.output_size();
        let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
        let dx = (x - footprint.cx) * width;
        let dy = (y - footprint.cy) * height;
        let (sin, cos) = footprint.rotation.to_radians().sin_cos();
        let along = dx * cos + dy * sin;
        let down = -dx * sin + dy * cos;
        let px = along / (footprint.w * width).max(1e-6) + 0.5;
        let py = down / (footprint.h * height).max(1e-6) + 0.5;
        let mapping = concat_vision::Mapping {
            crop: clip
                .crop
                .map(|crop| {
                    [
                        crop.left as f32,
                        crop.top as f32,
                        crop.right as f32,
                        crop.bottom as f32,
                    ]
                })
                .unwrap_or([0.0; 4]),
            flip_h: clip.flip_h,
            flip_v: clip.flip_v,
        };
        let (u, v) = mapping.source_of(px as f32, py as f32);
        let (u, v) = (f64::from(u), f64::from(v));
        // A stabilized picture is drawn through its correction: the point
        // under the brush is where the correction reads the frame from.
        match self.stabilize_correction(clip) {
            Some((correction, aspect)) => {
                let map = correction.at(self.source_at_playhead(clip));
                let (x, y) = map.apply(((u - 0.5) * aspect, v - 0.5));
                [x / aspect + 0.5, y + 0.5]
            }
            None => [u, v],
        }
    }

    /// The stroke in flight as the stage draws it: path commands over a
    /// 1000 × 1000 viewbox, the line's width as a fraction of the stage,
    /// and whether it is taking away. Empty between strokes.
    pub(super) fn stroke_overlay(&self) -> (String, f32, bool) {
        let Gesture::Paint {
            clip,
            tool,
            size,
            screen,
            ..
        } = &self.gesture
        else {
            return self
                .pending_stroke
                .clone()
                .unwrap_or((String::new(), 0.0, false));
        };
        let Some((first, rest)) = screen.split_first() else {
            return (String::new(), 0.0, false);
        };
        let mut path = format!("M {:.1} {:.1}", first.0 * 1000.0, first.1 * 1000.0);
        // A single press still draws a dot: a line to where it already is,
        // with round caps.
        for (x, y) in rest.iter().chain(rest.is_empty().then_some(first)) {
            path.push_str(&format!(" L {:.1} {:.1}", x * 1000.0, y * 1000.0));
        }
        // The brush is `size` of the picture's width; on the stage that is
        // `size` of the picture's footprint.
        let width = self
            .clip(clip)
            .map(|clip| self.footprint(clip).w)
            .unwrap_or(1.0) as f32
            * *size as f32;
        let erase = matches!(
            tool,
            model::BrushTool::Eraser | model::BrushTool::SmartEraser
        );
        (path, width, erase)
    }
}

/// A scale as a person reads it: "2", "1.5", "1.33" - the hundredths the
/// slider moves in, without the zeros after them.
pub(crate) fn times(scale: f32) -> String {
    let text = format!("{scale:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}
