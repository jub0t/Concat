// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Placing things on the timeline.

use super::*;

impl Studio {
    // ── placing things ──

    /// What a library payload names - "media:12", "text:default:Title" -
    /// before the timeline has decided where to put it.
    pub fn incoming(&self, payload: &str) -> Option<DropPlan> {
        let mut fields = payload.splitn(3, ':');
        let (sort, id) = (fields.next()?, fields.next()?);
        let label = fields.next().unwrap_or(id);
        match sort {
            "media" => {
                let item = self.media.by_row(self.project(), id.parse().ok()?)?;
                Some(DropPlan {
                    kind: match item.kind {
                        model::MediaKind::Audio => ClipKind::Audio,
                        model::MediaKind::Image => ClipKind::Image,
                        model::MediaKind::Video => ClipKind::Video,
                    },
                    label: item.name.clone(),
                    media: item.id.clone(),
                    start: 0.0,
                    // A still has no length of its own; a file with no stated
                    // duration gets the engine's own fallback.
                    duration: if item.kind == model::MediaKind::Image {
                        LAYER_DURATION
                    } else {
                        item.duration.unwrap_or(5.0) as f32
                    }
                    .max(MIN_DURATION),
                    row: 0,
                })
            }
            // A title: the preset's id rides where a file's media id would,
            // "default" for the plain one.
            "text" => Some(DropPlan {
                kind: ClipKind::Text,
                label: label.to_owned(),
                media: id.to_owned(),
                start: 0.0,
                duration: LAYER_DURATION,
                row: 0,
            }),
            // A figure from the Stickers page: the shape's name rides
            // where a file's media id would, and the label is what the
            // lane will call it.
            "shape" => Some(DropPlan {
                kind: ClipKind::Shape,
                label: label.to_owned(),
                media: model::ShapeStyle::preset(id).map(|_| id.to_owned())?,
                start: 0.0,
                duration: LAYER_DURATION,
                row: 0,
            }),
            // A sound from the Audio page: not in the bin yet, so its card
            // rides in `media` behind `sound:`, and the drop imports it.
            "sound" => {
                let (name, duration) = self.sound_summary(id)?;
                Some(DropPlan {
                    kind: ClipKind::Audio,
                    label: name,
                    media: format!("sound:{id}"),
                    start: 0.0,
                    duration: (duration as f32).max(MIN_DURATION),
                    row: 0,
                })
            }
            // A look dragged from the Filters page: a layer over a span.
            // The package id rides in `media`, there being no file.
            "filter" => Some(DropPlan {
                kind: super::layer_kind(Some(id)),
                label: label.to_owned(),
                media: id.to_owned(),
                start: 0.0,
                duration: LAYER_DURATION,
                row: 0,
            }),
            // A transition dragged from the Transitions page: a clip of its
            // own over the lanes beneath it (#621), drawn as a layer is. The
            // package id rides in `media` behind `transition:`, which
            // `place` reads.
            "transition" => Some(DropPlan {
                kind: ClipKind::Transition,
                label: label.to_owned(),
                media: format!("transition:{id}"),
                start: 0.0,
                duration: TRANSITION_DURATION,
                row: 0,
            }),
            // A sound effect dragged from the Effects page's sound shelf:
            // a sound layer over a span, treating everything heard there.
            // The package id rides in `media`, as a filter's does.
            "audio-filter" => Some(DropPlan {
                kind: ClipKind::SoundLayer,
                label: label.to_owned(),
                media: id.to_owned(),
                start: 0.0,
                duration: LAYER_DURATION,
                row: 0,
            }),
            _ => None,
        }
    }

    /// A drag over the lanes: the pointer names the moment and the lane.
    pub fn plan(&self, payload: &str, seconds: f32, row: i32) -> Option<DropPlan> {
        let mut plan = self.incoming(payload)?;
        let lanes = self.timeline().tracks.len() as i32;
        if lanes == 0 {
            return None;
        }
        plan.row = row.clamp(0, lanes - 1);
        if self
            .row_track(plan.row)
            .is_none_or(|track| self.locked(&track.id))
        {
            return None;
        }
        plan.start = self
            .snapped(seconds.max(0.0), 8.0 * self.lanes.seconds_per_pixel, "")
            .max(0.0);
        // A transition near a cut is centred on it, as `place_transition`
        // will lay it, so the ghost is where the clip lands.
        if plan.media.starts_with("transition:") {
            let half = f64::from(plan.duration) / 2.0;
            if let Some(cut) = self.nearest_cut(f64::from(plan.start), half) {
                plan.start = (cut - half).max(0.0) as f32;
            }
        }
        Some(plan)
    }

    /// The bin's selection, in the bin's order, when `media_id` is one card
    /// of a selection of several: what a drag of that card carries.
    fn dragged_selection(&self, media_id: &str) -> Option<Vec<String>> {
        let held = &self.media.selected;
        if held.len() < 2 || !held.contains(media_id) {
            return None;
        }
        Some(
            self.project()
                .media
                .iter()
                .filter(|item| held.contains(&item.id))
                .map(|item| item.id.clone())
                .collect(),
        )
    }

    /// Commits a plan that has a lane, and selects what it made.
    pub fn place(&mut self, plan: &DropPlan) {
        let Some(track_id) = self.row_track(plan.row).map(|track| track.id.clone()) else {
            return;
        };
        if let Some(card) = plan.media.strip_prefix("sound:") {
            self.place_card(card, Some(track_id), f64::from(plan.start));
            return;
        }
        if let Some(transition) = plan.media.strip_prefix("transition:") {
            let created = self.place_transition(
                transition,
                &plan.label,
                Some(track_id),
                f64::from(plan.start),
                f64::from(plan.duration),
            );
            if let Some(id) = created {
                self.selection = vec![id];
            }
            return;
        }
        let created = if plan.kind == ClipKind::Text {
            self.add_title(
                Some(track_id),
                f64::from(plan.start),
                f64::from(plan.duration),
                &plan.media,
            )
        } else if plan.kind == ClipKind::Shape {
            self.add_shape(
                Some(track_id),
                f64::from(plan.start),
                f64::from(plan.duration),
                &plan.media,
                &plan.label,
            )
        } else if matches!(
            plan.kind,
            ClipKind::Filter | ClipKind::Effect | ClipKind::Adjustment | ClipKind::SoundLayer
        ) {
            self.apply(Command::AddLayerClip {
                track_id: Some(track_id),
                start: f64::from(plan.start),
                duration: Some(f64::from(plan.duration)),
                effect_id: plan.media.clone(),
                name: plan.label.clone(),
                sound: plan.kind == ClipKind::SoundLayer,
            })
        } else if let Some(media_ids) = self.dragged_selection(&plan.media) {
            // A card dragged out of a selection in the bin brings the whole
            // selection, laid end to end from the drop, in the bin's order.
            // Only the dragged card used to land (reported on Reddit).
            self.apply(Command::AddClipsInSequence {
                media_ids,
                track_id: Some(track_id),
                start: f64::from(plan.start),
                ripple: true,
            })
        } else {
            self.apply(Command::AddClip {
                media_id: plan.media.clone(),
                track_id,
                start: f64::from(plan.start),
                ripple: true,
            })
        };
        if let Some(id) = created {
            self.selection = vec![id];
        }
    }

    /// A card clicked rather than dragged: the playhead names the moment
    /// and the engine finds a lane with room.
    pub fn place_at_playhead(&mut self, payload: &str) {
        let Some(plan) = self.incoming(payload) else {
            return;
        };
        let start = f64::from(self.playhead.max(0.0));
        if let Some(card) = plan.media.strip_prefix("sound:") {
            self.place_card(card, None, start);
            return;
        }
        if let Some(transition) = plan.media.strip_prefix("transition:") {
            let created = self.place_transition(
                transition,
                &plan.label,
                None,
                start,
                f64::from(plan.duration),
            );
            if let Some(id) = created {
                self.selection = vec![id];
            }
            return;
        }
        let created = if plan.kind == ClipKind::Text {
            self.add_title(None, start, f64::from(plan.duration), &plan.media)
        } else if plan.kind == ClipKind::Shape {
            self.add_shape(
                None,
                start,
                f64::from(plan.duration),
                &plan.media,
                &plan.label,
            )
        } else if matches!(
            plan.kind,
            ClipKind::Filter | ClipKind::Effect | ClipKind::Adjustment | ClipKind::SoundLayer
        ) {
            self.apply(Command::AddLayerClip {
                track_id: None,
                start,
                duration: Some(f64::from(plan.duration)),
                effect_id: plan.media.clone(),
                name: plan.label.clone(),
                sound: plan.kind == ClipKind::SoundLayer,
            })
        } else {
            self.apply(Command::AddClipAtFirstFree {
                media_id: plan.media.clone(),
                start,
            })
        };
        if let Some(id) = created {
            self.selection = vec![id];
        }
    }

    /// Places a title in the look a preset names - "default" for the plain
    /// title - and, when the preset brings a font, registers the font on
    /// the project in the same step, so the painter finds it the first
    /// time it draws the words.
    fn add_title(
        &mut self,
        track_id: Option<String>,
        start: f64,
        duration: f64,
        preset: &str,
    ) -> Option<String> {
        let (style, offset_y, font) = match self.text_presets.iter().find(|held| held.id == preset)
        {
            Some(found) => (
                found.style.clone(),
                found.offset_y,
                presets::install_font(&self.host.dirs, found),
            ),
            None => (new_title_style(&self.prefs), None, None),
        };
        // Over the picture, not under it: the first free lane above every
        // occupied one, minted at the top when there is none. A drop onto a
        // lane names its track and is placed there regardless.
        let add = Command::AddTextClip {
            track_id,
            start,
            style: Some(style),
            duration: Some(duration),
            offset_y,
            above: true,
        };
        match font {
            Some((family, path)) => self.apply(Command::Batch {
                commands: vec![Command::AddFont { family, path }, add],
            }),
            None => self.apply(add),
        }
    }

    /// A shape from the Stickers page, placed the way a title is: over
    /// the picture, on the first free lane above it.
    fn add_shape(
        &mut self,
        track_id: Option<String>,
        start: f64,
        duration: f64,
        shape: &str,
        name: &str,
    ) -> Option<String> {
        self.apply(Command::AddShapeClip {
            track_id,
            above: true,
            start,
            style: Some(model::ShapeStyle::preset(shape).unwrap_or_default()),
            duration: Some(duration),
            name: name.to_owned(),
        })
    }

    /// The selected clip's id, when exactly one is selected.
    /// The clips the inspector's knobs write to: the one selected clip, or
    /// every selected clip when two or more are selected and all of them
    /// are the same kind of thing - titles, pictures, shapes, adjustment
    /// layers or sounds. A batch is edited together: the inspector shows
    /// the first, and each knob turned, effect added or background picked
    /// lands on every one of them as one undo step (#470), which is what a
    /// separate Apply to all button stood in for. Clips on locked lanes are
    /// left out; kinds selected together have no inspector in common.
    /// https://github.com/jub0t/Concat/issues/224
    pub fn edit_targets(&self) -> Vec<String> {
        if let Some(id) = self.sole_selection() {
            return vec![id];
        }
        if self.selection.len() < 2 {
            return Vec::new();
        }
        let family = |id: &String| self.clip(id).map(|clip| inspector_family(clip.kind));
        let first = family(&self.selection[0]);
        if first.is_none() || !self.selection.iter().all(|id| family(id) == first) {
            return Vec::new();
        }
        self.selection
            .iter()
            .filter(|id| {
                self.clip(id)
                    .is_some_and(|clip| !self.locked(&clip.track_id))
            })
            .cloned()
            .collect()
    }

    pub(super) fn volume_targets(&self) -> Vec<String> {
        crate::batch_volume::targets(self.project(), &self.selection, |track| self.locked(track))
    }

    pub fn sole_selection(&self) -> Option<String> {
        (self.selection.len() == 1).then(|| self.selection[0].clone())
    }

    /// The look being shown over the picture, by id, while one is.
    pub fn audition_of(&self) -> Option<&str> {
        self.audition.as_deref()
    }

    /// Shows a look over the whole picture without laying it down: what a
    /// single click on a Filters card does. The same card clicked again
    /// takes it off; a double-click or the card's plus lays the layer.
    pub fn audition_catalogue(&mut self, id: &str) {
        if self.session.is_none() {
            return;
        }
        let same = self.audition.as_deref() == Some(id);
        if !same && self.audition.is_none() {
            self.notify(&t("studio.showingLookOverPicture"), false);
        }
        self.audition = (!same).then(|| id.to_owned());
        self.request_preview();
    }

    /// Takes a look's showing off the picture, when one is up: what a
    /// press on a clip, Esc and the monitor's cross do. A look left showing
    /// after the hand had moved on read as one laid on the clip, with no
    /// tab to take it off by.
    pub fn end_audition(&mut self) {
        if self.audition.take().is_some() {
            self.request_preview();
        }
    }

    /// Lays a filter down as a layer at the playhead - what the Filters
    /// page's double-click and plus do - and ends the showing, the layer
    /// now being on the timeline to see.
    pub fn place_filter_layer(&mut self, id: &str, label: &str) {
        self.audition = None;
        self.place_at_playhead(&format!("filter:{id}:{label}"));
    }

    /// Lays an adjustment layer at the playhead - the colour panel's own
    /// package at its defaults, over everything beneath it, so it changes
    /// nothing until a knob moves - called `label`, and opens the
    /// inspector on its Adjust page, where those knobs are.
    pub fn place_adjustment_layer(&mut self, label: &str) {
        self.place_filter_layer(ADJUST_ID, label);
        self.inspector_jump = (self.inspector_jump.0 + 1, "Layer", "Adjust");
    }

    /// A LUT from the Adjustments page: onto the selected clip's chain -
    /// a picture, a title, a shape or a layer - the way an effect goes on;
    /// with nothing selected, laid at the playhead as a layer of its own.
    pub fn apply_lut(&mut self, id: &str, label: &str) {
        if !self.edit_targets().is_empty() {
            self.apply_catalogue(id, true);
        } else {
            self.place_filter_layer(id, label);
        }
    }

    /// A catalogue filter or effect, applied to the selected clip's chain.
    pub fn apply_catalogue(&mut self, id: &str, video: bool) {
        let ids = self.edit_targets();
        let Some(first) = ids.first().and_then(|id| self.clip(id)) else {
            self.notify(&t("studio.selectClipTimelineFirst"), true);
            return;
        };
        // A batch is one kind of thing, so the first speaks for all of it.
        if video
            && !(graded(first.kind)
                || matches!(first.kind, model::ClipKind::Text | model::ClipKind::Shape))
        {
            self.notify(&t("studio.selectVideoImageText"), true);
            return;
        }
        if !video && first.kind == model::ClipKind::Image {
            self.notify("A still has no sound to filter", true);
            return;
        }
        let mut commands: Vec<Command> = ids
            .iter()
            .filter_map(|clip_id| {
                let clip = self.clip(clip_id)?;
                let entry = AppliedFilter::new(id);
                let patch = if video {
                    let mut effects = clip.video_effects.clone();
                    effects.push(entry);
                    ClipPatch {
                        video_effects: Some(effects),
                        ..ClipPatch::default()
                    }
                } else {
                    let mut filters = clip.filters.clone();
                    filters.push(entry);
                    ClipPatch {
                        filters: Some(filters),
                        ..ClipPatch::default()
                    }
                };
                Some(Command::UpdateClip {
                    clip_id: clip_id.clone(),
                    patch,
                })
            })
            .collect();
        let command = match commands.len() {
            0 => return,
            1 => commands.remove(0),
            _ => Command::Batch { commands },
        };
        self.apply(command);
        // Show it: the applied chain is where the knobs are, and a card
        // that did something with no visible result reads as a card that
        // did nothing.
        self.inspector_jump = (
            self.inspector_jump.0 + 1,
            if video { "Effects" } else { "Audio" },
            if video { "Effects" } else { "AudioEffects" },
        );
    }

    /// The clip that ends where `clip` starts, on the same track - the
    /// outgoing side of the cut a transition into `clip` rides. `None` when
    /// nothing is adjacent, which is when a transition has nothing to
    /// dissolve from.
    fn outgoing_of(&self, clip: &Clip) -> Option<&Clip> {
        let frame = self.frame_seconds();
        self.timeline()
            .clips
            .iter()
            .find(|other| {
                other.id != clip.id
                    && other.track_id == clip.track_id
                    && (other.start + other.duration - clip.start).abs() < frame / 2.0
            })
            .map(|other| other.as_ref())
    }

    /// One frame of the active timeline, in seconds - the same tolerance
    /// `concat-export`'s own adjacency test uses, so a clip this call finds
    /// adjacent is one the exporter will too.
    fn frame_seconds(&self) -> f64 {
        let video = &self.timeline().video;
        video.rate_den as f64 / video.rate_num.max(1) as f64
    }

    /// The duration a transition into `clip` can actually run at: the
    /// requested length, clamped to both clips' own durations. `None` when there
    /// is no adjacent clip to dissolve from, or both clips are shorter than a frame -
    /// the ways a transition here would render as nothing at all.
    pub fn transition_duration(&self, clip: &Clip, requested: f64) -> Option<f64> {
        let outgoing = self.outgoing_of(clip)?;
        let frame = self.frame_seconds();
        let max_duration = outgoing.duration.min(clip.duration);
        if max_duration < frame {
            return None;
        }
        let requested = if requested <= 0.0 {
            frame
        } else {
            requested.max(frame)
        };
        let duration = requested.min(max_duration).max(0.0);
        (duration >= frame).then_some(duration)
    }

    /// The cut a transition from the library goes on. Two clips selected
    /// that meet on one track name the cut between them - the simple way,
    /// and the one the Transitions page asks for. One clip selected names
    /// the cut into it, for a transition picked after a click on the lane.
    /// The incoming clip's id, or what to say instead.
    fn transition_cut(&self) -> Result<String, String> {
        match self.selection.as_slice() {
            [one] => Ok(one.clone()),
            [a, b] => {
                let (Some(a), Some(b)) = (self.clip(a), self.clip(b)) else {
                    return Err(t("studio.selectTwoAdjacentClips"));
                };
                let (first, second) = if a.start <= b.start { (a, b) } else { (b, a) };
                if self
                    .outgoing_of(second)
                    .is_some_and(|outgoing| outgoing.id == first.id)
                {
                    Ok(second.id.clone())
                } else {
                    Err(t("studio.clipsDoNotMeet"))
                }
            }
            _ => Err(t("studio.selectTwoAdjacentClips")),
        }
    }

    /// Places the catalogue transition `id` as a clip of its own (#621):
    /// centred on the cut the selection names (see
    /// [`Studio::transition_cut`]), or, with no cut selected, at the
    /// playhead - on the cut there if one is near, over the shot under it
    /// if not. A transition is no longer written onto the clip coming in.
    pub fn apply_transition(&mut self, id: &str) {
        let label = Catalogue::builtin()
            .get(id)
            .map(|package| package.manifest.effect.name.clone())
            .unwrap_or_default();
        let duration = f64::from(TRANSITION_DURATION);
        let at = match self.transition_cut() {
            Ok(clip_id) => match self.clip(&clip_id) {
                Some(clip) if self.outgoing_of(clip).is_some() => clip.start,
                _ => f64::from(self.playhead.max(0.0)),
            },
            Err(_) => f64::from(self.playhead.max(0.0)),
        };
        if let Some(created) = self.place_transition(id, &label, None, at, duration) {
            self.selection = vec![created];
        }
    }

    /// Lays the catalogue transition `id` as a clip `duration` long (#621):
    /// centred on the cut nearest `at` when one is within half its length,
    /// from `at` over the shot there otherwise. On `track_id` when that
    /// lane is free over its span; over the picture otherwise, where a
    /// layer goes, since a transition acts on what is beneath it.
    fn place_transition(
        &mut self,
        id: &str,
        label: &str,
        track_id: Option<String>,
        at: f64,
        duration: f64,
    ) -> Option<String> {
        let duration = duration.max(model::ranges::MIN_TRANSITION);
        let start = match self.nearest_cut(at, duration / 2.0) {
            Some(cut) => (cut - duration / 2.0).max(0.0),
            None => at.max(0.0),
        };
        let track_id = track_id.filter(|lane| {
            let free = self
                .timeline()
                .nearest_free_start(lane, start, duration, &[]);
            (free - start).abs() < 1e-9
        });
        self.apply(Command::AddTransitionClip {
            track_id,
            start,
            duration: Some(duration),
            transition_id: id.to_owned(),
            name: label.to_owned(),
        })
    }

    /// The cut nearest `at`, within `within` seconds: where a picture ends
    /// and the next on its lane begins.
    fn nearest_cut(&self, at: f64, within: f64) -> Option<f64> {
        let clips = &self.timeline().clips;
        clips
            .iter()
            .filter(|incoming| {
                incoming.kind.is_visual()
                    && clips.iter().any(|outgoing| {
                        outgoing.id != incoming.id
                            && outgoing.kind.is_visual()
                            && outgoing.track_id == incoming.track_id
                            && (outgoing.start + outgoing.duration - incoming.start).abs() < 1e-3
                    })
            })
            .map(|clip| clip.start)
            .filter(|cut| (cut - at).abs() <= within)
            .min_by(|a, b| (a - at).abs().total_cmp(&(b - at).abs()))
    }

    /// The clip whose incoming transition is selected: the one picked on
    /// its own while no clip is, if it still has a transition.
    pub fn selected_transition(&self) -> Option<&Clip> {
        if !self.selection.is_empty() {
            return None;
        }
        let id = self.transition_selected.as_deref()?;
        self.clip(id).filter(|clip| clip.transition_in.is_some())
    }

    /// The clip a transition verb acts on: the selected transition's, or
    /// the one selected clip's.
    fn transition_target(&self) -> Option<String> {
        self.selected_transition()
            .map(|clip| clip.id.clone())
            .or_else(|| self.sole_selection())
    }

    /// Takes the transition off its clip, leaving a plain cut.
    pub fn remove_transition(&mut self) {
        let Some(clip_id) = self.transition_target() else {
            return;
        };
        self.transition_selected = None;
        self.apply(Command::UpdateClip {
            clip_id,
            patch: ClipPatch {
                transition_in: Some(None),
                ..ClipPatch::default()
            },
        });
    }

    /// Sets the selected clip's transition to `seconds`, clamped to what
    /// the cut can actually support - the same ceiling `apply_transition`
    /// enforces, kept here so a drag on the timeline can never ask for more
    /// than the render will give it.
    pub fn set_transition_duration(&mut self, seconds: f64) {
        let Some(clip_id) = self.transition_target() else {
            return;
        };
        self.set_clip_transition_duration(&clip_id, seconds);
    }

    /// Sets one of the selected transition's own knobs, held inside the
    /// range its package gives it. A knob the package does not have is
    /// refused rather than stored.
    pub fn set_transition_param(&mut self, key: &str, value: f64) {
        let Some(clip_id) = self.transition_target() else {
            return;
        };
        let Some(existing) = self
            .clip(&clip_id)
            .and_then(|clip| clip.transition_in.clone())
        else {
            return;
        };
        let Some(param) = Catalogue::builtin()
            .get(&existing.id)
            .and_then(|package| package.manifest.param(key).cloned())
        else {
            return;
        };
        if !value.is_finite() {
            return;
        }
        let mut params = existing.params.clone();
        params.insert(key.to_owned(), value.clamp(param.min, param.max));
        // One undo step a drag, not one a tick of it.
        self.apply_within(
            &format!("transition-knob:{clip_id}:{key}"),
            Command::UpdateClip {
                clip_id,
                patch: ClipPatch {
                    transition_in: Some(Some(Transition { params, ..existing })),
                    ..ClipPatch::default()
                },
            },
        );
    }

    /// Sets `clip_id`'s transition to `seconds`, clamped to what the cut can
    /// actually support.
    pub fn set_clip_transition_duration(&mut self, clip_id: &str, seconds: f64) {
        let Some(clip) = self.clip(clip_id) else {
            return;
        };
        let Some(existing) = clip.transition_in.clone() else {
            return;
        };
        let Some(duration) = self.transition_duration(clip, seconds.max(0.0)) else {
            return;
        };
        self.apply(Command::UpdateClip {
            clip_id: clip_id.to_owned(),
            patch: ClipPatch {
                transition_in: Some(Some(Transition {
                    duration,
                    ..existing
                })),
                ..ClipPatch::default()
            },
        });
    }

    /// Drops one entry from the selected clip's chains, by the row id the
    /// inspector shows: video effects count from 1, audio filters from 1000.
    pub fn remove_effect(&mut self, row: i32) {
        if row >= 1000 {
            self.chain_remove(true, row - 1000);
        } else {
            self.chain_remove(false, (row - 1).max(0));
        }
    }
}
