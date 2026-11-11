// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Projects: opening, saving, recents and templates.

use super::*;

impl Studio {
    // ── projects ──

    /// Opens a project as the session and leaves the launch screen, or
    /// says why it could not.
    pub fn open_project(&mut self, info: ProjectInfo) -> Result<(), String> {
        let name = info.name.clone();
        let began = std::time::Instant::now();
        let opened = self.open_project_timed(info);
        match &opened {
            Ok(()) => {
                if let Some(session) = &self.session {
                    let timeline = session.project().active();
                    let tracks = match timeline.tracks.len() {
                        1 => "1 track".to_owned(),
                        count => format!("{count} tracks"),
                    };
                    log::info!(
                        "project: opened {name} in {} · {tracks}, {}, {}",
                        concat_log::span(began.elapsed()),
                        clips_of(timeline.clips.len()),
                        concat_log::span(std::time::Duration::from_secs_f64(
                            timeline.duration().max(0.0)
                        )),
                    );
                }
            }
            Err(error) => {
                let left = if self.session.is_some() {
                    format!("{} stays open", self.project_name)
                } else {
                    "the launch screen stays up".to_owned()
                };
                log::error!("project: opening {name} failed: {error}; {left}");
            }
        }
        opened
    }

    /// The project in the folder at `path`, read and opened: what Open and
    /// Recents do. A folder that will not read says so in the log as
    /// [`Studio::open_project`] does for one that will not open.
    pub fn open_project_at(&mut self, path: &str) -> Result<(), String> {
        let info = projects::open(path).inspect_err(|error| {
            let left = if self.session.is_some() {
                format!("{} stays open", self.project_name)
            } else {
                "the launch screen stays up".to_owned()
            };
            log::error!(
                "project: reading {} failed: {error}; {left}",
                file_name_of(path)
            );
        })?;
        self.open_project(info)
    }

    /// [`Studio::open_project`], which says how it went.
    fn open_project_timed(&mut self, info: ProjectInfo) -> Result<(), String> {
        log::debug!("project: opening {}", info.path);
        // File › Open and Recents from a window with a project in it: that
        // project is closed first, as a close would - saved, its folder let
        // go, its jobs stopped. Swapping the session alone dropped what the
        // last autosave had not written yet and kept the old folder locked
        // against every other Concat for the rest of the run. A save that
        // fails keeps it open and the other one shut.
        if let Some(open) = self
            .session
            .as_ref()
            .map(|session| session.path().to_owned())
        {
            if same_folder(&open, &info.path) {
                log::debug!("project: {} is already open", info.name);
                return Ok(());
            }
            self.save_and_release()
                .map_err(|error| tf("studio.couldNotSave", &[&error]))?;
        }
        match self
            .host
            .open_projects
            .claim(&info.path, concat_api::Holder::Window)
        {
            Ok(()) => {}
            Err(concat_api::Refusal::Here(_)) => {
                return Err(t("studio.projectOpenThroughRemote"));
            }
            // Another process has it - a second window, an agent's
            // `concat --mcp`, the CLI - and two saving one folder lose
            // each other's work.
            Err(concat_api::Refusal::Elsewhere(owner)) => {
                let pid = owner
                    .pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "?".to_owned());
                log::debug!("project: {} is held by process {pid}", info.path);
                return Err(tf("studio.projectOpenElsewhere", &[&pid]));
            }
        }
        match Session::open_info(&info) {
            Ok(session) => {
                if let Err(error) = projects::remember(&self.host.dirs.config, &info) {
                    log::warn!(
                        "project: adding {} to Recents failed: {error}; it opens all the same",
                        info.name
                    );
                }
                self.pause();
                // The proxies of media no longer in the project, or read
                // as a range they no longer are, go now: nothing else
                // sweeps cache/proxy.
                let project_dir = std::path::Path::new(session.path());
                let kept: HashSet<std::path::PathBuf> = session
                    .project()
                    .media
                    .iter()
                    .filter_map(|item| {
                        concat_host::proxy::path_for(
                            project_dir,
                            &item.path,
                            item.color_range.map(concat_export::engine_range),
                            item.color_space.is_hdr(),
                        )
                    })
                    .collect();
                let swept = concat_host::proxy::sweep(project_dir, &kept);
                if swept > 0 {
                    log::debug!(
                        "project: {swept} stale proxies swept from {}",
                        project_dir.display()
                    );
                }
                self.session = Some(session);
                // The relink dialog belongs to the project that opened it;
                // this project's own check follows below.
                self.relink = Default::default();
                crate::host::next_project_epoch();
                self.forget_art();
                self.art_failed.clear();
                self.art_stamp = None;
                self.proxied.clear();
                self.echo = None;
                self.dirty = false;
                self.project_name = info.name.clone();
                self.export.name = projects::folder_name(&info.name);
                self.selection.clear();
                self.transition_selected = None;
                self.media.selected.clear();
                self.playhead = 0.0;
                self.handle(crate::panes::Msg::Timeline(
                    crate::panes::timeline::TimelineMsg::Reset,
                ));
                self.on_start = false;
                self.handle(crate::panes::Msg::Monitor(
                    crate::panes::monitor::MonitorMsg::Opened,
                ));
                self.recents = projects::recents(&self.host.dirs.config);
                self.host.monitor.clear();
                self.audition = None;
                self.revision += 1;
                self.flat = None;
                self.sync_audio();
                self.request_media_art();
                self.request_preview();
                self.ensure_cutouts();
                self.ensure_stabilizers();
                self.ensure_voices();
                self.ensure_regions();

                // Effects this build no longer has, named once as the project
                // opens: their links stay in the clips, marked not installed.
                let retired = self
                    .session
                    .as_ref()
                    .map(|session| retired_effects(session.project()))
                    .unwrap_or_default();
                if !retired.is_empty() {
                    self.notify(&tf("studio.retiredEffects", &[&retired.join(", ")]), false);
                }
                // Effects that open as the ones standing in for them: the
                // document changes when it is next saved, so say so.
                let upgraded = self
                    .session
                    .as_ref()
                    .map(|session| upgraded_effects(session.upgraded()))
                    .unwrap_or_default();
                if !upgraded.is_empty() {
                    self.notify(
                        &tf("studio.upgradedEffects", &[&upgraded.join(", ")]),
                        false,
                    );
                }
                // Clips that played backwards or moved by a preset in an
                // earlier version, which this one does not read: named,
                // with the way back where there is one.
                let retired = self
                    .session
                    .as_ref()
                    .map(|session| session.retired().clone())
                    .unwrap_or_default();
                if !retired.reversed.is_empty() {
                    self.notify(
                        &tf("studio.clipsNowForwards", &[&few(&retired.reversed)]),
                        false,
                    );
                }
                if !retired.animated.is_empty() {
                    self.notify(
                        &tf("studio.clipsLostAnimations", &[&few(&retired.animated)]),
                        false,
                    );
                }

                // Log missing media to file for debugging
                if let Some(session) = &self.session {
                    let missing = session.project().missing_media();
                    if !missing.is_empty() {
                        let log_path = std::path::Path::new(&info.path)
                            .join("cache")
                            .join("missing_media.log");
                        if let Ok(mut file) = std::fs::File::create(&log_path) {
                            use std::io::Write;
                            let _ = writeln!(file, "{} media files missing:", missing.len());
                            for m in &missing {
                                let _ = writeln!(file, "  - {} ({})", m.name, m.path);
                            }
                        }

                        self.handle(crate::panes::Msg::Relink(
                            crate::panes::relink::RelinkMsg::Show(missing),
                        ));
                    }
                }
                Ok(())
            }
            Err(error) => {
                self.host.open_projects.release(&info.path);
                Err(error)
            }
        }
    }

    /// Looks for media the open project can no longer find, and opens the
    /// relink dialog on whatever is missing that the user has not already
    /// dismissed.
    ///
    /// Runs when the window is brought back to the front - the moment a
    /// person returns from moving a file - since the check
    /// [`Studio::open_project`] makes cannot see a file that went away
    /// afterwards: the bin and the timeline keep the item, the preview
    /// only fails to read it, and nothing else would say why.
    ///
    /// Returns whether the dialog changed, so the caller publishes only
    /// when there is something new to show.
    pub fn check_missing_media(&mut self) -> bool {
        if self.on_start || self.session.is_none() {
            return false;
        }
        let missing = self
            .session
            .as_ref()
            .map(|session| session.project().missing_media())
            .unwrap_or_default();
        let before: Vec<String> = self
            .relink
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect();
        let was_open = self.relink.open;
        self.handle(crate::panes::Msg::Relink(
            crate::panes::relink::RelinkMsg::Recheck(missing),
        ));
        let after: Vec<String> = self
            .relink
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect();
        self.relink.open != was_open || before != after
    }

    /// Clears all cached artwork and waveforms from the current project.
    pub fn clear_project_cache(&mut self) {
        let Some(path) = self
            .session
            .as_ref()
            .map(|session| session.path().to_string())
        else {
            return;
        };
        match concat_host::projects::clear_cache(&path) {
            Ok(count) => self.notify(&format!("Cleared {count} cache files"), false),
            Err(error) => self.notify(&format!("Cache clear failed: {error}"), true),
        }
        self.request_media_art();
        self.request_preview();
    }

    /// Saves, then closes the session and returns to the launch screen.
    ///
    /// A save that fails returns its reason and stops short of the close:
    /// the session, its running jobs and the autosave timer are left as they
    /// were, so a caller that was about to hide the window or quit can keep
    /// it up rather than drop unsaved work on the floor. Not untouched, to be
    /// exact - the close flushes a half-typed commit and pauses playback
    /// before it saves, and a failed save leaves those two as done. What a
    /// failure does not reach is the teardown: the worker cancellations, the
    /// job lists, the autosave timer and the session itself.
    pub fn close_project(&mut self) -> Result<(), String> {
        self.save_and_release().inspect_err(|error| {
            self.notify(&tf("studio.couldNotSave", &[error]), true);
        })
    }

    /// [`Studio::close_project`] without the notice: the project saved, then
    /// let go, or left open when the save fails.
    fn save_and_release(&mut self) -> Result<(), String> {
        log::debug!("project: closing {}", self.project_name);
        // Words still being typed land in the save, not on the floor.
        self.flush_commit();
        self.pause();
        // Saved before the workers are cancelled: a close that cannot be
        // written is not a close, and the jobs it would have stopped are
        // not ours to throw away.
        if let Some(session) = self.session.as_mut() {
            let (path, document) = session.prepare_save(None);
            let ticket = projects::ticket();
            if let Err(error) = projects::save_ordered(&path, &document, ticket) {
                log::error!(
                    "project: saving {} to close it failed: {error}; it stays open",
                    self.project_name
                );
                return Err(error);
            }
        }
        let name = self.session.is_some().then(|| self.project_name.clone());
        self.release_project();
        if let Some(name) = name {
            log::info!("project: closed {name}");
        }
        Ok(())
    }

    /// Asks whether to quit though the project could not be saved: the
    /// window's ways out call it when `close_project` refused.
    pub fn offer_quit_without_saving(&mut self, error: String) {
        self.confirm = Some(Confirm::QuitWithoutSaving { error });
    }

    /// Lets the open project go, unsaved: what `close_project` does once
    /// the save has landed, and what quitting without saving does instead.
    pub(super) fn release_project(&mut self) {
        self.host.cutouts.cancel();
        self.host.stabilizers.cancel();
        self.host.enhancers.cancel();
        self.host.reversers.cancel();
        self.enhance_jobs.clear();
        self.enhance_preview = None;
        self.enhance_previewing = None;
        self.enhance_ticked.clear();
        self.reverse_jobs.clear();
        self.cancel_merge();
        self.cutout_jobs.clear();
        self.stabilize_jobs.clear();
        self.stabilize_retry.clear();
        self.cancel_voices();
        self.region_job = None;
        self.picking = None;
        self.autosave.stop();
        // The launch screen's card for this project is read again: the
        // edit just made may have moved its first frame and its length.
        // The old poster stays up until the new one replaces it.
        if let Some(session) = self.session.as_ref() {
            self.lengths.remove(session.path());
        }
        if let Some(session) = self.session.as_ref() {
            self.host.open_projects.release(session.path());
        }
        self.session = None;
        // Whatever a worker still brings back for this project is dropped
        // at delivery; the sheets that were waiting on one stop waiting.
        crate::host::next_project_epoch();
        self.forget_art();
        self.art_failed.clear();
        self.art_stamp = None;
        self.proxied.clear();
        self.captions.running = false;
        self.captions.pace = Default::default();
        self.speech.running = false;
        self.speech.pace = Default::default();
        // No dialog about files the project that just closed could not
        // find stays up over the launch screen.
        self.relink = Default::default();
        self.source = None;
        // A take running as the project closes stops, keeping its file;
        // the next project's speakers are not left silent.
        self.voiceover = Default::default();
        self.host.playback.set_muted(false);
        self.echo = None;
        self.dirty = false;
        self.selection.clear();
        self.transition_selected = None;
        self.gesture = Gesture::None;
        self.handle(crate::panes::Msg::Monitor(
            crate::panes::monitor::MonitorMsg::Closed,
        ));
        self.host.monitor.clear();
        self.audition = None;
        self.revision += 1;
        self.flat = None;
        self.host
            .playback
            .set_clips(std::path::PathBuf::new(), Vec::new());
        self.on_start = true;
        self.recents = projects::recents(&self.host.dirs.config);
    }

    /// Posters, and the running time, for the recents not yet looked at,
    /// read on a worker. A missing folder has neither to give.
    pub(super) fn request_posters(&mut self) {
        let wanted: Vec<String> = self
            .recents
            .iter()
            .filter(|recent| recent.present)
            .map(|recent| recent.info.path.clone())
            .filter(|path| !self.lengths.contains_key(path) && !self.posters_pending.contains(path))
            .collect();
        for path in wanted {
            self.posters_pending.insert(path.clone());
            spawn(
                move || {
                    let cached = media::poster_cache(&path);
                    // A project with no poster - nothing on it, or nothing
                    // but black - keeps the film mark; see `poster_frame`.
                    let made = media::poster_frame(&path).is_ok();
                    let length = media::timeline_seconds(&path);
                    (path, made.then_some(cached), length)
                },
                |studio, _, _, (path, cached, length)| {
                    studio.posters_pending.remove(&path);
                    if let Some(poster) = cached.and_then(|cached| image_at(&cached)) {
                        studio.posters.insert(path.clone(), poster);
                    }
                    studio.lengths.insert(path, length);
                },
            );
        }
    }

    /// Routes one message to its pane and applies it. The pane is taken out
    /// of the studio for the duration, so its `update` can be handed the
    /// rest of the window without borrowing itself twice; a pane never
    /// reads its own slot on the studio.
    pub fn handle(&mut self, msg: crate::panes::Msg) {
        // The activity trail: what the person did, for the log. A burst
        // - a wheel's zooms - is one line and a count; see `Burst`.
        if let Some(activity) = crate::panes::activity(&msg) {
            self.note_activity(activity);
        }
        match msg {
            crate::panes::Msg::Export(msg) => {
                let mut pane = std::mem::take(&mut self.export);
                pane.update(msg, self);
                self.export = pane;
            }
            crate::panes::Msg::Settings(msg) => {
                let mut pane = std::mem::take(&mut self.settings);
                pane.update(msg, self);
                self.settings = pane;
            }
            crate::panes::Msg::Captions(msg) => {
                let mut pane = std::mem::take(&mut self.captions);
                pane.update(msg, self);
                self.captions = pane;
            }
            crate::panes::Msg::Speech(msg) => {
                let mut pane = std::mem::take(&mut self.speech);
                pane.update(msg, self);
                self.speech = pane;
            }
            crate::panes::Msg::Voiceover(msg) => {
                let mut pane = std::mem::take(&mut self.voiceover);
                pane.update(msg, self);
                self.voiceover = pane;
            }
            crate::panes::Msg::Sounds(msg) => {
                let mut pane = std::mem::take(&mut self.sounds);
                pane.update(msg, self);
                self.sounds = pane;
            }
            crate::panes::Msg::Relink(msg) => {
                let mut pane = std::mem::take(&mut self.relink);
                pane.update(msg, self);
                self.relink = pane;
            }
            crate::panes::Msg::Project(msg) => {
                let mut pane = std::mem::take(&mut self.project_sheet);
                pane.update(msg, self);
                self.project_sheet = pane;
            }
            crate::panes::Msg::Start(msg) => {
                let mut pane = std::mem::take(&mut self.start);
                pane.update(msg, self);
                self.start = pane;
            }
            crate::panes::Msg::Media(msg) => {
                let mut pane = std::mem::take(&mut self.media);
                pane.update(msg, self);
                self.media = pane;
            }
            crate::panes::Msg::Monitor(msg) => {
                let mut pane = std::mem::take(&mut self.monitor);
                pane.update(msg, self);
                self.monitor = pane;
            }
            crate::panes::Msg::Timeline(msg) => {
                let mut pane = std::mem::take(&mut self.lanes);
                pane.update(msg, self);
                self.lanes = pane;
            }
        }
    }
}
