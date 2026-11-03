// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Reading the edit back out of the session, and changing it.

use super::*;

impl Studio {
    // ── reading the edit ──

    /// The project as the window should draw it: the echo while a gesture
    /// runs, the session's otherwise, and nothing at all on the launch screen.
    pub fn project(&self) -> &Project {
        self.echo
            .as_ref()
            .or_else(|| self.session.as_ref().map(|session| session.project()))
            .unwrap_or(&self.empty)
    }

    pub fn timeline(&self) -> &Timeline {
        self.project().active()
    }

    pub fn clip(&self, id: &str) -> Option<&Clip> {
        self.timeline().clip(id)
    }

    /// The active timeline's frame rate. With no project open, the empty
    /// project's one timeline answers, at the default.
    pub fn frame_rate(&self) -> f32 {
        self.project().active().video.rate() as f32
    }

    /// The active timeline's output size.
    pub fn output_size(&self) -> (u32, u32) {
        let video = self.project().active().video;
        (video.width, video.height)
    }

    /// The monitor's quality tier for the active timeline: 0 full, 1 half,
    /// 2 quarter; see `MonitorPane::quality_of`.
    pub fn quality_of(&self) -> usize {
        self.monitor.quality_of(self.project())
    }

    /// The track a row index names. Rows count from the top of the panel and
    /// the model stores lanes bottom-most first - the compositing order - so
    /// this is the one place the two orders meet.
    /// Asks before the track on lane `row` goes, empty or not: a track is
    /// a whole lane of the cut, and the button that drops it sits beside
    /// the ones that hide and mute it.
    /// Lets go of every picture and waveform made for the project's media,
    /// as a project closes or another opens: the bin's thumbnails, the
    /// filmstrips, the waveform pyramids and their drawn envelopes. They
    /// were never dropped, so each project opened in a session added its
    /// art to the last one's until the app quit.
    pub(super) fn forget_art(&mut self) {
        self.peaks.clear();
        self.strips.clear();
        self.windows.clear();
        self.art_pending.clear();
        self.window_pending.clear();
        self.waves.borrow_mut().clear();
        self.media.thumbs.clear();
    }

    pub fn ask_remove_track(&mut self, row: i32) {
        let Some(track_id) = self.row_track(row).map(|track| track.id.clone()) else {
            return;
        };
        self.confirm = Some(Confirm::RemoveTrack { track_id });
    }

    /// The confirmation sheet's answer: go ahead with what it asked about.
    pub fn confirm_accepted(&mut self) {
        match self.confirm.take() {
            Some(Confirm::RemoveTrack { track_id }) => {
                self.apply(Command::RemoveTrack { track_id });
            }
            Some(Confirm::ReplaceExport { .. }) => {
                self.handle(crate::panes::Msg::Export(
                    crate::panes::export::ExportMsg::Overwrite,
                ));
            }
            Some(Confirm::QuitWithoutSaving { .. }) => {
                log::warn!("close: quitting without saving, as asked");
                self.release_project();
                slint::quit_event_loop().ok();
            }
            None => {}
        }
    }

    /// The confirmation sheet, as Slint shows it.
    pub(super) fn confirm_data(&self) -> crate::ui::ConfirmData {
        let Some(confirm) = &self.confirm else {
            return crate::ui::ConfirmData::default();
        };
        match confirm {
            Confirm::RemoveTrack { track_id } => {
                let clips = self
                    .timeline()
                    .clips
                    .iter()
                    .filter(|clip| &clip.track_id == track_id)
                    .count();
                let message = match clips {
                    0 => t("timeline.deleteTrackEmpty"),
                    1 => t("timeline.deleteTrackOneClip"),
                    many => tf("timeline.deleteTrackClips", &[&many]),
                };
                crate::ui::ConfirmData {
                    open: true,
                    title: t("timeline.deleteTrackTitle").into(),
                    message: message.into(),
                    action: t("common.delete").into(),
                }
            }
            Confirm::ReplaceExport { path } => crate::ui::ConfirmData {
                open: true,
                title: t("export.replaceFileTitle").into(),
                message: tf("export.replaceFileMessage", &[path]).into(),
                action: t("export.replace").into(),
            },
            Confirm::QuitWithoutSaving { error } => crate::ui::ConfirmData {
                open: true,
                title: t("close.unsavedTitle").into(),
                message: tf("close.unsavedMessage", &[error]).into(),
                action: t("close.quitWithoutSaving").into(),
            },
        }
    }

    pub fn row_track(&self, row: i32) -> Option<&Track> {
        let tracks = &self.timeline().tracks;
        let count = tracks.len() as i32;
        if row < 0 || row >= count {
            return None;
        }
        tracks.get((count - 1 - row) as usize)
    }

    pub fn row_of(&self, track_id: &str) -> i32 {
        let tracks = &self.timeline().tracks;
        tracks
            .iter()
            .position(|track| track.id == track_id)
            .map(|index| tracks.len() as i32 - 1 - index as i32)
            .unwrap_or(0)
    }

    pub fn locked(&self, track_id: &str) -> bool {
        self.lanes
            .lane_view
            .get(track_id)
            .is_some_and(|view| view.locked)
    }

    pub(super) fn lane_size(&self, track_id: &str) -> TrackSize {
        self.lanes
            .lane_view
            .get(track_id)
            .map_or(TrackSize::Auto, |view| view.size)
    }

    /// How tall one lane is drawn: a lane takes the height of the tallest
    /// thing on it, without the lanes having to be typed. An empty one takes
    /// the middle size.
    pub(super) fn lane_height(&self, lane: &Track) -> f32 {
        match self.lane_size(&lane.id) {
            TrackSize::Small => LANE_SMALL,
            TrackSize::Medium => LANE_MEDIUM,
            TrackSize::Large => LANE_LARGE,
            TrackSize::Auto => {
                let tallest = self
                    .timeline()
                    .clips
                    .iter()
                    .filter(|clip| clip.track_id == lane.id)
                    .map(|clip| match clip.kind {
                        model::ClipKind::Video | model::ClipKind::Image => LANE_LARGE,
                        model::ClipKind::Audio => LANE_MEDIUM,
                        // A title is its name strip alone; a layer has no
                        // picture at all; a shape's name says what it is.
                        // None needs a body's height.
                        model::ClipKind::Text
                        | model::ClipKind::Layer
                        | model::ClipKind::SoundLayer
                        | model::ClipKind::Shape => LANE_SMALL,
                    })
                    .fold(0.0_f32, f32::max);
                if tallest > 0.0 { tallest } else { LANE_MEDIUM }
            }
        }
    }

    /// Every lane's height, top-most first.
    pub fn lane_heights(&self) -> Vec<f32> {
        self.timeline()
            .tracks
            .iter()
            .rev()
            .map(|lane| self.lane_height(lane))
            .collect()
    }

    pub fn row_at(&self, y: f32) -> i32 {
        row_at(&self.lane_heights(), y)
    }

    /// Seconds the project runs to, for Fit and for the scroll floor.
    pub fn duration(&self) -> f32 {
        self.timeline().clips.iter().fold(0.0_f64, |longest, clip| {
            longest.max(clip.start + clip.duration)
        }) as f32
    }

    /// The edit point next to the playhead - the nearest clip start or end
    /// on any lane, the origin or the project's end - after it when
    /// `forward`, before it otherwise; the playhead itself when there is
    /// none that way. Counted in whole frames, so a playhead parked on a
    /// cut is on it and the next press goes on to the one beyond, however
    /// the seconds round. Up and Down, as Premiere and Resolve put them.
    pub fn edit_point(&self, forward: bool) -> f32 {
        let spans: Vec<(f64, f64)> = self
            .timeline()
            .clips
            .iter()
            .map(|clip| (clip.start, clip.duration))
            .collect();
        edit_point(&spans, self.playhead, self.frame_rate(), forward)
    }

    /// Clip edges, the playhead and zero all pull, and the nearest inside
    /// the threshold wins.
    /// How far a moving set of clips should shift so the nearest of its
    /// edges - any clip's start or end - lands on a snap target: the
    /// origin, the playhead, or an edge of a clip that is not moving. Zero
    /// when nothing is in reach. A drag used to snap only the dragged
    /// clip's start, so an end never met the next clip (reported on
    /// Reddit), and a group could snap to where its own clips had been.
    pub fn snap_shift(&self, edges: &[f32], threshold: f32, moving: &[&str]) -> f32 {
        if !self.lanes.snap {
            return 0.0;
        }
        let mut targets = vec![0.0, self.playhead];
        for clip in &self.timeline().clips {
            if moving.contains(&clip.id.as_str()) {
                continue;
            }
            targets.push(clip.start as f32);
            targets.push((clip.start + clip.duration) as f32);
        }
        let mut best = 0.0;
        let mut best_distance = threshold;
        for &edge in edges {
            for &target in &targets {
                let distance = (target - edge).abs();
                if distance < best_distance {
                    best = target - edge;
                    best_distance = distance;
                }
            }
        }
        best
    }

    pub fn snapped(&self, time: f32, threshold: f32, exclude: &str) -> f32 {
        if !self.lanes.snap {
            return time;
        }
        let mut best = time;
        let mut best_distance = threshold;
        let mut consider = |target: f32| {
            let distance = (target - time).abs();
            if distance < best_distance {
                best = target;
                best_distance = distance;
            }
        };
        consider(0.0);
        consider(self.playhead);
        for clip in &self.timeline().clips {
            if clip.id == exclude {
                continue;
            }
            consider(clip.start as f32);
            consider((clip.start + clip.duration) as f32);
        }
        best
    }

    // ── changing the edit ──

    /// Applies one command to the session and reports the id it minted.
    /// A refusal becomes a notice; the echo, if any, is dropped either way,
    /// because the session's project is the truth again.
    /// Logs one line of the activity trail, folding a burst of one kind
    /// of message into its first line and, when the burst ends - a
    /// different message, an edit, or a pause - one line with the count.
    pub(super) fn note_activity(&mut self, activity: crate::panes::Activity) {
        use crate::panes::Burst;
        let now = std::time::Instant::now();
        let Some(key) = activity.burst else {
            self.end_burst();
            log::log!(activity.level, "{}", activity.line);
            return;
        };
        match &mut self.activity_burst {
            Some(burst) if burst.key == key && now.duration_since(burst.last) < Burst::GAP => {
                burst.count += 1;
                burst.line = activity.line;
                burst.last = now;
            }
            _ => {
                self.end_burst();
                log::log!(activity.level, "{}", activity.line);
                self.activity_burst = Some(Burst {
                    key,
                    line: activity.line,
                    count: 1,
                    began: now,
                    last: now,
                });
            }
        }
    }

    /// Ends the burst being folded, if one is, with its count.
    fn end_burst(&mut self) {
        if let Some(burst) = self.activity_burst.take() {
            burst.flush();
        }
    }

    pub fn apply(&mut self, command: Command) -> Option<String> {
        // An edit ends whatever burst the view was in.
        self.end_burst();
        log::info!("edit: {}", command_name(&command));
        self.flush_commit();
        self.echo = None;
        // Anything but an inspector commit ends the coalescing window; the
        // commit path sets `last_commit` again right after calling here.
        self.last_commit = None;
        let session = self.session.as_mut()?;
        let chosen = matches!(command, Command::SetTimelineVideo { .. });
        let before = session.video().color_space;
        match session.apply(command) {
            Ok(view) => {
                let after = session.video().color_space;
                self.after_change();
                if !chosen {
                    self.tell_of_hdr(before, after);
                }
                view.created_id
            }
            Err(error) => {
                self.notify(&error, true);
                None
            }
        }
    }

    /// [`Studio::apply`] as one move of an inspector gesture: the same
    /// bookkeeping, but the editor folds it into the gesture's undo step
    /// and the coalescing window stays open for the next move.
    pub(super) fn apply_within(&mut self, gesture: &str, command: Command) -> Option<String> {
        // A knob's gesture, folded into one step; said at Debug, since a
        // dial turned is many of these.
        log::debug!("edit: {} ({gesture})", command_name(&command));
        self.echo = None;
        let session = self.session.as_mut()?;
        let before = session.video().color_space;
        match session.apply_within(Some(gesture), command) {
            Ok(view) => {
                let after = session.video().color_space;
                self.after_change();
                self.tell_of_hdr(before, after);
                view.created_id
            }
            Err(error) => {
                self.notify(&error, true);
                None
            }
        }
    }

    /// [`Studio::apply`], or [`Studio::apply_within`] when `gesture` names
    /// one: for the edits that are a step of their own from one caller and
    /// part of a larger step from another, as laying a copy is.
    pub(super) fn apply_in(&mut self, gesture: Option<&str>, command: Command) -> Option<String> {
        match gesture {
            Some(gesture) => self.apply_within(gesture, command),
            None => self.apply(command),
        }
    }

    /// Says so when an edit turned the active timeline HDR - its first HDR
    /// clip, which the editor follows (see `Editor::follow_first_hdr`) - so
    /// the change is never a surprise, and says where it is set back.
    fn tell_of_hdr(&mut self, before: model::ColorSpace, after: model::ColorSpace) {
        if before.is_sdr() && after.is_hdr() {
            let name = self.timeline().name.clone();
            self.notify(&tf("studio.timelineNowHdr", &[&name]), false);
        }
    }

    /// The bookkeeping every change to the edit needs: the caches that
    /// follow the document, the autosave, the monitor and the mix.
    fn after_change(&mut self) {
        self.dirty = true;
        self.revision += 1;
        self.assign_media_rows();
        let survivors: HashSet<String> = self
            .timeline()
            .clips
            .iter()
            .map(|clip| clip.id.clone())
            .collect();
        self.selection.retain(|id| survivors.contains(id));
        self.schedule_autosave();
        self.sync_audio();
        self.request_media_art();
        self.request_preview();
        self.ensure_cutouts();
        self.ensure_stabilizers();
        self.ensure_regions();
    }

    pub fn undo(&mut self) {
        self.flush_commit();
        self.echo = None;
        self.words = None;
        if let Some(session) = self.session.as_mut()
            && session.can_undo()
        {
            session.undo();
            self.after_change();
        }
    }

    pub fn redo(&mut self) {
        self.flush_commit();
        self.echo = None;
        self.words = None;
        if let Some(session) = self.session.as_mut()
            && session.can_redo()
        {
            session.redo();
            self.after_change();
        }
    }

    /// Starts a gesture's preview: the echo is a clone of the project the
    /// pointer will mutate.
    pub fn begin_echo(&mut self) {
        if self.echo.is_none() {
            self.echo = Some(self.project().clone());
        }
    }

    pub fn echo_clip_mut(&mut self, id: &str) -> Option<&mut Clip> {
        self.echo.as_mut()?.active_mut().clip_mut(id)
    }

    fn schedule_autosave(&mut self) {
        // A second and a half after the last change, not after the first:
        // a drag that commits ten edits saves once.
        self.autosave.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_millis(1500),
            || {
                crate::host::Shell::with(|shell, app| {
                    shell.studio.borrow_mut().save(false);
                    shell.studio.borrow().publish(&app, &shell.models);
                });
            },
        );
    }

    /// Writes the document, on a worker. `announce` says so on success;
    /// a failure is always said.
    pub fn save(&mut self, announce: bool) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        self.autosave.stop();
        let (path, document) = session.prepare_save(None);
        // Capture the write order with the snapshot, not later when a
        // worker happens to pick it up.
        let ticket = projects::ticket();
        self.dirty = false;
        spawn_in_project(
            move || projects::save_ordered(&path, &document, ticket),
            move |studio, _, _, result| match result {
                Ok(_) if announce => studio.notify(&t("studio.projectSaved"), false),
                Ok(_) => {}
                Err(error) => {
                    studio.dirty = true;
                    studio.notify(&tf("studio.couldNotSave", &[&error]), true);
                }
            },
        );
    }

    /// The audible clip set, handed to playback whenever the edit changes.
    /// Not while a library file has the speakers: the timeline's sound is
    /// handed back when it lets them go.
    pub(crate) fn sync_audio(&mut self) {
        if self.source.is_some() {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let specs = audio_specs(&session.flattened_clips());
        self.host
            .playback
            .set_clips(std::path::PathBuf::from(session.path()), specs);
    }
}
