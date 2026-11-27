// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Reading the edit back out of the session, and changing it.

use super::*;

impl Studio {
    // ── reading the edit ──

    /// The document as the window should draw it: the echo while a
    /// gesture runs, the session's snapshot otherwise, and an empty one on
    /// the launch screen.
    pub fn doc(&self) -> &Doc {
        self.echo.as_ref().unwrap_or(&self.document)
    }

    /// The session's snapshot, never the echo: what a commit is measured
    /// against.
    pub fn committed(&self) -> &Doc {
        &self.document
    }

    /// Takes the session's snapshot again, after anything that changed it.
    pub(super) fn refresh_document(&mut self) {
        self.document = self
            .session
            .as_ref()
            .map_or_else(Document::default, |session| session.document());
    }

    /// The item an id the panels hold names, on the document drawn.
    pub fn clip(&self, id: &str) -> Option<&Item> {
        doc_view::item_by(self.doc(), id)
    }

    /// What an item is to the panels.
    pub fn kind(&self, item: &Item) -> Kind {
        doc_view::kind(self.doc(), item)
    }

    /// What the item under an id is; [`Kind::Other`] when it is gone.
    pub fn kind_by(&self, id: &str) -> Kind {
        self.clip(id).map_or(Kind::Other, |item| self.kind(item))
    }

    /// Every item of the sequence showing, each with its id as the panels
    /// hold it, lane by lane from the bottom.
    pub fn clips(&self) -> Vec<(String, &Item)> {
        doc_view::items(self.doc())
            .into_iter()
            .map(|(id, item)| (id.to_string(), item))
            .collect()
    }

    /// The sequence showing's frame rate; 30 with nothing open.
    pub fn frame_rate(&self) -> f32 {
        doc_view::rate(self.doc()) as f32
    }

    /// The sequence showing's output size.
    pub fn output_size(&self) -> (u32, u32) {
        doc_view::frame_size(self.doc())
    }

    /// The monitor's quality tier for the sequence showing: 0 full, 1 half,
    /// 2 quarter; see `MonitorPane::quality_of`.
    pub fn quality_of(&self) -> usize {
        self.monitor.quality_of(self.doc())
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
        let Some(lane) = self.row_track(row) else {
            return;
        };
        self.confirm = Some(Confirm::RemoveTrack { lane });
    }

    /// The confirmation sheet's answer: go ahead with what it asked about.
    pub fn confirm_accepted(&mut self) {
        match self.confirm.take() {
            Some(Confirm::RemoveTrack { lane }) => {
                self.apply(Command::RemoveTrack { lane });
            }
            Some(Confirm::ReplaceExport { .. }) => {
                self.handle(crate::panes::Msg::Export(
                    crate::panes::export::ExportMsg::Overwrite,
                ));
            }
            Some(Confirm::Restart) => {
                let project = self
                    .session
                    .as_ref()
                    .map(|session| std::path::PathBuf::from(session.path()));
                crate::panes::settings::restart(project);
            }
            Some(Confirm::StopServerJobs { .. }) => {
                self.handle(crate::panes::Msg::Settings(
                    crate::panes::settings::SettingsMsg::ServerApply { stop_jobs: true },
                ));
            }
            Some(Confirm::QuitWithoutSaving { .. }) => {
                crate::closing("by Quit without saving");
                log::warn!(
                    "close: {} could not be saved, and the person chose to leave without it; its unsaved edits are lost",
                    self.project_name
                );
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
            Confirm::RemoveTrack { lane } => {
                let clips = doc_view::on_lane(self.doc(), *lane).count();
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
                    ..Default::default()
                }
            }
            Confirm::ReplaceExport { path } => crate::ui::ConfirmData {
                open: true,
                title: t("export.replaceFileTitle").into(),
                message: tf("export.replaceFileMessage", &[path]).into(),
                action: t("export.replace").into(),
                ..Default::default()
            },
            Confirm::QuitWithoutSaving { error } => crate::ui::ConfirmData {
                open: true,
                title: t("close.unsavedTitle").into(),
                message: tf("close.unsavedMessage", &[error]).into(),
                action: t("close.quitWithoutSaving").into(),
                ..Default::default()
            },
            Confirm::Restart => crate::ui::ConfirmData {
                open: true,
                title: t("settings.restartTitle").into(),
                message: t("settings.restartMessage").into(),
                action: t("settings.restartNow").into(),
                cancel: t("settings.restartLater").into(),
                calm: true,
                glyph: crate::ui::Glyph::Reset,
            },
            Confirm::StopServerJobs { jobs } => crate::ui::ConfirmData {
                open: true,
                title: t("settings.stopJobsTitle").into(),
                message: match jobs {
                    1 => t("settings.stopJobsOne"),
                    many => tf("settings.stopJobsMany", &[many]),
                }
                .into(),
                action: t("settings.applyAndStop").into(),
                ..Default::default()
            },
        }
    }

    /// The lane on row `row` of the panel, rows counted from the top.
    pub fn row_track(&self, row: i32) -> Option<LaneId> {
        doc_view::lane_at_row(self.doc(), usize::try_from(row).ok()?)
    }

    /// The row of a lane; the top row for one not showing.
    pub fn row_of_lane(&self, lane: LaneId) -> i32 {
        doc_view::row_of(self.doc(), lane).map_or(0, |row| row as i32)
    }

    /// Whether a lane is locked on the panel.
    pub fn lane_locked(&self, lane: LaneId) -> bool {
        self.locked(&lane.to_string())
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
    pub(super) fn lane_height(&self, lane: LaneId) -> f32 {
        match self.lane_size(&lane.to_string()) {
            TrackSize::Small => LANE_SMALL,
            TrackSize::Medium => LANE_MEDIUM,
            TrackSize::Large => LANE_LARGE,
            TrackSize::Auto => {
                let tallest = doc_view::on_lane(self.doc(), lane)
                    .map(|(_, item)| match self.kind(item) {
                        Kind::Video | Kind::Image | Kind::Other => LANE_LARGE,
                        Kind::Audio => LANE_MEDIUM,
                        // A title is its name strip alone; a layer has no
                        // picture at all; a shape's name says what it is.
                        // None needs a body's height.
                        Kind::Text
                        | Kind::Layer
                        | Kind::SoundLayer
                        | Kind::Shape
                        | Kind::Transition => LANE_SMALL,
                    })
                    .fold(0.0_f32, f32::max);
                if tallest > 0.0 { tallest } else { LANE_MEDIUM }
            }
        }
    }

    /// Every lane's height, top-most first.
    pub fn lane_heights(&self) -> Vec<f32> {
        doc_view::lanes(self.doc())
            .into_iter()
            .rev()
            .map(|lane| self.lane_height(lane))
            .collect()
    }

    pub fn row_at(&self, y: f32) -> i32 {
        row_at(&self.lane_heights(), y)
    }

    /// Seconds the project runs to, for Fit and for the scroll floor.
    pub fn duration(&self) -> f32 {
        doc_view::secs(doc_view::duration(self.doc())) as f32
    }

    /// The edit point next to the playhead - the nearest clip start or end
    /// on any lane, the origin or the project's end - after it when
    /// `forward`, before it otherwise; the playhead itself when there is
    /// none that way. Counted in whole frames, so a playhead parked on a
    /// cut is on it and the next press goes on to the one beyond, however
    /// the seconds round. Up and Down, as Premiere and Resolve put them.
    pub fn edit_point(&self, forward: bool) -> f32 {
        let spans: Vec<(f64, f64)> = doc_view::items(self.doc())
            .into_iter()
            .map(|(_, item)| (doc_view::start_s(item), doc_view::len_s(item)))
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
        let moving: Vec<ItemId> = moving.iter().filter_map(|id| doc_view::parse(id)).collect();
        for (id, item) in doc_view::items(self.doc()) {
            if moving.contains(&id) {
                continue;
            }
            targets.push(doc_view::start_s(item) as f32);
            targets.push(doc_view::end_s(item) as f32);
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

    /// Where a scrub asked for `time` lands: on the timeline's start, a
    /// clip's edge, a shown beat or a key of the selected clip when one is
    /// within eight pixels, and at `time` when none is or the toolbar's
    /// playhead snap is off. Not the playhead itself, which [`Self::snapped`]
    /// also pulls towards: a playhead that snapped to where it is would
    /// never leave. Nor on a phone's lanes, which slide under a playhead
    /// held at their middle - a pull there would jerk the whole cut.
    pub fn playhead_snapped(&self, time: f32) -> f32 {
        if !self.prefs.playhead_snaps() || self.lanes.centred {
            return time;
        }
        let edges = doc_view::items(self.doc())
            .into_iter()
            .flat_map(|(_, item)| [doc_view::start_s(item) as f32, doc_view::end_s(item) as f32]);
        let beats = self.timeline_beats().into_iter().map(|beat| beat as f32);
        pulled(
            time,
            8.0 * self.lanes.seconds_per_pixel,
            std::iter::once(0.0)
                .chain(edges)
                .chain(beats)
                .chain(self.key_marks()),
        )
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
        let exclude: Option<ItemId> = doc_view::parse(exclude);
        for (id, item) in doc_view::items(self.doc()) {
            if Some(id) == exclude {
                continue;
            }
            consider(doc_view::start_s(item) as f32);
            consider(doc_view::end_s(item) as f32);
        }
        // The song's beats, so a cut lands on the beat.
        for beat in self.timeline_beats() {
            consider(beat as f32);
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

    pub fn apply(&mut self, command: Command) -> Option<AnyId> {
        // An edit ends whatever burst the view was in.
        self.end_burst();
        let words = command_words(&command);
        Asked::log(format_args!("edit: {words}"));
        self.flush_commit();
        // Anything but an inspector commit ends the coalescing window; the
        // commit path sets `last_commit` again right after calling here.
        self.last_commit = None;
        self.apply_as(None, command, words)
    }

    /// [`Studio::apply`] as one move of an inspector gesture: the same
    /// bookkeeping, but the session folds it into the gesture's undo step
    /// and the coalescing window stays open for the next move.
    pub(super) fn apply_within(&mut self, gesture: &str, command: Command) -> Option<AnyId> {
        // A knob's gesture, folded into one step; said at Debug, since a
        // dial turned is many of these.
        let words = command_words(&command);
        log::debug!("edit: {words} (part of one gesture)");
        self.apply_as(Some(gesture), command, words)
    }

    /// The command made on the session, as a step of its own or as a move
    /// of `gesture`. The echo, if any, is dropped either way, because the
    /// session's document is the truth again; a refusal becomes a notice.
    /// The selection the edit reports becomes the window's.
    fn apply_as(
        &mut self,
        gesture: Option<&str>,
        command: Command,
        words: String,
    ) -> Option<AnyId> {
        self.echo = None;
        let selection = self.selected_ids();
        let session = self.session.as_mut()?;
        session.set_selection(selection);
        let chosen = matches!(command, Command::SetTimelineVideo { .. });
        let before = colour_of_showing(&self.document);
        match session.apply_within(gesture, &command) {
            Ok(change) => {
                self.refresh_document();
                let after = colour_of_showing(&self.document);
                if let Some(selection) = &change.applied.selection {
                    self.selection = selection.iter().map(ItemId::to_string).collect();
                }
                self.after_change();
                if !chosen {
                    self.tell_of_hdr(&before, &after);
                }
                change.applied.created
            }
            Err(error) => {
                log::warn!("edit: {words} was asked for and refused: {error}; nothing changed");
                self.notify(&error, true);
                None
            }
        }
    }

    /// [`Studio::apply`], or [`Studio::apply_within`] when `gesture` names
    /// one: for the edits that are a step of their own from one caller and
    /// part of a larger step from another, as laying a copy is.
    pub(super) fn apply_in(&mut self, gesture: Option<&str>, command: Command) -> Option<AnyId> {
        match gesture {
            Some(gesture) => self.apply_within(gesture, command),
            None => self.apply(command),
        }
    }

    /// [`Studio::apply`] for an edit that makes an item, answering with its
    /// id as the panels hold it.
    pub(crate) fn apply_making(&mut self, command: Command) -> Option<String> {
        made_item(self.apply(command))
    }

    /// The selection as the document's ids, for the history to keep.
    pub(super) fn selected_ids(&self) -> Vec<ItemId> {
        self.selection
            .iter()
            .filter_map(|id| doc_view::parse(id))
            .collect()
    }

    /// Says so when an edit turned the sequence showing HDR, so the change
    /// is never a surprise, and says where it is set back.
    fn tell_of_hdr(&mut self, before: &model::ColorSpace, after: &model::ColorSpace) {
        if *before == model::ColorSpace::Sdr
            && matches!(after, model::ColorSpace::Hlg | model::ColorSpace::Pq)
        {
            let name = doc_view::sequence(self.doc())
                .map(|sequence| sequence.name.clone())
                .unwrap_or_default();
            self.notify(&tf("studio.timelineNowHdr", &[&name]), false);
        }
    }

    /// The bookkeeping every change to the edit needs: the caches that
    /// follow the document, the autosave, the monitor and the mix.
    fn after_change(&mut self) {
        self.dirty = true;
        self.revision += 1;
        self.assign_media_rows();
        let survivors: HashSet<String> = self.clips().into_iter().map(|(id, _)| id).collect();
        self.selection.retain(|id| survivors.contains(id));
        self.schedule_autosave();
        self.sync_audio();
        self.request_media_art();
        self.request_preview();
        self.ensure_cutouts();
        self.ensure_stabilizers();
        self.ensure_voices();
        self.ensure_regions();
    }

    pub fn undo(&mut self) {
        self.step_history(true);
    }

    pub fn redo(&mut self) {
        self.step_history(false);
    }

    /// Undo when `back`, redo otherwise: the step taken, and the selection
    /// that comes back with it.
    fn step_history(&mut self, back: bool) {
        let word = if back { "Undo" } else { "Redo" };
        self.flush_commit();
        self.echo = None;
        self.words = None;
        let selection = self.selected_ids();
        let change = self.session.as_mut().and_then(|session| {
            session.set_selection(selection);
            if back { session.undo() } else { session.redo() }
        });
        let Some(change) = change else {
            log::debug!("edit: {word} was asked for with nothing to take");
            return;
        };
        self.refresh_document();
        if let Some(selection) = &change.applied.selection {
            self.selection = selection.iter().map(ItemId::to_string).collect();
        }
        Asked::log(format_args!("edit: {word}"));
        self.after_change();
    }

    /// Starts a gesture's preview: the echo is a copy of the document the
    /// pointer will change, a pointer per table to make.
    pub fn begin_echo(&mut self) {
        if self.echo.is_none() {
            self.echo = Some(Doc::clone(self.doc()));
        }
    }

    /// Changes one item on the echo, as a gesture previews it. False when
    /// there is no echo or no such item.
    pub fn echo_edit(&mut self, id: &str, change: impl FnOnce(&mut Item)) -> bool {
        let (Some(echo), Some(id)) = (self.echo.as_mut(), doc_view::parse::<ItemId>(id)) else {
            return false;
        };
        echo.items.update(id, change).is_ok()
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
                    shell.studio.borrow().publish_status(&app);
                });
            },
        );
    }

    /// Writes the document, on the save lane. `announce` says so on
    /// success; a failure is always said.
    ///
    /// The snapshot is taken here, on the window's thread, with the edit
    /// that asked for it already applied; the lane only writes it. It has
    /// a thread of its own, so a save never waits behind an export or an
    /// analysis, and a save still waiting when the next is asked for is
    /// replaced by it - see `host::spawn_save`.
    pub fn save(&mut self, announce: bool) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        self.autosave.stop();
        // The write order is captured with the snapshot, not later when a
        // worker happens to pick it up.
        let pending = session.prepare_save(None);
        let path = pending.folder.to_string_lossy().into_owned();
        self.dirty = false;
        let name = self.project_name.clone();
        let began = std::time::Instant::now();
        crate::host::spawn_save(
            path,
            announce,
            move || concat_host::session::write_save(&pending),
            move |studio, _, _, result, announce| match result {
                Ok(_) if announce => {
                    log::info!(
                        "project: saved {name} in {}",
                        concat_log::span(began.elapsed())
                    );
                    studio.notify(&t("studio.projectSaved"), false);
                }
                Ok(_) => log::debug!(
                    "project: autosaved {name} in {}",
                    concat_log::span(began.elapsed())
                ),
                // The project stays marked as changed, so the next edit's
                // autosave and the next Save both try again.
                Err(error) if announce => {
                    log::error!(
                        "project: saving {name} failed: {error}; it stays marked unsaved and the next edit or Save tries again"
                    );
                    studio.dirty = true;
                    studio.notify(&tf("studio.couldNotSave", &[&error]), true);
                }
                Err(error) => {
                    log::warn!(
                        "project: autosaving {name} failed: {error}; it stays marked unsaved and the next edit or Save tries again"
                    );
                    studio.dirty = true;
                    studio.notify(&tf("studio.couldNotSave", &[&error]), true);
                }
            },
            |studio, app, _| studio.publish_status(app),
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

/// The nearest of `targets` to `time` that is closer than `threshold`, or
/// `time` itself when none is.
pub(crate) fn pulled(time: f32, threshold: f32, targets: impl IntoIterator<Item = f32>) -> f32 {
    let mut best = time;
    let mut best_distance = threshold;
    for target in targets {
        let distance = (target - time).abs();
        if distance < best_distance {
            best = target;
            best_distance = distance;
        }
    }
    best
}
