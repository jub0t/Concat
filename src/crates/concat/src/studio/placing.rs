// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Placing things on the timeline.

use concat_doc::model::{CutIn, MediaKind};

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
                let card = self.media.by_row(self.doc(), id.parse().ok()?)?;
                let (media_id, item) = (card.media, card.item);
                Some(DropPlan {
                    kind: match item.kind {
                        MediaKind::Audio => ClipKind::Audio,
                        MediaKind::Image => ClipKind::Image,
                        MediaKind::Video => ClipKind::Video,
                        MediaKind::Unknown(_) => return None,
                    },
                    label: item.name.clone(),
                    media: media_id.to_string(),
                    start: 0.0,
                    // A still has no length of its own; a file with no stated
                    // duration gets the document's own.
                    duration: if item.kind == MediaKind::Image {
                        LAYER_DURATION
                    } else {
                        doc_view::len_secs(item.length.unwrap_or(concat_doc::commands::STILL_LEN))
                            as f32
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
                media: doc_view::shape_preset(id).map(|_| id.to_owned())?,
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
        let lanes = doc_view::lanes(self.doc()).len() as i32;
        if lanes == 0 {
            return None;
        }
        plan.row = row.clamp(0, lanes - 1);
        if self
            .row_track(plan.row)
            .is_none_or(|lane| self.lane_locked(lane))
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
    fn dragged_selection(&self, media_id: &str) -> Option<Vec<MediaId>> {
        let held = &self.media.selected;
        if held.len() < 2 || !held.contains(media_id) {
            return None;
        }
        Some(
            doc_view::bin(self.doc())
                .filter(|(id, _)| held.contains(&id.to_string()))
                .map(|(id, _)| id)
                .collect(),
        )
    }

    /// Commits a plan that has a lane, and selects what it made.
    pub fn place(&mut self, plan: &DropPlan) {
        let Some(lane) = self.row_track(plan.row) else {
            return;
        };
        if let Some(card) = plan.media.strip_prefix("sound:") {
            self.place_card(card, Some(lane.to_string()), f64::from(plan.start));
            return;
        }
        if let Some(transition) = plan.media.strip_prefix("transition:") {
            let created = self.place_transition(
                transition,
                &plan.label,
                Some(lane),
                f64::from(plan.start),
                f64::from(plan.duration),
            );
            if let Some(id) = created {
                self.selection = vec![id];
            }
            return;
        }
        let start = doc_view::tick(f64::from(plan.start));
        let len = doc_view::len(f64::from(plan.duration));
        let created = if plan.kind == ClipKind::Text {
            self.add_title(Some(lane), start, len, &plan.media)
        } else if plan.kind == ClipKind::Shape {
            self.add_shape(Some(lane), start, len, &plan.media, &plan.label)
        } else if matches!(
            plan.kind,
            ClipKind::Filter | ClipKind::Effect | ClipKind::Adjustment | ClipKind::SoundLayer
        ) {
            self.apply_making(Command::AddLayerClip {
                start,
                len: Some(len),
                package: plan.media.clone(),
                name: plan.label.clone(),
                sound: plan.kind == ClipKind::SoundLayer,
                lane: Some(lane),
            })
        } else if let Some(media) = self.dragged_selection(&plan.media) {
            // A card dragged out of a selection in the bin brings the whole
            // selection, laid end to end from the drop, in the bin's order.
            // Only the dragged card used to land (reported on Reddit).
            self.apply_making(Command::AddClipsInSequence {
                media,
                start,
                lane: Some(lane),
                ripple: true,
            })
        } else if let Some(media) = doc_view::parse::<MediaId>(&plan.media) {
            self.apply_making(Command::AddClip {
                media,
                lane,
                start,
                ripple: true,
            })
        } else {
            None
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
        let seconds = f64::from(self.playhead.max(0.0));
        if let Some(card) = plan.media.strip_prefix("sound:") {
            self.place_card(card, None, seconds);
            return;
        }
        if let Some(transition) = plan.media.strip_prefix("transition:") {
            let created = self.place_transition(
                transition,
                &plan.label,
                None,
                seconds,
                f64::from(plan.duration),
            );
            if let Some(id) = created {
                self.selection = vec![id];
            }
            return;
        }
        let start = doc_view::tick(seconds);
        let len = doc_view::len(f64::from(plan.duration));
        let created = if plan.kind == ClipKind::Text {
            self.add_title(None, start, len, &plan.media)
        } else if plan.kind == ClipKind::Shape {
            self.add_shape(None, start, len, &plan.media, &plan.label)
        } else if matches!(
            plan.kind,
            ClipKind::Filter | ClipKind::Effect | ClipKind::Adjustment | ClipKind::SoundLayer
        ) {
            self.apply_making(Command::AddLayerClip {
                start,
                len: Some(len),
                package: plan.media.clone(),
                name: plan.label.clone(),
                sound: plan.kind == ClipKind::SoundLayer,
                lane: None,
            })
        } else if let Some(media) = doc_view::parse::<MediaId>(&plan.media) {
            self.apply_making(Command::AddClipAtFirstFree { media, start })
        } else {
            None
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
        lane: Option<LaneId>,
        start: Tick,
        len: TickLen,
        preset: &str,
    ) -> Option<String> {
        let (title, offset_y, font) = match self.text_presets.iter().find(|held| held.id == preset)
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
        // lane names its lane and is placed there regardless.
        let add = Command::AddTextClip {
            start,
            len: Some(len),
            lane,
            above: true,
            offset_y,
            title: Some(title),
        };
        match font {
            Some((family, path)) => self.apply_making(Command::Batch {
                commands: vec![Command::AddFont { family, path }, add],
            }),
            None => self.apply_making(add),
        }
    }

    /// A shape from the Stickers page, placed the way a title is: over
    /// the picture, on the first free lane above it.
    fn add_shape(
        &mut self,
        lane: Option<LaneId>,
        start: Tick,
        len: TickLen,
        shape: &str,
        name: &str,
    ) -> Option<String> {
        self.apply_making(Command::AddShapeClip {
            start,
            len: Some(len),
            name: name.to_owned(),
            style: Some(doc_view::shape_preset(shape).unwrap_or_default()),
            lane,
            above: true,
        })
    }

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
        let family = |id: &String| self.clip(id).map(|clip| inspector_family(self.kind(clip)));
        let first = family(&self.selection[0]);
        if first.is_none() || !self.selection.iter().all(|id| family(id) == first) {
            return Vec::new();
        }
        self.selection
            .iter()
            .filter(|id| {
                self.clip(id)
                    .is_some_and(|clip| !self.lane_locked(clip.lane))
            })
            .cloned()
            .collect()
    }

    /// The titles a change to a title's look writes to: [`Self::edit_targets`],
    /// except that a caption selected alone stands for every title on its
    /// caption lane - a caption lane has one look, so restyling a caption
    /// restyles them all, as one undo step. Its words, effects, masks and
    /// keys stay its own; only the look is shared.
    pub fn style_targets(&self) -> Vec<String> {
        let ids = self.edit_targets();
        let [only] = ids.as_slice() else {
            return ids;
        };
        let Some(clip) = self.clip(only) else {
            return ids;
        };
        let lane = clip.lane;
        let caption_lane = self.kind(clip) == Kind::Text
            && !self.lane_locked(lane)
            && doc_view::lane(self.doc(), lane).is_some_and(|lane| lane.captions.is_some());
        if !caption_lane {
            return ids;
        }
        let mut lane_titles: Vec<String> = doc_view::on_lane(self.doc(), lane)
            .filter(|(_, other)| self.kind(other) == Kind::Text)
            .map(|(id, _)| id.to_string())
            .collect();
        // The selected one first: a batch's knobs read the first clip.
        lane_titles.retain(|id| id != only);
        lane_titles.insert(0, only.clone());
        lane_titles
    }

    pub(super) fn volume_targets(&self) -> Vec<String> {
        crate::batch_volume::targets(self.doc(), &self.selection, |lane| self.locked(lane))
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
        let Some(first) = ids
            .first()
            .and_then(|id| self.clip(id))
            .map(|c| self.kind(c))
        else {
            self.notify(&t("studio.selectClipTimelineFirst"), true);
            return;
        };
        // A batch is one kind of thing, so the first speaks for all of it.
        if video && !(graded(first) || matches!(first, Kind::Text | Kind::Shape)) {
            self.notify(&t("studio.selectVideoImageText"), true);
            return;
        }
        if !video && first == Kind::Image {
            self.notify(&t("studio.stillHasNoSound"), true);
            return;
        }
        let mut commands: Vec<Command> = ids
            .iter()
            .filter_map(|clip_id| {
                let clip = self.clip(clip_id)?;
                let entry = doc_view::new_effect(id);
                let patch = if video {
                    let mut effects = doc_view::chain_owned(&clip.effects);
                    effects.push(entry);
                    ItemPatch {
                        effects: Some(effects),
                        ..ItemPatch::default()
                    }
                } else {
                    let mut filters = doc_view::chain_owned(&clip.filters);
                    filters.push(entry);
                    ItemPatch {
                        filters: Some(filters),
                        ..ItemPatch::default()
                    }
                };
                Some(Command::UpdateClip {
                    item: doc_view::parse(clip_id)?,
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

    /// The clip that ends where `clip` starts, on the same lane - the
    /// outgoing side of the cut a transition into `clip` rides. `None` when
    /// nothing is adjacent, which is when a transition has nothing to
    /// dissolve from.
    fn outgoing_of(&self, clip: &Item) -> Option<&Item> {
        let frame = self.frame_seconds();
        let start = doc_view::start_s(clip);
        doc_view::on_lane(self.doc(), clip.lane)
            .map(|(_, other)| other)
            .find(|other| *other != clip && (doc_view::end_s(other) - start).abs() < frame / 2.0)
    }

    /// One frame of the sequence showing, in seconds - the tolerance the
    /// export's own adjacency test uses, so a clip this call finds adjacent
    /// is one the exporter will too.
    fn frame_seconds(&self) -> f64 {
        1.0 / doc_view::rate(self.doc()).max(1.0)
    }

    /// The duration a transition into `clip` can actually run at, in
    /// seconds: the requested length, clamped to both clips' own durations.
    /// `None` when there is no adjacent clip to dissolve from, or both
    /// clips are shorter than a frame - the ways a transition here would
    /// render as nothing at all.
    pub fn transition_duration(&self, clip: &Item, requested: f64) -> Option<f64> {
        let outgoing = self.outgoing_of(clip)?;
        let frame = self.frame_seconds();
        let max_duration = doc_view::len_s(outgoing).min(doc_view::len_s(clip));
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
    /// that meet on one lane name the cut between them - the simple way,
    /// and the one the Transitions page asks for. One clip selected names
    /// the cut into it, for a transition picked after a click on the lane.
    /// The incoming clip's id, or what to say instead.
    fn transition_cut(&self) -> Result<String, String> {
        match self.selection.as_slice() {
            [one] => Ok(one.clone()),
            [a_id, b_id] => {
                let (Some(a), Some(b)) = (self.clip(a_id), self.clip(b_id)) else {
                    return Err(t("studio.selectTwoAdjacentClips"));
                };
                let ((first, _), (second, second_id)) = if a.start <= b.start {
                    ((a, a_id), (b, b_id))
                } else {
                    ((b, b_id), (a, a_id))
                };
                if self
                    .outgoing_of(second)
                    .is_some_and(|outgoing| outgoing == first)
                {
                    Ok(second_id.clone())
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
    /// if not.
    pub fn apply_transition(&mut self, id: &str) {
        let label = Catalogue::builtin()
            .get(id)
            .map(|package| package.manifest.effect.name.clone())
            .unwrap_or_default();
        let duration = f64::from(TRANSITION_DURATION);
        let playhead = f64::from(self.playhead.max(0.0));
        let at = match self.transition_cut() {
            Ok(clip_id) => match self.clip(&clip_id) {
                Some(clip) if self.outgoing_of(clip).is_some() => doc_view::start_s(clip),
                _ => playhead,
            },
            Err(_) => playhead,
        };
        if let Some(created) = self.place_transition(id, &label, None, at, duration) {
            self.selection = vec![created];
        }
    }

    /// Lays the catalogue transition `id` as a clip `duration` seconds long
    /// (#621): centred on the cut nearest `at` when one is within half its
    /// length, from `at` over the shot there otherwise. On `lane` when that
    /// lane is free over its span; over the picture otherwise, where a
    /// layer goes, since a transition acts on what is beneath it.
    fn place_transition(
        &mut self,
        id: &str,
        label: &str,
        lane: Option<LaneId>,
        at: f64,
        duration: f64,
    ) -> Option<String> {
        let shortest = doc_view::len_secs(concat_doc::commands::MIN_TRANSITION);
        let duration = duration.max(shortest);
        let start = match self.nearest_cut(at, duration / 2.0) {
            Some(cut) => (cut - duration / 2.0).max(0.0),
            None => at.max(0.0),
        };
        let (start, len) = (doc_view::tick(start), doc_view::len(duration));
        let lane = lane.filter(|lane| doc_view::is_free(self.doc(), *lane, start, len, &[]));
        self.apply_making(Command::AddTransitionClip {
            start,
            len: Some(len),
            package: id.to_owned(),
            name: label.to_owned(),
            lane,
        })
    }

    /// The cut nearest `at`, within `within` seconds: where a picture ends
    /// and the next on its lane begins.
    fn nearest_cut(&self, at: f64, within: f64) -> Option<f64> {
        let clips = doc_view::items(self.doc());
        clips
            .iter()
            .filter(|(id, incoming)| {
                self.kind(incoming).is_visual()
                    && clips.iter().any(|(other, outgoing)| {
                        other != id
                            && self.kind(outgoing).is_visual()
                            && outgoing.lane == incoming.lane
                            && (doc_view::end_s(outgoing) - doc_view::start_s(incoming)).abs()
                                < 1e-3
                    })
            })
            .map(|(_, clip)| doc_view::start_s(clip))
            .filter(|cut| (cut - at).abs() <= within)
            .min_by(|a, b| (a - at).abs().total_cmp(&(b - at).abs()))
    }

    /// The clip whose incoming transition is selected: the one picked on
    /// its own while no clip is, if it still has a transition.
    pub fn selected_transition(&self) -> Option<&Item> {
        if !self.selection.is_empty() {
            return None;
        }
        let id = self.transition_selected.as_deref()?;
        self.clip(id).filter(|clip| clip.cut_in.is_some())
    }

    /// The clip a transition verb acts on: the selected transition's, or
    /// the one selected clip's.
    fn transition_target(&self) -> Option<String> {
        if self.selected_transition().is_some() {
            return self.transition_selected.clone();
        }
        self.sole_selection()
    }

    /// Takes the transition off its clip, leaving a plain cut.
    pub fn remove_transition(&mut self) {
        let Some(item) = self.transition_target().and_then(|id| doc_view::parse(&id)) else {
            return;
        };
        self.transition_selected = None;
        self.apply(Command::UpdateClip {
            item,
            patch: ItemPatch {
                cut_in: Some(None),
                ..ItemPatch::default()
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
        let Some(item) = doc_view::parse::<ItemId>(&clip_id) else {
            return;
        };
        let Some(existing) = self.clip(&clip_id).and_then(|clip| clip.cut_in.clone()) else {
            return;
        };
        let Some(param) = Catalogue::builtin()
            .get(&existing.effect.package)
            .and_then(|package| package.manifest.param(key).cloned())
        else {
            return;
        };
        if !value.is_finite() {
            return;
        }
        let mut effect = existing.effect.clone();
        doc_view::set_param(&mut effect, key, value.clamp(param.min, param.max));
        // One undo step a drag, not one a tick of it.
        self.apply_within(
            &format!("transition-knob:{clip_id}:{key}"),
            Command::UpdateClip {
                item,
                patch: ItemPatch {
                    cut_in: Some(Some(CutIn {
                        effect,
                        len: existing.len,
                    })),
                    ..ItemPatch::default()
                },
            },
        );
    }

    /// Sets `clip_id`'s transition to `seconds`, clamped to what the cut can
    /// actually support.
    pub fn set_clip_transition_duration(&mut self, clip_id: &str, seconds: f64) {
        let (Some(item), Some(clip)) = (doc_view::parse::<ItemId>(clip_id), self.clip(clip_id))
        else {
            return;
        };
        let Some(existing) = clip.cut_in.clone() else {
            return;
        };
        let Some(duration) = self.transition_duration(clip, seconds.max(0.0)) else {
            return;
        };
        self.apply(Command::UpdateClip {
            item,
            patch: ItemPatch {
                cut_in: Some(Some(CutIn {
                    len: doc_view::len(duration),
                    ..existing
                })),
                ..ItemPatch::default()
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
