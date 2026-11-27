// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Notices, and publishing the state into Slint.

use super::*;

impl Studio {
    // ── notices ──

    /// Raises the bottom-right notice: done, or failed. Every caller is
    /// some other handler that has just done - or refused to do - the thing
    /// it is reporting, so this only records; the publish that handler was
    /// going to make anyway is what puts it on screen. A confirmation over a
    /// failure still up is dropped; see [`ToastState::raise`].
    pub fn notify(&mut self, message: &str, failed: bool) {
        let kind = if failed {
            ToastKind::Failed
        } else {
            ToastKind::Done
        };
        self.toast.raise(message, kind);
    }

    /// [`Studio::notify`] for something that worked, but not as well as it
    /// should: the work goes on, and the person may want the log.
    pub fn warn(&mut self, message: &str) {
        self.toast.raise(message, ToastKind::Warning);
    }

    // ── publishing ──

    pub fn publish(&self, app: &App, models: &Models) {
        // The tray's three menu labels and the tooltips' bare-letter
        // hints, spelled from the keymap so a rebind moves the words
        // with the key. An action left unbound shows an em dash rather
        // than an empty bracket: there is a key, there just is not one
        // now.
        let hint = |id: &str| {
            let shown = keymap::label(&self.prefs, id);
            if shown.is_empty() {
                "—".to_owned()
            } else {
                shown
            }
        };
        let editor = app.global::<Editor>();
        editor.set_keys_undo(hint("undo").into());
        editor.set_keys_redo(hint("redo").into());
        editor.set_keys_split(hint("split").into());
        editor.set_keys_split_tool(hint("split-tool").into());
        editor.set_keys_delete_left(hint("delete-left").into());
        editor.set_keys_delete_right(hint("delete-right").into());
        editor.set_keys_pan(hint("pan").into());
        editor.set_keys_preview_axis(hint("preview-axis").into());
        self.publish_lanes(app, models);
        self.publish_chrome(app, models);
        self.publish_dock(app, models);
    }

    /// The bin's cards and nothing else: what the scrub copies' progress
    /// needs redrawn while they are written, without the lanes and the
    /// rest of the window a whole publish would rebuild twice a second.
    pub fn publish_bin(&self, models: &Models) {
        super::sync(&models.media, self.media.rows(self));
    }

    /// The project's saved state and the toast, and nothing else: what a
    /// save changes. The autosave publishes only this - a whole publish a
    /// second and a half after every edit was all cost (#622).
    pub fn publish_status(&self, app: &App) {
        app.set_project_status(
            if self.dirty {
                "unsaved changes"
            } else {
                "saved"
            }
            .into(),
        );
        app.set_toast(ToastData {
            token: self.toast.token,
            message: self.toast.message.as_str().into(),
            kind: self.toast.kind,
            hold: self.toast.hold_ms,
        });
    }

    pub fn publish_dock(&self, _app: &App, models: &Models) {
        let out = self.dock_layout();
        sync(&models.seats, out.seats);
        sync(&models.dividers, out.dividers);
        // A timeline put back while something plays brings its meters
        // with it; one taken away stops them at their next tick.
        self.wake_meters();
    }

    /// [`Studio::publish_dock`] for a change to the arrangement, which is
    /// remembered once the panels settle.
    pub fn publish_dock_remembered(&self, app: &App, models: &Models) {
        self.publish_dock(app, models);
        self.remember_layout_soon();
    }

    /// Saves the layout a moment after the last change: a gutter drag or a
    /// window resize arrives as a stream, and the file is written once.
    pub fn remember_layout_soon(&self) {
        if !self.prefs.remembers_layout() || self.meters.phone {
            return;
        }
        self.layout_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_millis(600),
            || {
                crate::host::Shell::with(|shell, app| {
                    shell.studio.borrow_mut().save_layout(app.window());
                });
            },
        );
    }

    /// Writes the docks and the window's size into the settings, when they
    /// changed and Settings > General says to remember them. A phone's
    /// shell has no dock to keep.
    pub fn save_layout(&mut self, window: &slint::Window) {
        self.layout_timer.stop();
        if !self.prefs.remembers_layout() || self.meters.phone {
            return;
        }
        let (wide, compact) = if self.compact {
            (&self.dock_aside, &self.dock)
        } else {
            (&self.dock, &self.dock_aside)
        };
        let mut layout = self.prefs.layout.clone().unwrap_or_default();
        layout.wide = Some(wide.to_saved());
        layout.compact = Some(compact.to_saved());
        let maximized = crate::platform::is_maximized(window);
        layout.maximized = maximized;
        // A maximised, full-screen or minimised window's size is not the
        // one to come back to; the last ordinary size is kept instead.
        let size = window.size().to_logical(window.scale_factor());
        if !maximized && !window.is_fullscreen() && size.width >= 320.0 && size.height >= 240.0 {
            layout.width = Some(size.width);
            layout.height = Some(size.height);
        }
        if self.prefs.layout.as_ref() != Some(&layout) {
            self.prefs.layout = Some(layout);
            self.prefs.save(&self.host.dirs);
        }
    }

    /// The window is a phone's: the lanes keep the playhead at their middle
    /// and glide under it. See `platform::phone` and `TimelinePane::centred`.
    pub fn set_phone(&mut self, phone: bool) {
        // A phone's Filters and Transitions sheets open on every shelf at
        // once, under their All pill; a desk's strip starts on the first.
        if phone {
            self.library[0].group = -1;
            self.library[3].group = -1;
        }
        self.lanes.centred = phone;
        self.meters.phone = phone;
        let playhead = self.playhead;
        self.lanes.recentre(playhead);
    }

    /// Shows the compact dock, or the wide one, keeping whichever is put
    /// away whole; see `COMPACT_WIDTH`.
    pub fn set_compact(&mut self, compact: bool) {
        if compact != self.compact {
            std::mem::swap(&mut self.dock, &mut self.dock_aside);
            self.compact = compact;
            // Text to speech moves with the window: open in the wide dock,
            // it is the compact one's sheet, and a sheet open as the window
            // widens becomes its pane - or, closed meanwhile, its pane goes.
            if compact {
                self.speech_docked = self.dock.contains(crate::ui::PaneKind::Speech);
            } else {
                self.speech_to_dock();
            }
        }
    }

    /// Whether the monitor has the whole window: the full-screen preview,
    /// which only an open project has.
    pub fn preview_full(&self) -> bool {
        self.preview_full && !self.on_start
    }

    pub fn dock_layout(&self) -> DockLayout {
        let mut out = DockLayout::default();
        let (width, height) = self.workspace;
        // The full-screen preview: one seat, the monitor's, edge to edge,
        // whatever the arrangement is - which is left as it was, panes,
        // ratios and all, for when the preview gives the window back. A
        // monitor is seated even where the arrangement has none.
        if self.preview_full() {
            return crate::dock::alone(crate::ui::PaneKind::Preview, width, height);
        }
        if width > SEAT_GAP * 2.0 && height > SEAT_GAP * 2.0 {
            lay_out(
                &self.dock,
                (
                    SEAT_GAP,
                    SEAT_GAP,
                    width - 2.0 * SEAT_GAP,
                    height - 2.0 * SEAT_GAP,
                ),
                &mut out,
            );
        }
        out
    }

    pub fn split_extent(&self, index: usize) -> Option<f32> {
        self.dock_layout().extents.get(index).copied()
    }

    pub fn split_ratio(&self, index: usize) -> Option<f32> {
        match self.dock.at(&self.dock.split_path(index)?) {
            Dock::Split { ratio, .. } => Some(*ratio),
            _ => None,
        }
    }

    /// The monitor's picture alone: what a frame arriving during playback
    /// changes.
    pub fn publish_frame(&self, app: &App) {
        app.global::<Editor>()
            .set_preview_frame(self.monitor.image.clone());
    }

    /// The timeline and the readouts that follow it: what runs on every
    /// event of a scrub, a drag, a trim or a knob.
    pub fn publish_lanes(&self, app: &App, models: &Models) {
        let editor = app.global::<Editor>();
        let beats = self.beats_shown();
        editor.set_beats_shown(beats.shown);
        editor.set_beats_tempo(beats.tempo.into());
        editor.set_beats_note(beats.note.into());
        editor.set_beats_on(beats.on);
        editor.set_beats_marks(beats.marks);
        let doc = self.doc();
        sync(
            &models.tabs,
            doc.view
                .tabs
                .iter()
                .filter_map(|id| Some((*id, doc.sequences.get(*id)?)))
                .map(|(id, sequence)| TimelineTabData {
                    id: doc_view::text(id),
                    name: sequence.name.as_str().into(),
                })
                .collect(),
        );

        let mut top = 0.0;
        sync(
            &models.tracks,
            doc_view::lanes(doc)
                .into_iter()
                .rev()
                .filter_map(|id| Some((id, doc_view::lane(doc, id)?)))
                .map(|(id, lane)| {
                    let height = self.lane_height(id);
                    let key = id.to_string();
                    // Eye drops picture from the composite. Enable it when the
                    // lane has any picture, when it is empty, or when it is
                    // already hidden (so picture can be brought back). Dim
                    // only on a visible lane that holds only audio / sound
                    // layers (#833); the slot stays so header columns align.
                    let mut on = doc_view::on_lane(doc, id).peekable();
                    let empty = on.peek().is_none();
                    let pictured = on.any(|(_, clip)| {
                        !matches!(self.kind(clip), Kind::Audio | Kind::SoundLayer)
                    });
                    let shows_eye = !lane.visible || pictured || empty;
                    let row = TrackData {
                        id: key.as_str().into(),
                        visible: lane.visible,
                        muted: lane.muted,
                        locked: self.locked(&key),
                        size: self.lane_size(&key),
                        height,
                        top,
                        shows_eye,
                    };
                    top += height;
                    row
                })
                .collect(),
        );

        // Only the clips near the view: what the lanes show plus a screen
        // either side; see `TimelinePane::published_span`.
        let span = self.lanes.published_span();
        let picked = self
            .selected_transition()
            .is_some()
            .then(|| self.transition_selected.clone())
            .flatten();
        sync(
            &models.clips,
            self.clips()
                .into_iter()
                .filter(|(_, clip)| {
                    crate::panes::timeline::TimelinePane::shows(
                        span,
                        doc_view::start_s(clip) as f32,
                        doc_view::len_s(clip) as f32,
                    )
                })
                .map(|(id, clip)| {
                    let kind = self.kind(clip);
                    let wave = if matches!(kind, Kind::Audio | Kind::Video) {
                        self.wave(clip)
                    } else {
                        Default::default()
                    };
                    let marks = self.beat_marks(clip);
                    ClipData {
                        id: id.as_str().into(),
                        name: clip.name.as_str().into(),
                        kind: kind_of(doc, clip),
                        row: self.row_of_lane(clip.lane),
                        start: doc_view::start_s(clip) as f32,
                        duration: doc_view::len_s(clip) as f32,
                        selected: self.selection.contains(&id),
                        transition_selected: picked.as_deref() == Some(id.as_str()),
                        fx: doc_view::chain(&clip.effects)
                            .iter()
                            .any(|effect| effect.enabled),
                        transition_duration: clip
                            .cut_in
                            .as_ref()
                            .map_or(0.0, |cut| doc_view::len_secs(cut.len) as f32),
                        fade_in: doc_view::dur_secs(clip.sound.fade_in) as f32,
                        fade_out: doc_view::dur_secs(clip.sound.fade_out) as f32,
                        volume: clip.sound.volume.value as f32,
                        text_body: doc_view::title(clip)
                            .map(|text| text.content.as_str())
                            .unwrap_or_default()
                            .into(),
                        wave: wave.0,
                        wave_from: wave.1,
                        wave_span: wave.2,
                        beats: marks.0,
                        bars: marks.1,
                        has_audio: self.clip_has_sound(clip),
                        strip: self.strip_of(clip),
                    }
                })
                .collect(),
        );

        // What is under the playhead. The topmost picture wins, the way the
        // compositor stacks them: the lanes come bottom first, so the last
        // one found.
        let playhead = doc_view::tick(f64::from(self.playhead));
        let showing = doc_view::items(doc)
            .into_iter()
            .filter(|(_, clip)| {
                self.kind(clip).is_visual()
                    && clip.start <= playhead
                    && playhead < doc_view::end(clip)
            })
            .map(|(_, clip)| clip)
            .last();
        // A picture from the bin on the monitor is shown whatever is under
        // the timeline's playhead: in a new project there is nothing, and
        // its frame was decoded and then hidden (#464).
        let source = self
            .source
            .as_ref()
            .filter(|source| source.kind != model::MediaKind::Audio);
        editor.set_has_picture(source.is_some() || showing.is_some());
        editor.set_see_through(
            doc_view::sequence(doc).is_some_and(|sequence| sequence.frame.canvas.a < 255),
        );
        editor.set_preview_clip_name(
            source
                .map(|source| source.name.as_str())
                .or(showing.map(|clip| clip.name.as_str()))
                .unwrap_or_default()
                .into(),
        );
        editor.set_preview_duration(self.duration());
        editor.set_playhead_free(!self.prefs.playhead_stops_at_end);
        editor.set_playing(self.playing);
        self.publish_frame(app);
        sync(&models.stage, self.stage_items());
        sync(&models.guides, self.stage_guides.clone());
        let (path, width, erase) = self.stroke_overlay();
        editor.set_stroke_path(path.into());
        editor.set_stroke_width(width);
        editor.set_stroke_erase(erase);
        editor.set_brush_tool(self.brush as i32);
        editor.set_brush_size(self.brush_size as f32);
        editor.set_painting(self.painting);
        editor.set_picking_colour(self.picking.is_some());

        editor.set_drop(match &self.drop {
            Some(plan) => DropData {
                active: true,
                kind: plan.kind,
                label: plan.label.as_str().into(),
                start: plan.start,
                duration: plan.duration,
                row: plan.row,
            },
            None => DropData::default(),
        });

        editor.set_selected_clip(self.selected());
        editor.set_selected_transition(self.selected_transition_data());
        let (knobs, choices) = self.transition_knobs();
        sync(&models.transition_knobs, knobs);
        sync_with(&models.transition_choices, choices, same_transition_choice);
        // The background and Fit and Fill, on their own global: the Details
        // panel, the Video panel and the phone all read it.
        self.publish_backdrop(app);
        // Written only when it differs: a fresh model is a change to every
        // binding that reads it, and this one is read on every publish.
        let labels = self.audio_track_labels();
        let shown = editor.get_audio_tracks();
        let same = shown.row_count() == labels.len()
            && labels
                .iter()
                .enumerate()
                .all(|(row, label)| shown.row_data(row).as_ref() == Some(label));
        if !same {
            editor.set_audio_tracks(slint::ModelRc::new(VecModel::from(labels)));
        }
        // The keyframe cluster's state, on its own global: every keyable row
        // in the inspector reads it, and none of them is threaded to.
        let keys = app.global::<Keyframes>();
        // The Keyframes panel: a row holds a model of its keys, so rows are
        // compared by what those keys are, and an unchanged row is left be.
        sync_with(
            &models.key_editor_rows,
            self.key_editor_rows(),
            same_key_row,
        );
        // How far into the selected clip the playhead is, 0 to 1 of it.
        app.global::<KeyEditor>().set_at(
            self.sole_selection()
                .and_then(|id| self.clip(&id))
                .map_or(0.0, |clip| {
                    let at = doc_view::content_at(clip, doc_view::tick(f64::from(self.playhead)));
                    doc_view::share_of(clip, at) as f32
                }),
        );
        self.publish_speed_ramp(app, models);
        let rows = self.key_rows();
        keys.set_available(!rows.is_empty());
        sync(&models.key_rows, rows);
        let fonts = Some(self.fonts_fingerprint());
        let searched = Some({
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (fonts, &self.font_query).hash(&mut hasher);
            hasher.finish()
        });
        if self.fonts_published.get() != fonts || self.font_search_published.get() != searched {
            self.fonts_published.set(fonts);
            self.font_search_published.set(searched);
            let families = self.font_families();
            let query = self.font_query.trim().to_lowercase();
            sync(
                &models.font_matches,
                families
                    .iter()
                    .filter(|family| query.is_empty() || family.to_lowercase().contains(&query))
                    .map(SharedString::from)
                    .collect(),
            );
            sync(
                &models.font_families,
                families.into_iter().map(SharedString::from).collect(),
            );
        }
        editor.set_inspector_jump_token(self.inspector_jump.0);
        editor.set_library_audition(self.audition_of().unwrap_or("").into());
        editor.set_inspector_jump_tab(self.inspector_jump.1.into());
        editor.set_inspector_jump_section(self.inspector_jump.2.into());
        let active = doc
            .view
            .tabs
            .iter()
            .position(|id| Some(*id) == doc_view::showing(doc))
            .unwrap_or(0);
        editor.set_timeline_current_tab(active as i32);
        editor.set_playhead(self.playhead);
        sync(&models.key_marks, self.key_marks());
        editor.set_scroll_left(self.lanes.scroll_left);
        editor.set_seconds_per_pixel(self.lanes.seconds_per_pixel);
        editor.set_duration_seconds(self.duration());
        editor.set_frame_rate(self.frame_rate());
        editor.set_tool(self.lanes.tool);
        editor.set_snap(self.lanes.snap);
        editor.set_playhead_snap(self.prefs.playhead_snaps());
        editor.set_magnetic(self.prefs.magnetic());
        editor.set_linkage(doc_view::sequence(doc).is_some_and(|sequence| sequence.linkage));
        editor.set_trim_follow(self.prefs.trim_follow);
        editor.set_meters_shown(self.prefs.meters_shown());
        editor.set_click_moves_playhead(self.prefs.click_moves_playhead());
        editor.set_preview_full(self.preview_full());
        editor.set_preview_axis(self.prefs.preview_axis);
        editor.set_preview_axis_audio(self.prefs.preview_axis_audio);
        editor.set_pan_mode(self.lanes.pan_mode);
        editor.set_selected_count(self.selection.len() as i32);
        // The tray's undo and redo buttons grey out on these; the Edit menu
        // asks the session itself when it opens.
        // https://github.com/jub0t/Concat/issues/154
        let (can_undo, can_redo) = self.session.as_ref().map_or((false, false), |session| {
            (session.can_undo(), session.can_redo())
        });
        editor.set_can_undo(can_undo);
        editor.set_can_redo(can_redo);
        let (sound_selected, title_selected) = self.sound_tools();
        editor.set_sound_selected(sound_selected);
        editor.set_title_selected(title_selected);
        let (picture_selected, freezable, reversible) = self.transform_tools();
        editor.set_picture_selected(picture_selected);
        editor.set_freezable(freezable);
        editor.set_reversible(reversible);
        editor.set_merge_label(self.merge_label().into());
        editor.set_merge_blocked_because(match self.merge_blocked() {
            Some(reason) => reason.into(),
            None => SharedString::new(),
        });

        // The selected clip's chains, as the inspector's two stacks.
        let sole = self.sole_selection();
        let clip = sole.as_deref().and_then(|id| self.clip(id));
        let effects = clip.map_or_else(Vec::new, |clip| doc_view::chain(&clip.effects));
        let filters = clip.map_or_else(Vec::new, |clip| doc_view::chain(&clip.filters));
        let kind = clip.map(|clip| self.kind(clip));
        let video: Vec<EffectData> = effects
            .iter()
            .enumerate()
            .map(|(index, effect)| EffectData {
                id: index as i32 + 1,
                name: label_of(&effect.package).into(),
                audio: false,
            })
            .collect();
        let audio: Vec<EffectData> = filters
            .iter()
            .enumerate()
            .map(|(index, filter)| EffectData {
                id: index as i32 + 1000,
                name: label_of(&filter.package).into(),
                audio: true,
            })
            .collect();
        sync(&models.video_effects, video);
        sync(&models.audio_effects, audio);

        // The same chains as the inspector's stacks see them: a row per
        // link and a row per knob, from the catalogue's manifests.
        let (visual, visual_params) = chain_rows(&effects);
        let (sound, sound_params) = chain_rows(&filters);
        sync(&models.applied_visual, visual);
        sync(&models.visual_params, visual_params);
        sync(
            &models.visual_colours,
            match kind {
                Some(kind) => chain_colours(
                    &effects,
                    picks_colour(kind),
                    self.picking
                        .as_ref()
                        .filter(|pick| Some(pick.clip.as_str()) == sole.as_deref())
                        .map(|pick| (pick.entry, pick.key.as_str())),
                ),
                None => Vec::new(),
            },
        );
        sync_with(
            &models.visual_choices,
            chain_choices(&effects),
            same_chain_choice,
        );
        sync(&models.applied_audio, sound);
        sync(&models.audio_params, sound_params);
        let graded_clip = kind.is_some_and(graded);
        sync(
            &models.adjust_params,
            if graded_clip {
                adjust_rows(&effects, self.key_point().map(|(_, at)| at))
            } else {
                Vec::new()
            },
        );

        // The wheels and curves: the chain's by link, the colour panel's
        // keyable at the playhead. Each curve's points go into the model
        // kept for it.
        let (visual_wheels, visual_curves) = chain_grading(&effects);
        let (adjust_wheels, adjust_curves) = match Catalogue::builtin().get(ADJUST_ID) {
            Some(package) if graded_clip => grading_rows(
                package,
                effects
                    .iter()
                    .copied()
                    .find(|entry| entry.package == ADJUST_ID),
                -1,
                Some(self.key_point().map(|(_, at)| at)),
            ),
            _ => Default::default(),
        };
        let placed = |curves: Vec<(CurveData, Vec<CurvePointData>)>| -> Vec<CurveData> {
            let mut kept = models.curve_points.borrow_mut();
            curves
                .into_iter()
                .map(|(mut curve, points)| {
                    let model = Rc::clone(
                        kept.entry((curve.entry, curve.key.to_string()))
                            .or_insert_with(|| Rc::new(VecModel::default())),
                    );
                    sync(&model, points);
                    curve.points = ModelRc::from(model);
                    curve
                })
                .collect()
        };
        sync(&models.adjust_wheels, adjust_wheels);
        sync(&models.adjust_curves, placed(adjust_curves));
        // The chain's, one group a link, in chain order, each group's list
        // the model kept for its link.
        let visual_curves = placed(visual_curves);
        let mut entries: Vec<i32> = visual_wheels
            .iter()
            .map(|wheel| wheel.entry)
            .chain(visual_curves.iter().map(|curve| curve.entry))
            .collect();
        entries.sort_unstable();
        entries.dedup();
        let (mut wheel_groups, mut curve_groups) = (Vec::new(), Vec::new());
        for entry in entries {
            let wheels: Vec<WheelData> = visual_wheels
                .iter()
                .filter(|wheel| wheel.entry == entry)
                .cloned()
                .collect();
            if !wheels.is_empty() {
                let model = Rc::clone(
                    models
                        .link_wheels
                        .borrow_mut()
                        .entry(entry)
                        .or_insert_with(|| Rc::new(VecModel::default())),
                );
                sync(&model, wheels);
                wheel_groups.push(WheelGroupData {
                    entry,
                    wheels: ModelRc::from(model),
                });
            }
            let curves: Vec<CurveData> = visual_curves
                .iter()
                .filter(|curve| curve.entry == entry)
                .cloned()
                .collect();
            if !curves.is_empty() {
                let model = Rc::clone(
                    models
                        .link_curves
                        .borrow_mut()
                        .entry(entry)
                        .or_insert_with(|| Rc::new(VecModel::default())),
                );
                sync(&model, curves);
                curve_groups.push(CurveGroupData {
                    entry,
                    curves: ModelRc::from(model),
                });
            }
        }
        sync(&models.visual_wheels, wheel_groups);
        sync(&models.visual_curves, curve_groups);
    }
}
