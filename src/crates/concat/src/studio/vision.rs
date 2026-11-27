// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah
//! Cutouts, stabilization and Enhance.

use concat_core::Halt;
use concat_doc::model::{Cutout, CutoutMode, Media, MediaOrigin, Subject};

use super::*;

/// The narrowest and widest a cutout's brush may be, as shares of the
/// picture's width: a hair's line to half the picture.
pub(crate) const MIN_BRUSH: f64 = 0.005;
pub(crate) const MAX_BRUSH: f64 = 0.5;

/// How soft a new cutout's edge is, as a share of the picture's width.
const FIRST_FEATHER: f64 = 0.01;

/// A cutout as it is first turned on: the model's mask as it is, looking
/// for whatever the picture is of.
fn first_cutout() -> Cutout {
    Cutout {
        mode: CutoutMode::Auto,
        subject: Subject::Auto,
        feather: FIRST_FEATHER,
        strokes: Vec::new(),
    }
}

/// What a cutout keeps, as an analysis is named by it.
fn subject_key(subject: &Subject) -> &str {
    match subject {
        Subject::Auto => "auto",
        Subject::Person => "person",
        Subject::Object => "object",
        Subject::Unknown(raw) => raw.as_str(),
    }
}

/// The source an item covers, in seconds from its in-point: how much of
/// its file it plays, however fast.
fn covered_seconds(clip: &Item) -> f64 {
    doc_view::covered(clip).map_or(0.0, |(_, span)| doc_view::dur_secs(span).abs())
}

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
        let wanted: Vec<(String, AnalyseRequest)> = Cutouts::requests(self.doc(), &project)
            .into_iter()
            .map(|(media, request)| {
                (
                    Self::analysis_key(&media.to_string(), &request.subject),
                    request,
                )
            })
            .collect();
        // One the app died analysing last time is skipped, and said once:
        // starting it again by itself closed the app on every open.
        let mut next = None;
        for (id, request) in wanted {
            if Cutouts::outstanding(&request) == 0 {
                continue;
            }
            let dir =
                concat_vision::mask_dir(&request.project, &request.media_path, &request.subject);
            if self.failed_analyses.contains(&dir) {
                continue;
            }
            if Cutouts::tripped(&request) {
                if self.tripped_strokes.insert(dir) {
                    log::warn!(
                        "cutout: skipped Remove background on a file: Concat closed while analysing it last time, and would again"
                    );
                    self.notify(&t("studio.removeBackgroundSkipped"), true);
                }
                continue;
            }
            next = Some((id, request, dir));
            break;
        }
        let Some((id, request, dir)) = next else {
            return;
        };
        self.cutout_jobs.insert(id.clone(), (false, 0.0));
        let cutouts = Arc::clone(&self.host.cutouts);
        let epoch = crate::host::project_epoch();
        spawn_long_in_project(
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
                (id, dir, result)
            },
            |studio, _, _, (id, dir, result)| {
                studio.cutout_jobs.remove(&id);
                match result {
                    Ok(_) => {
                        // The monitor shows the cut, and whatever else is
                        // waiting gets its turn.
                        studio.request_preview();
                        studio.ensure_cutouts();
                    }
                    Err(Halt::Cancelled) => {}
                    Err(Halt::Failed(error)) => {
                        studio.failed_analyses.insert(dir);
                        studio.notify(&tf("studio.removeBackground", &[&error]), true);
                        // The next clip waiting has its turn.
                        studio.ensure_cutouts();
                    }
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
        for (media, request) in concat_host::Stabilizers::requests(self.doc(), &project) {
            let media_id = media.to_string();
            if concat_host::Stabilizers::outstanding(&request).is_empty() {
                continue;
            }
            let dir = concat_host::Stabilizers::dir(&request);
            if self.failed_analyses.contains(&dir) {
                continue;
            }
            if concat_host::Stabilizers::tripped(&request) {
                if self.stabilize_retry.remove(&media_id) {
                    concat_host::jobs::Tripwire::clear(&dir);
                } else {
                    // Skipped, and said once: starting it again by itself
                    // would close the app on every open.
                    if self.tripped_strokes.insert(dir) {
                        log::warn!(
                            "stabilize: skipped a file: Concat closed while measuring it last time, and would again"
                        );
                        self.notify(&t("studio.stabilizeSkipped"), true);
                    }
                    continue;
                }
            }
            next = Some((media_id, request, dir));
            break;
        }
        let Some((media_id, request, dir)) = next else {
            return;
        };
        self.stabilize_jobs.insert(media_id.clone(), 0.0);
        let stabilizers = Arc::clone(&self.host.stabilizers);
        let epoch = crate::host::project_epoch();
        spawn_long_in_project(
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
                (media_id, dir, result)
            },
            |studio, _, _, (media_id, dir, result)| {
                studio.stabilize_jobs.remove(&media_id);
                match result {
                    Ok(_) => {
                        // The kept plan was built before the measurement
                        // and would go on showing the clip as shot.
                        studio.host.monitor.forget_plan();
                        studio.request_preview();
                        studio.ensure_stabilizers();
                    }
                    Err(Halt::Cancelled) => {}
                    Err(Halt::Failed(error)) => {
                        studio.failed_analyses.insert(dir);
                        studio.notify(&tf("studio.stabilize", &[&error]), true);
                        studio.ensure_stabilizers();
                    }
                }
            },
        );
    }

    /// The zoom a stabilized clip is drawn at and the zoom it would need
    /// to steady every frame, once its media's movement is measured.
    pub(super) fn stabilize_readout(&self, clip: &Item) -> Option<(f64, f64)> {
        let (correction, _) = self.stabilize_correction(clip)?;
        Some((correction.zoom, correction.needed))
    }

    /// A stabilized clip's correction, worked out as the renderer works it
    /// out, with the picture's aspect it is measured in; `None` for a clip
    /// not stabilized or not measured yet.
    fn stabilize_correction(
        &self,
        clip: &Item,
    ) -> Option<(concat_vision::stabilize::Correction, f64)> {
        let settings = clip.picture.stabilize.as_ref()?;
        let session = self.session.as_ref()?;
        let media = doc_view::media_of(self.doc(), clip)?;
        let dir =
            concat_vision::stabilize::motion_dir(std::path::Path::new(session.path()), &media.path);
        let motion = concat_vision::stabilize::Motion::read(&dir)?;
        let speed = doc_view::speed(clip).abs().max(0.0625);
        let start = doc_view::in_point_s(clip);
        let correction = concat_vision::stabilize::path::correct(
            &motion,
            (start, start + covered_seconds(clip)),
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

    /// The clip's menu: its sound as it plays - trimmed, at its speed, with
    /// its level, fades and effects baked in - written to a WAV the user
    /// names, and brought into the bin as a file of its own, selected,
    /// with the bin showing all its media so the new file is in view.
    ///
    /// The export's own audio path does the work, on the clip alone: the
    /// sequence is flattened as if just this clip were on it, the one export
    /// clip that comes out is moved to the origin, and the mixer writes it
    /// the way it writes an export's soundtrack.
    ///
    /// A plain import, not a Generated › Processed one: the user picked
    /// where the file goes and what it is called, which makes it theirs
    /// the way a file dragged in is. On a phone, which has no save dialog,
    /// it goes in the project's audio folder under the clip's name.
    pub fn export_clip_audio(&mut self, id: &str) {
        let (Some(item_id), Some(clip)) = (doc_view::parse::<ItemId>(id), self.clip(id).cloned())
        else {
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
        let alone = doc_view::showing(self.doc()).map_or_else(Vec::new, |sequence| {
            concat_export::flatten::clips_of(
                self.doc(),
                sequence,
                Some(&[item_id]),
                Some(&project_dir),
            )
        });
        let Some(mut piece) = alone.into_iter().next() else {
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
        // The clip's name, which the log leaves out: it can be the
        // person's own words.
        _name: &str,
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
        let file_name = crate::studio::file_name_of(&written);
        log::info!(
            "export: a clip's sound to {file_name} · {}",
            concat_log::span(std::time::Duration::from_secs_f64(duration.max(0.0)))
        );
        self.notify(&t("studio.exportingAudio"), false);
        spawn_long_in_project(
            move || -> Result<concat_host::media::MediaSummary, String> {
                std::fs::create_dir_all(&out_dir)
                    .map_err(|error| format!("could not create {}: {error}", out_dir.display()))?;
                concat_media::audio::mix_to_file(&pieces, duration, &file)
                    .map_err(|error| error.to_string())?;
                concat_host::media::probe(&file.to_string_lossy())
            },
            move |studio, _, _, result| match result {
                Ok(summary) => {
                    let media = summary.to_new_media();
                    let path = media.path.clone();
                    let minted = studio
                        .apply(Command::AddMedia { media })
                        .and_then(|id| id.typed::<MediaId>())
                        .or_else(|| studio.media_at_path(&path))
                        .map(|id| id.to_string());
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
                    log::warn!(
                        "export: writing a clip's sound to {file_name} failed: {error}; nothing was added"
                    );
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
        let (Some(item), Some(clip)) = (doc_view::parse::<ItemId>(&id), self.clip(&id)) else {
            return;
        };
        if self.lane_locked(clip.lane) {
            return;
        }
        let linked = matches!(&clip.content, Content::Media(cut) if cut.enhanced.is_some());
        if !on && self.enhance_jobs.contains_key(&id) {
            self.host.enhancers.cancel();
        }
        if on {
            self.enhance_ticked.insert(id.clone());
        } else {
            self.enhance_ticked.remove(&id);
        }
        if linked {
            self.apply(Command::SetClipEnhanced { item, on });
            self.request_preview();
        }
    }

    /// Whether clip `clip`'s Enhance section is ticked: its copy shown, or,
    /// without a copy, the tick put on in this window.
    pub(super) fn enhance_ticked(&self, clip: &Item) -> bool {
        match &clip.content {
            Content::Media(cut) if cut.enhanced.is_some() => {
                cut.enhanced.as_ref().is_some_and(|enhanced| enhanced.on)
            }
            _ => doc_view::id_of(self.doc(), clip)
                .is_some_and(|id| self.enhance_ticked.contains(&id.to_string())),
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
            .is_some_and(|clip| !self.lane_locked(clip.lane) && self.enhance_ticked(clip))
        {
            self.enhance_clip(&id);
        }
    }

    /// The file Enhance reads for `clip`, with its id: always its original,
    /// since the cut names the original and an enhanced copy beside it, so
    /// a second run restores the source, not the first run's answer.
    pub(super) fn enhance_source(&self, clip: &Item) -> Option<(String, Media)> {
        let id = doc_view::media_id(clip)?;
        let media = doc_view::media(self.doc(), id)?.clone();
        Some((id.to_string(), media))
    }

    /// How many times larger `media` comes out at the section's Scale.
    pub(super) fn enhance_scale_of(&self, media: &Media) -> f32 {
        let (width, height) = media
            .size
            .map_or((0, 0), |size| (size.width.get(), size.height.get()));
        concat_vision::enhance::scale_of(self.enhance_scale, width, height)
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
        let kind = self.kind(&clip);
        if kind != Kind::Video && kind != Kind::Image {
            return;
        }
        let Some((media_id, media)) = self.enhance_source(&clip) else {
            return;
        };
        if media.size.is_none() {
            self.notify(&t("studio.fileHasNoPicture"), true);
            return;
        }
        let scale = self.enhance_scale_of(&media);
        // The moment under the playhead in the original's seconds, which
        // is the file the preview reads; a cut's times are always the
        // original's, whichever of the two it shows.
        let seconds = if kind == Kind::Image {
            0.0
        } else {
            self.source_at_playhead(&clip)
        };
        let request = concat_host::enhance::PreviewRequest {
            media_path: media.path.clone(),
            seconds,
            scale,
        };
        self.enhance_previewing = Some((id.clone(), false, 0.0));
        let enhancers = Arc::clone(&self.host.enhancers);
        let epoch = crate::host::project_epoch();
        spawn_long_in_project(
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
                    Err(Halt::Cancelled) => {}
                    Err(Halt::Failed(error)) => {
                        studio.notify(&tf("studio.enhance", &[&error]), true);
                    }
                }
            },
        );
    }

    /// The preview on screen for clip `clip_id` read from file `media_id`
    /// at `scale`, if the last one made is that.
    pub(super) fn enhance_preview_for(
        &self,
        clip_id: &str,
        media_id: &str,
        scale: f32,
    ) -> Option<&EnhancePreview> {
        self.enhance_preview.as_ref().filter(|preview| {
            preview.clip_id == clip_id
                && preview.media_id == media_id
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
        let kind = self.kind(&clip);
        if kind != Kind::Video && kind != Kind::Image {
            return;
        }
        let Some((_, media)) = self.enhance_source(&clip) else {
            return;
        };
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        if media.size.is_none() {
            self.notify(&t("studio.fileHasNoPicture"), true);
            return;
        }
        let scale = self.enhance_scale_of(&media);
        let strength = self.enhance_strength;
        let still = kind == Kind::Image;
        // The span the clip shows of the original, and a second either side
        // so a small trim after is still inside the copy: the work is the
        // clip's, not the whole file's.
        let span = (!still).then(|| {
            let start = doc_view::in_point_s(&clip);
            let covered = covered_seconds(&clip);
            let length = media.length.map_or(f64::MAX, doc_view::len_secs);
            let from = (start - ENHANCE_MARGIN).max(0.0);
            let to = (start + covered + ENHANCE_MARGIN).min(length);
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
        let clip_id = id.to_owned();
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
        spawn_long_in_project(
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
                    Err(Halt::Cancelled) => {}
                    Err(Halt::Failed(error)) => {
                        studio.notify(&tf("studio.enhance", &[&error]), true);
                    }
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
        let audio_only = match self.kind(&clip) {
            Kind::Video => false,
            Kind::Audio => true,
            _ => return,
        };
        if self.lane_locked(clip.lane) {
            return;
        }
        let Some(media) = doc_view::media_of(self.doc(), &clip).cloned() else {
            return;
        };
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = std::path::PathBuf::from(session.path());
        // The span the clip covers of the file: its in-point, and the
        // footage its length plays at its pace, a curve's included.
        let start = doc_view::in_point_s(&clip);
        let covered = covered_seconds(&clip);
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
        let clip_id = id.to_owned();
        self.reverse_jobs.insert(clip_id.clone(), 0.0);
        self.notify(&tf("studio.reversing", &[&media.name]), false);
        let reversers = Arc::clone(&self.host.reversers);
        let epoch = crate::host::project_epoch();
        spawn_long_in_project(
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
                    Err(Halt::Cancelled) => {}
                    Err(Halt::Failed(error)) => {
                        studio.notify(&tf("studio.reverse", &[&error]), true);
                    }
                }
            },
        );
    }

    /// Points clip `id` at the reversed copy at `path`, its in-point at
    /// the copy's own zero, and shows it - unless the clip has moved on
    /// its file meanwhile, in which case the copy covers a span the clip
    /// no longer shows, and the ask has to be made again.
    fn adopt_reversed(&mut self, id: &str, path: &std::path::Path, start: f64, covered: f64) {
        let (Some(item), Some(clip)) = (doc_view::parse::<ItemId>(id), self.clip(id).cloned())
        else {
            return;
        };
        if (doc_view::in_point_s(&clip) - start).abs() > 1e-6
            || (covered_seconds(&clip) - covered).abs() > 1e-6
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
        let mut file = summary.to_new_media();
        if let Some(original) = doc_view::media_of(self.doc(), &clip) {
            file.name = format!("{} (reversed)", original.name);
        }
        file.origin = Some(MediaOrigin::Processed);
        self.apply(Command::ReplaceClipMedia {
            item,
            file,
            enhanced: false,
            copy_start: TickDur::ZERO,
            source_start: Some(Tick::ZERO),
        });
        self.request_preview();
        self.notify(&t("studio.reversedClipNowShows"), false);
    }

    /// Points clip `id` at the enhanced copy at `path`, probed the way an
    /// import is, and shows it. The copy starts `offset` seconds into the
    /// original, and the clip's times stay the original's.
    fn adopt_enhanced(
        &mut self,
        id: &str,
        path: &std::path::Path,
        scale: f32,
        percent: u32,
        offset: f64,
    ) {
        let (Some(item), Some(clip)) = (doc_view::parse::<ItemId>(id), self.clip(id).cloned())
        else {
            return;
        };
        // The clip's in-point in the copy; a clip moved off the copy's span
        // while it was made is left as it is rather than pointed past the
        // copy's ends.
        let start = doc_view::in_point_s(&clip) - offset;
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
        let mut file = summary.to_new_media();
        // The bin names the copy after the original, marked with what it
        // was made with - one file can have a copy for each setting -
        // rather than after its cache file.
        if let Some((_, original)) = self.enhance_source(&clip) {
            file.name = format!("{} (enhanced {}x, {percent}%)", original.name, times(scale));
        }
        if let Some(length) = file.length.map(doc_view::len_secs)
            && start + covered_seconds(&clip) > length + 1e-3
        {
            self.notify(&t("studio.clipChangedWhileBeing"), true);
            return;
        }
        self.apply(Command::ReplaceClipMedia {
            item,
            file,
            enhanced: true,
            copy_start: doc_view::dur(offset),
            source_start: None,
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
        let (Some(item), Some(clip)) = (doc_view::parse::<ItemId>(&id), self.clip(&id).cloned())
        else {
            return;
        };
        let held = clip.picture.cutout.clone().unwrap_or_else(first_cutout);
        let cutout = match mode {
            1 => Some(Cutout {
                mode: CutoutMode::Auto,
                ..held
            }),
            2 => Some(Cutout {
                mode: CutoutMode::Custom,
                ..held
            }),
            _ => None,
        };
        if mode == 2 {
            self.painting = true;
        }
        // Turned on anew: an analysis that failed is tried again, since
        // the person asked.
        if clip.picture.cutout.is_none() && cutout.is_some() {
            self.failed_analyses.clear();
        }
        if cutout != clip.picture.cutout {
            self.apply(Command::SetClipCutout { item, cutout });
        }
    }

    /// The name an analysis runs under: the media and what it keeps.
    pub(super) fn analysis_key(media_id: &str, subject: &Subject) -> String {
        format!("{media_id}:{}", subject_key(subject))
    }

    /// The Subject row: 0 automatic, 1 person, 2 object. A change means
    /// other masks, which the analysis notices on its own.
    pub fn cutout_subject(&mut self, index: i32) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(item) = doc_view::parse::<ItemId>(&id) else {
            return;
        };
        let Some(cutout) = self.clip(&id).and_then(|clip| clip.picture.cutout.clone()) else {
            return;
        };
        let subject = match index {
            1 => Subject::Person,
            2 => Subject::Object,
            _ => Subject::Auto,
        };
        if cutout.subject == subject {
            return;
        }
        self.apply(Command::SetClipCutout {
            item,
            cutout: Some(Cutout { subject, ..cutout }),
        });
    }

    /// Takes every stroke off the selected clip's cutout, keeping it custom.
    pub fn cutout_clear(&mut self) {
        let Some(id) = self.sole_selection() else {
            return;
        };
        let Some(item) = doc_view::parse::<ItemId>(&id) else {
            return;
        };
        let Some(cutout) = self.clip(&id).and_then(|clip| clip.picture.cutout.clone()) else {
            return;
        };
        if cutout.strokes.is_empty() {
            return;
        }
        self.apply(Command::SetClipCutout {
            item,
            cutout: Some(Cutout {
                strokes: Vec::new(),
                ..cutout
            }),
        });
    }

    pub fn cutout_tool(&mut self, index: i32) {
        self.brush = index.clamp(0, BRUSHES.len() as i32 - 1) as usize;
    }

    pub fn cutout_size(&mut self, size: f32) {
        self.brush_size = f64::from(size).clamp(MIN_BRUSH, MAX_BRUSH);
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
    pub(super) fn paint_target(&self) -> Option<&Item> {
        if !self.painting {
            return None;
        }
        let id = self.sole_selection()?;
        let clip = self.clip(&id)?;
        let custom = clip
            .picture
            .cutout
            .as_ref()
            .is_some_and(|cutout| cutout.mode == CutoutMode::Custom);
        if !custom || !self.kind(clip).is_visual() || self.lane_locked(clip.lane) {
            return None;
        }
        self.stage_clips()
            .into_iter()
            .any(|(shown, _)| shown == id)
            .then_some(clip)
    }

    /// Where a stage point lands in the source picture, in the fractions a
    /// stroke is stored in: the footprint's turn undone, then the crop and
    /// the flips, the same way a decoded pixel finds its mask, then a
    /// stabilized clip's correction at the playhead.
    pub(super) fn stage_to_source(&self, clip: &Item, x: f64, y: f64) -> [f64; 2] {
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
                .picture
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
            flip_h: clip.picture.flip_h,
            flip_v: clip.picture.flip_v,
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
