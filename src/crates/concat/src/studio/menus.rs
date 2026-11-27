// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Edits from the menus and the tray.

use concat_doc::commands::MIN_LEN;
use concat_doc::model::TimeMap;

use super::*;

/// Which side of the playhead Delete Left and Delete Right take away.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlayheadSide {
    /// Everything before it.
    Left,
    /// Everything after it.
    Right,
}

/// Whether `at` is inside an item, at least the shortest clip from either
/// end: where a cut can be made.
fn cuts(item: &Item, at: Tick) -> bool {
    item.start + MIN_LEN < at && at + MIN_LEN < doc_view::end(item)
}

/// The cut Delete Left or Delete Right makes of each item `at` runs
/// through: a head trim up to it, or a tail trim back to it, closing the
/// lane behind with `ripple`. Later items first, so a ripple of one never
/// moves another still to be cut. `None` when `at` runs through none.
pub(super) fn delete_to_playhead(
    doc: &Doc,
    ids: &[ItemId],
    at: Tick,
    side: PlayheadSide,
    ripple: bool,
) -> Option<Command> {
    let mut cut: Vec<(ItemId, &Item)> = ids
        .iter()
        .filter_map(|id| Some((*id, doc_view::item(doc, *id)?)))
        .filter(|(_, item)| cuts(item, at))
        .collect();
    if cut.is_empty() {
        return None;
    }
    cut.sort_by_key(|(id, item)| std::cmp::Reverse((item.start, *id)));
    let commands: Vec<Command> = cut
        .into_iter()
        .map(|(id, item)| match side {
            PlayheadSide::Left => Command::TrimClip {
                item: id,
                edge: TrimEdge::Head,
                delta: at - item.start,
                ripple,
            },
            PlayheadSide::Right => Command::TrimClip {
                item: id,
                edge: TrimEdge::Tail,
                delta: at - doc_view::end(item),
                ripple,
            },
        })
        .collect();
    Some(match commands.len() {
        1 => commands.into_iter().next()?,
        _ => Command::Batch { commands },
    })
}

/// An effect as a copy holds it: its keys moved by `shift` onto the
/// copy's content time. It is given a new id by the edit it goes in, so
/// a tie to one of the source's masks cannot follow it and is let go.
fn shifted_effect(effect: &model::Effect, shift: TickDur) -> model::Effect {
    let mut effect = effect.clone();
    effect.mask = None;
    for param in effect.params.values_mut() {
        for key in &mut param.keys {
            key.at += shift;
        }
    }
    effect
}

/// A mask as a copy holds it, its keys moved by `shift`.
fn shifted_mask(mask: &model::Mask, shift: TickDur) -> model::Mask {
    let mut mask = mask.clone();
    let each = |number: &mut model::Animated<f64>| {
        for key in &mut number.keys {
            key.at += shift;
        }
    };
    for number in [
        &mut mask.x,
        &mut mask.y,
        &mut mask.scale,
        &mut mask.rotation,
        &mut mask.feather,
        &mut mask.expand,
        &mut mask.opacity,
    ] {
        each(number);
    }
    if let model::MaskSource::Shape {
        width,
        height,
        roundness,
        ..
    } = &mut mask.source
    {
        for number in [width, height, roundness] {
            each(number);
        }
    }
    mask
}

/// The edit that puts down the first piece of a copy of `source` on
/// `lane`, where nothing on it lies after: the whole of its file for a
/// cut, the title, figure, layer or transition for the rest. `None` for
/// what cannot be copied.
pub(super) fn lay_copy(doc: &Doc, source: &Item, lane: LaneId) -> Option<Command> {
    let start = doc_view::lane_end(doc, lane);
    Some(match &source.content {
        Content::Media(cut) => Command::AddClip {
            media: cut.media,
            lane,
            start,
            ripple: false,
        },
        Content::Title(title) => Command::AddTextClip {
            start,
            len: Some(source.len),
            lane: Some(lane),
            above: false,
            offset_y: Some(source.picture.offset_y.value),
            title: Some(title.clone()),
        },
        Content::Shape(style) => Command::AddShapeClip {
            start,
            len: Some(source.len),
            name: source.name.clone(),
            style: Some(style.clone()),
            lane: Some(lane),
            above: false,
        },
        Content::Layer | Content::SoundLayer => {
            let sound = matches!(source.content, Content::SoundLayer);
            let chain = if sound {
                &source.filters
            } else {
                &source.effects
            };
            Command::AddLayerClip {
                start,
                len: Some(source.len),
                package: doc_view::chain(chain).first()?.package.clone(),
                name: source.name.clone(),
                sound,
                lane: Some(lane),
            }
        }
        Content::Transition(effect) => Command::AddTransitionClip {
            start,
            len: Some(source.len),
            package: effect.package.clone(),
            name: source.name.clone(),
            lane: Some(lane),
        },
        Content::Nest(_) | Content::Unknown { .. } => return None,
    })
}

/// The edits, made in order as one batch, that make `laid` - just put
/// down by [`lay_copy`] with nothing after it on its lane - into a copy of
/// `source` on `lane` at `at`: its footage and its pace, its place, and
/// then everything else it has.
///
/// The footage is cut at the laid clip's own pace, the tail before the
/// head so the head never meets the shortest-clip limit; then the clip is
/// retimed, which works the length out from the footage; and then it is
/// moved, to the free start on the lane nearest `at`, so no other clip
/// moves or is cut. Every edit is plain, never magnetic. The keys follow
/// onto the copy's own content time.
pub(super) fn shape_copy(
    doc: &Doc,
    source: &Item,
    laid_id: ItemId,
    lane: LaneId,
    at: Tick,
) -> Vec<Command> {
    let Some(laid) = doc_view::item(doc, laid_id) else {
        return Vec::new();
    };
    let mut edits = Vec::new();
    let trim = |edge: TrimEdge, delta: TickDur| Command::TrimClip {
        item: laid_id,
        edge,
        delta,
        ripple: false,
    };
    // Where the copy's content time will start: what its window's offset
    // comes to once the head is cut.
    let mut offset = laid.offset;
    let still = doc_view::kind(doc, source) == Kind::Image;
    match (&source.content, still) {
        (Content::Media(_), false) => {
            if let (Some((from, span)), Some((laid_from, laid_span))) =
                (doc_view::covered(source), doc_view::covered(laid))
            {
                let off_the_tail = (from + span) - (laid_from + laid_span);
                if off_the_tail != TickDur::ZERO {
                    edits.push(trim(TrimEdge::Tail, off_the_tail));
                }
                let off_the_head = from - laid_from;
                if off_the_head != TickDur::ZERO {
                    edits.push(trim(TrimEdge::Head, off_the_head));
                    offset += off_the_head;
                }
            }
            match doc_view::time_map(source) {
                Some(TimeMap::Keyed { .. }) => {
                    if let Some(curve) = concat_doc::commands::speed_graph(source) {
                        edits.push(Command::SetClipSpeedCurve {
                            item: laid_id,
                            curve: Some(curve),
                            ripple: false,
                        });
                    }
                }
                Some(TimeMap::Linear { speed, .. }) if *speed != model::Speed::NORMAL => {
                    edits.push(Command::SetClipSpeed {
                        item: laid_id,
                        speed: *speed,
                        ripple: false,
                    });
                }
                _ => {}
            }
        }
        // A still has no footage to cut: it is held exactly as long.
        _ => {
            let longer = source.len.dur() - laid.len.dur();
            if longer != TickDur::ZERO {
                edits.push(trim(TrimEdge::Tail, longer));
            }
        }
    }
    let start = doc_view::nearest_free_start(doc, lane, at, source.len, &[laid_id]);
    edits.push(Command::MoveClips {
        moves: vec![ClipMove {
            item: laid_id,
            start,
            lane: Some(lane),
        }],
    });
    edits.extend(match_commands(source, laid_id, offset - source.offset));
    edits
}

/// The edits that take `copy` - a whole copy of `source` the document
/// made beside it - to `lane` at the free start nearest `at`, named as a
/// copy unless it is a title, whose name follows its words.
pub(super) fn move_copy(
    doc: &Doc,
    source: &Item,
    copy: ItemId,
    lane: LaneId,
    at: Tick,
) -> Vec<Command> {
    let start = doc_view::nearest_free_start(doc, lane, at, source.len, &[copy]);
    let mut edits = vec![Command::MoveClips {
        moves: vec![ClipMove {
            item: copy,
            start,
            lane: Some(lane),
        }],
    }];
    if doc_view::title(source).is_none() {
        edits.push(Command::UpdateClip {
            item: copy,
            patch: ItemPatch {
                name: Some(format!("{} copy", source.name)),
                ..ItemPatch::default()
            },
        });
    }
    edits
}

/// The edits that make `copy` like `source` in everything but its place
/// and its footage, its keys moved by `shift` onto its own content time.
/// The whole of it goes, not a list of fields: masks, cutouts,
/// stabilizing, voices, backgrounds and keys included. A title's name
/// follows its words; anything else says it is a copy. Two things stay
/// behind: the transition on the cut into the source, and a tie to what
/// its sound was detached from.
pub(super) fn match_commands(source: &Item, copy: ItemId, shift: TickDur) -> Vec<Command> {
    let picture = &source.picture;
    let mut edits = vec![Command::SetClipTransform {
        item: copy,
        offset_x: Some(picture.offset_x.value),
        offset_y: Some(picture.offset_y.value),
        rotation: Some(picture.rotation.value),
        scale: Some(picture.scale.value),
        stretch_x: Some(picture.stretch_x.value),
        stretch_y: Some(picture.stretch_y.value),
    }];
    let title = doc_view::title(source).cloned();
    let patch = ItemPatch {
        stream: match &source.content {
            Content::Media(cut) => Some(cut.stream),
            _ => None,
        },
        background: Some(picture.background.clone()),
        blend: Some(picture.blend.clone()),
        crop: Some(picture.crop),
        fade_in: Some(source.sound.fade_in),
        fade_out: Some(source.sound.fade_out),
        filters: Some(
            doc_view::chain(&source.filters)
                .into_iter()
                .map(|effect| shifted_effect(effect, shift))
                .collect(),
        ),
        effects: Some(
            doc_view::chain(&source.effects)
                .into_iter()
                .map(|effect| shifted_effect(effect, shift))
                .collect(),
        ),
        flip_h: Some(picture.flip_h),
        flip_v: Some(picture.flip_v),
        masks: Some(
            doc_view::masks(&source.masks)
                .into_iter()
                .map(|mask| shifted_mask(mask, shift))
                .collect(),
        ),
        motion: Some(picture.motion.clone()),
        muted: Some(source.sound.muted),
        name: title.is_none().then(|| format!("{} copy", source.name)),
        opacity: Some(picture.opacity.value),
        keep_pitch: Some(source.sound.keep_pitch),
        shape: doc_view::shape(source).cloned(),
        title,
        cut_in: Some(None),
        volume: Some(source.sound.volume.value),
    };
    edits.push(Command::UpdateClip { item: copy, patch });
    if picture.cutout.is_some() {
        edits.push(Command::SetClipCutout {
            item: copy,
            cutout: picture.cutout.clone(),
        });
    }
    if picture.stabilize.is_some() {
        edits.push(Command::SetClipStabilize {
            item: copy,
            stabilize: picture.stabilize.clone(),
        });
    }
    if source.sound.voice.is_some() {
        edits.push(Command::SetClipVoice {
            item: copy,
            voice: source.sound.voice.clone(),
        });
    }
    for property in doc_view::KEY_PROPERTIES {
        for key in &doc_view::animated(source, property).keys {
            edits.push(Command::SetClipKey {
                item: copy,
                property,
                at: key.at + shift,
                value: key.value,
                ease: key.ease.clone(),
            });
        }
    }
    edits
}

impl Studio {
    // ── edits from menus and the tray ──

    /// Split at `at`: every selected clip the instant runs through, or every
    /// clip at all when nothing is selected.
    pub fn split_at(&mut self, at: f32, only_selected: bool) {
        let at = doc_view::tick(f64::from(at));
        let victims: Vec<ItemId> = doc_view::items(self.doc())
            .into_iter()
            .filter(|(id, item)| {
                cuts(item, at)
                    && (!only_selected
                        || self.selection.is_empty()
                        || self.selection.contains(&id.to_string()))
                    && !self.lane_locked(item.lane)
            })
            .map(|(id, _)| id)
            .collect();
        if !victims.is_empty() {
            self.apply(Command::SplitClips { items: victims, at });
        }
    }

    /// Delete Left / Delete Right, Q and W: the selected clips the
    /// playhead runs through lose everything before it, or everything
    /// after it, as one undo step. With nothing selected, the one clip
    /// under the playhead is cut; with several there and none chosen, a
    /// notice asks for one rather than guessing - a caption over the
    /// picture is as much "the clip" as the picture is. The lane closes
    /// behind the cut only on a magnetic timeline, as any trim's does.
    /// https://github.com/jub0t/Concat/issues/400
    pub fn delete_to_playhead(&mut self, side: PlayheadSide) {
        let at = doc_view::tick(f64::from(self.playhead));
        let under: Vec<String> = doc_view::items(self.doc())
            .into_iter()
            .filter(|(_, item)| cuts(item, at) && !self.lane_locked(item.lane))
            .map(|(id, _)| id.to_string())
            .collect();
        let targets: Vec<String> = if self.selection.is_empty() {
            if under.len() > 1 {
                self.notify(&t("studio.selectClipToCut"), true);
                return;
            }
            under
        } else {
            under
                .into_iter()
                .filter(|id| self.selection.contains(id))
                .collect()
        };
        self.delete_clips_to_playhead(&targets, side);
    }

    /// The cut itself, on these clips; see [`Self::delete_to_playhead`].
    /// A Delete Left ends with the playhead on the frame it was showing,
    /// which a magnetic cut has moved back to the clip's start.
    pub(super) fn delete_clips_to_playhead(&mut self, ids: &[String], side: PlayheadSide) {
        let at = doc_view::tick(f64::from(self.playhead));
        let ids: Vec<String> = ids
            .iter()
            .filter(|id| {
                self.clip(id)
                    .is_some_and(|clip| !self.lane_locked(clip.lane))
            })
            .cloned()
            .collect();
        let typed: Vec<ItemId> = ids.iter().filter_map(|id| doc_view::parse(id)).collect();
        let Some(command) = delete_to_playhead(self.doc(), &typed, at, side, self.closes_gap(&ids))
        else {
            self.notify(&t("studio.parkPlayheadInsideClip"), true);
            return;
        };
        self.flush_commit();
        let command = doc_view::sweep_strays(&self.document, command, &typed);
        self.apply(command);
        if side == PlayheadSide::Left {
            let start = ids
                .iter()
                .filter_map(|id| self.clip(id))
                .map(|clip| clip.start)
                .min();
            if let Some(start) = start
                && start < at
            {
                self.seek(doc_view::secs(start) as f32);
            }
        }
    }

    /// Save Video Frame As: the frame under the playhead, drawn at the
    /// output's full size rather than the monitor's, written as a PNG
    /// where the person says - offered in the project's frames folder -
    /// and put in the bin, selected, so it can go straight onto a lane as
    /// a freeze frame. The dialog opens on the next turn of the loop, as
    /// every dialog reached from a right-click menu must.
    /// https://github.com/jub0t/Concat/issues/238
    pub fn save_frame_as(&mut self) {
        if self.playing {
            self.pause();
        }
        let Some((clips, settings)) = self.preview_clips() else {
            return;
        };
        let Some(project_dir) = self
            .session
            .as_ref()
            .map(|session| std::path::PathBuf::from(session.path()))
        else {
            return;
        };
        let (width, height) = self.output_size();
        let frame = doc_view::sequence(self.doc()).map(|sequence| sequence.frame.clone());
        let (color_space, canvas) =
            frame.map_or((concat_export::ColorSpace::Sdr, [0, 0, 0, 255]), |f| {
                (
                    concat_export::ColorSpace::of(&f.color),
                    [f.canvas.r, f.canvas.g, f.canvas.b, f.canvas.a],
                )
            });
        let spec = concat_host::preview::FrameSpec {
            time: f64::from(self.playhead),
            width,
            height,
            moving: false,
            proxy: false,
            color_space,
            canvas,
            nearest: false,
        };
        let monitor = self.host.monitor.clone();
        let out_dir = project_dir.join("frames");
        let suggested = format!(
            "frame-{}.png",
            frames_timecode(self.playhead, self.frame_rate()).replace(':', ".")
        );
        let title = t("lib.saveFrame");
        let family = t("lib.pngImage");
        // With the studio unborrowed too; see `host::after_dialog`.
        crate::host::after_dialog(
            move || {
                let chosen = crate::platform::save_file(
                    &title,
                    &out_dir,
                    &suggested,
                    (family.as_str(), &["png"]),
                )?;
                Some(
                    if chosen
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
                    {
                        chosen
                    } else {
                        chosen.with_extension("png")
                    },
                )
            },
            move |studio, _, _, file| {
                let written = file.to_string_lossy().into_owned();
                studio.notify(&t("studio.savingFrame"), false);
                spawn_in_project(
                    move || -> Result<concat_host::media::MediaSummary, String> {
                        let pixels = monitor.frame(clips, &settings, spec)?;
                        let image = image::RgbaImage::from_raw(width, height, pixels)
                            .ok_or_else(|| "the frame came back the wrong size".to_owned())?;
                        if let Some(parent) = file.parent() {
                            std::fs::create_dir_all(parent).map_err(|error| {
                                format!("could not create {}: {error}", parent.display())
                            })?;
                        }
                        image.save(&file).map_err(|error| error.to_string())?;
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
                            studio.media.filter = MediaFilter::All;
                            studio.media.selected.clear();
                            if let Some(id) = minted {
                                studio.media.selected.insert(id);
                            }
                            studio.media_jump += 1;
                            studio.notify(&tf("studio.frameSaved", &[&written]), false);
                        }
                        Err(error) => {
                            log::warn!(
                                "export: saving the frame failed: {error}; it is not in the bin"
                            );
                            studio.notify(&tf("studio.couldNotSaveFrame", &[&error]), true);
                        }
                    },
                );
            },
        );
    }

    /// The file of the bin at a path, if the bin has it.
    pub(super) fn media_at_path(&self, path: &str) -> Option<MediaId> {
        doc_view::bin(self.doc())
            .find(|(_, media)| media.path == path)
            .map(|(id, _)| id)
    }

    /// The volume line on clip `id` dragged to `gain`: written the way the
    /// inspector's knob writes it - the constant, or a key at the playhead
    /// once the volume rides - and committed as the knob's moves are, one
    /// undo step a drag. The clip need not be selected.
    pub fn clip_volume(&mut self, id: &str, gain: f32) {
        let Some(clip) = self.clip(id) else {
            return;
        };
        if self.lane_locked(clip.lane) {
            return;
        }
        let at = doc_view::content_at(clip, doc_view::tick(f64::from(self.playhead)));
        // An edit still waiting to land on other clips - words typed into a
        // title a moment ago - lands first; otherwise this drag took over
        // its echo and its targets, and the words went with them (audit
        // 2026-10-04, #9). The drag's own moves share one commit.
        if self.commit_pending && self.commit_targets != [id] {
            self.flush_commit();
        }
        self.commit_targets = vec![id.to_owned()];
        self.begin_echo();
        self.echo_edit(id, |clip| {
            doc_view::write_at(
                doc_view::animated_mut(clip, KeyProperty::Volume),
                f64::from(gain).clamp(0.0, 2.0),
                at,
            );
        });
        self.clip_commit();
    }

    /// A fade knob on clip `id` dragged: its fade in, or with `out` its fade
    /// out, to `seconds`, up to half the clip as the inspector's knob holds
    /// it. Committed as the volume line's drag is, one undo step a drag.
    pub fn clip_fade(&mut self, id: &str, out: bool, seconds: f32) {
        let Some(clip) = self.clip(id) else {
            return;
        };
        if self.lane_locked(clip.lane) {
            return;
        }
        if self.commit_pending && self.commit_targets != [id] {
            self.flush_commit();
        }
        self.commit_targets = vec![id.to_owned()];
        self.begin_echo();
        self.echo_edit(id, |clip| {
            let half = TickDur::new(clip.len.get() / 2);
            let fade = doc_view::dur(f64::from(seconds)).clamp(TickDur::ZERO, half);
            if out {
                clip.sound.fade_out = fade;
            } else {
                clip.sound.fade_in = fade;
            }
        });
        self.clip_commit();
    }

    /// The split tool pressed clip `id` at `seconds`: one cut, on the
    /// frame nearest the press, unless it is within a frame of either end
    /// or the lane is locked.
    /// https://github.com/jub0t/Concat/issues/241
    pub fn split_clip_at(&mut self, id: &str, seconds: f32) {
        let (Some(item), Some(clip)) = (doc_view::parse::<ItemId>(id), self.clip(id)) else {
            return;
        };
        if self.lane_locked(clip.lane) {
            return;
        }
        let pressed = doc_view::tick(f64::from(seconds));
        let at = doc_view::sequence(self.doc())
            .and_then(|sequence| sequence.frame.rate.snap(pressed, concat_doc::Snap::Nearest))
            .unwrap_or(pressed);
        if !cuts(clip, at) {
            return;
        }
        self.flush_commit();
        self.apply(Command::SplitClips {
            items: vec![item],
            at,
        });
    }

    /// Delete: the selection goes, and the hole stays unless the timeline
    /// is magnetic.
    pub fn delete_selected(&mut self) {
        // A selected transition goes on its own, leaving a plain cut.
        if self.selected_transition().is_some() {
            self.remove_transition();
            return;
        }
        let magnetic = self.closes_gap(&self.selection);
        self.remove_selected(magnetic);
    }

    /// Whether removing these clips closes the gap they leave: always with
    /// the magnetic timeline on, and on a phone, until the person sets it,
    /// when every one of them is on the main picture's lane - the main
    /// track, where a Split and Delete is done (#497). Titles, captions and
    /// overlays keep their time there: a caption deleted must not pull the
    /// ones after it off the words they were timed to.
    pub(super) fn closes_gap(&self, ids: &[String]) -> bool {
        if self.prefs.magnetic {
            return true;
        }
        if !self.prefs.magnetic() {
            return false;
        }
        let main = doc_view::picture_lane(self.doc());
        !ids.is_empty()
            && ids
                .iter()
                .all(|id| self.clip(id).is_some_and(|clip| main == Some(clip.lane)))
    }

    /// Ripple delete (⇧⌫): the selection goes and each lane closes behind
    /// it, so a rough cut needs no dragging-left after every deletion.
    /// https://github.com/jub0t/Concat/issues/106
    pub fn ripple_delete_selected(&mut self) {
        if self.selected_transition().is_some() {
            self.remove_transition();
            return;
        }
        self.remove_selected(true);
    }

    pub(super) fn remove_selected(&mut self, ripple: bool) {
        let doomed: Vec<ItemId> = self
            .selection
            .iter()
            .filter(|id| {
                self.clip(id)
                    .is_some_and(|clip| !self.lane_locked(clip.lane))
            })
            .filter_map(|id| doc_view::parse(id))
            .collect();
        if !doomed.is_empty() {
            self.apply(Command::RemoveClips {
                items: doomed,
                ripple,
            });
        }
        self.selection.clear();
    }

    /// A freeze frame at the playhead on a picture clip.
    ///
    /// Video extracts a JPEG still into the project cache; images reuse their
    /// media. The engine command splits the clip, inserts the hold, and
    /// ripples the rest of the lane.
    pub fn freeze_at_playhead(&mut self) {
        let at = doc_view::tick(f64::from(self.playhead));
        let target = self
            .menu_target
            .clone()
            .or_else(|| self.sole_selection())
            .filter(|id| {
                self.clip(id).is_some_and(|clip| {
                    matches!(self.kind(clip), Kind::Video | Kind::Image)
                        && !self.lane_locked(clip.lane)
                        && cuts(clip, at)
                })
            });
        let Some((id, clip)) = target.and_then(|id| {
            let clip = self.clip(&id)?.clone();
            Some((doc_view::parse::<ItemId>(&id)?, clip))
        }) else {
            self.notify(&t("studio.parkPlayheadToFreeze"), true);
            return;
        };

        let still = if self.kind(&clip) == Kind::Video {
            let (Some(media), Some(session)) = (
                doc_view::media_of(self.doc(), &clip).cloned(),
                self.session.as_ref(),
            ) else {
                return;
            };
            let source_time = doc_view::source_second(&clip, doc_view::secs(at));
            match media::freeze_still(session.path(), &media.path, &id.to_string(), source_time) {
                Ok(still) => Some(still),
                Err(error) => {
                    self.notify(&error, true);
                    return;
                }
            }
        } else {
            None
        };

        let created = self.apply_making(Command::FreezeFrame {
            item: id,
            at,
            len: Some(doc_view::len(1.0)),
            still,
        });
        if let Some(id) = created {
            self.selection = vec![id];
        }
    }

    /// Duplicate: a copy of every selected clip on an unlocked lane - or of
    /// the clip the menu was opened on, when nothing is selected - laid
    /// right after its own end on its own lane, or in the first gap from
    /// there, the latest first, so a copy never lands on a source still
    /// waiting its turn. Each says it is a copy by its name. The lot is one
    /// undo step, and the copy of the earliest source is left selected;
    /// when no copy could be made the selection stays as it was.
    pub fn duplicate_selected(&mut self) {
        let mut sources: Vec<String> = self
            .selection
            .iter()
            .filter(|id| {
                self.clip(id)
                    .is_some_and(|clip| !self.lane_locked(clip.lane))
            })
            .cloned()
            .collect();
        if sources.is_empty()
            && let Some(id) = self.menu_target.clone()
            && self
                .clip(&id)
                .is_some_and(|clip| !self.lane_locked(clip.lane))
        {
            sources.push(id);
        }
        let items: Vec<ItemId> = sources
            .iter()
            .filter_map(|id| doc_view::parse(id))
            .collect();
        if items.is_empty() {
            return;
        }
        self.flush_commit();
        let before = self.selection.clone();
        if self
            .apply_within("duplicate", Command::DuplicateClips { items })
            .is_none()
        {
            self.selection = before;
            if let Some(session) = self.session.as_mut() {
                session.end_gesture();
            }
            return;
        }
        // The copies, earliest first, as the edit selects them.
        let copies = self.selection.clone();
        let renames: Vec<Command> = copies
            .iter()
            .filter_map(|id| {
                let copy = self.clip(id)?;
                if doc_view::title(copy).is_some() {
                    return None;
                }
                Some(Command::UpdateClip {
                    item: doc_view::parse(id)?,
                    patch: ItemPatch {
                        name: Some(format!("{} copy", copy.name)),
                        ..ItemPatch::default()
                    },
                })
            })
            .collect();
        if !renames.is_empty() {
            self.apply_within("duplicate", Command::Batch { commands: renames });
        }
        // One step for the lot, and the next Duplicate a step of its own.
        if let Some(session) = self.session.as_mut() {
            session.end_gesture();
        }
        self.selection = copies.into_iter().take(1).collect();
    }

    /// Copy: the selected clips, or `anchor` alone when it is not among
    /// them - the clip a menu was opened on. Each is held with the row of
    /// its lane, so a paste into another timeline lands it on the same row
    /// there; the media is the project's, shared by every timeline. The
    /// held clips are kept in order of their start, the earliest first.
    pub fn copy_clips(&mut self, anchor: Option<&str>) {
        let mut ids: Vec<String> = self.selection.clone();
        if let Some(anchor) = anchor
            && !ids.iter().any(|id| id == anchor)
        {
            ids = vec![anchor.to_owned()];
        }
        let lanes = doc_view::lanes(self.doc());
        let mut held: Vec<Held> = ids
            .iter()
            .filter_map(|id| {
                let clip = self.clip(id)?.clone();
                let row = lanes.iter().position(|lane| *lane == clip.lane)?;
                let media = doc_view::media_of(self.doc(), &clip).cloned();
                Some(Held { row, clip, media })
            })
            .collect();
        if held.is_empty() {
            return;
        }
        held.sort_by(|left, right| {
            left.clip
                .start
                .cmp(&right.clip.start)
                .then(left.row.cmp(&right.row))
        });
        self.clipboard = held;
    }

    /// Paste: the held clips as a group, its shape kept, as one undo step.
    /// After `after` and on its lane - the clip a menu was opened on, or
    /// the one selected - with the rest of the group on the rows around it
    /// as they were copied; or, with nothing to follow, at the playhead on
    /// the rows they were copied from. In another timeline the rows are
    /// matched by number (#290). The new clips are selected.
    pub fn paste_clips(&mut self, after: Option<&str>) {
        if self.clipboard.is_empty() {
            return;
        }
        let held = self.clipboard.clone();
        let first_row = held[0].row;
        let lanes = doc_view::lanes(self.doc());
        let (base, shift) = match after.and_then(|id| self.clip(id).cloned()) {
            Some(anchor) => {
                let row = lanes
                    .iter()
                    .position(|lane| *lane == anchor.lane)
                    .unwrap_or(first_row);
                let shift =
                    isize::try_from(row).unwrap_or(0) - isize::try_from(first_row).unwrap_or(0);
                (doc_view::end(&anchor), shift)
            }
            None => (doc_view::tick(f64::from(self.playhead)), 0),
        };
        let plan = paste_plan(&held, base, shift, lanes.len(), &|row| {
            lanes.get(row).is_some_and(|lane| self.lane_locked(*lane))
        });
        self.flush_commit();
        let mut created = Vec::new();
        for (held, place) in held.iter().zip(plan) {
            let Some((row, start)) = place else {
                continue;
            };
            let Some(source) = self.held_here(held) else {
                continue;
            };
            if let Some(id) = self.place_copy(&source, lanes[row], start, Some("paste")) {
                created.push(id);
            }
        }
        // One step for the lot, and the next paste a step of its own.
        if let Some(session) = self.session.as_mut() {
            session.end_gesture();
        }
        if !created.is_empty() {
            self.selection = created;
        }
    }

    /// The held clip as this project knows it: cut from this project's
    /// file for its path - the same one when it was copied here, the one
    /// with the same path when it was copied elsewhere, or the file added
    /// to the bin when this project has not got it. What it names that the
    /// project has not got - an enhanced copy, a background still - is let
    /// go.
    fn held_here(&mut self, held: &Held) -> Option<Item> {
        let mut clip = held.clip.clone();
        if let Some(file) = &held.media {
            let here = match self.media_at_path(&file.path) {
                Some(id) => id,
                None => self
                    .apply_in(
                        Some("paste"),
                        Command::AddMedia {
                            media: file.clone(),
                        },
                    )?
                    .typed::<MediaId>()?,
            };
            if let Content::Media(cut) = &mut clip.content {
                cut.media = here;
            }
        }
        let doc = self.doc();
        if let Content::Media(cut) = &mut clip.content
            && cut
                .enhanced
                .as_ref()
                .is_some_and(|enhanced| doc_view::media(doc, enhanced.copy).is_none())
        {
            cut.enhanced = None;
        }
        if clip
            .picture
            .background
            .as_ref()
            .and_then(model::Background::media)
            .is_some_and(|still| doc_view::media(doc, still).is_none())
        {
            clip.picture.background = None;
        }
        Some(clip)
    }

    /// Lays a copy of `source` on `lane` starting at `start` - its trims,
    /// mix, placement, speed, keys and effects with it - and says the new
    /// clip's id. A copy shows the same stretch of its file at the same
    /// pace; a start that would cover a clip becomes the nearest one on the
    /// lane where the whole copy fits, and no other clip moves.
    ///
    /// A source that is on this timeline as it was held is copied whole by
    /// the document, keys, curve and all, then moved into place; anything
    /// else is laid and shaped (see [`lay_copy`] and [`shape_copy`]).
    /// `gesture` folds the edits into one undo step with whatever else
    /// names it; `None` leaves them steps of their own. A refused edit
    /// shows the reason, leaves no clip behind and says `None`.
    fn place_copy(
        &mut self,
        source: &Item,
        lane: LaneId,
        start: Tick,
        gesture: Option<&str>,
    ) -> Option<String> {
        let same_sequence = doc_view::sequence_of(self.doc(), source)
            == doc_view::lane(self.doc(), lane).map(|lane| lane.sequence);
        let whole = doc_view::id_of(self.doc(), source).filter(|_| same_sequence);
        if let Some(original) = whole {
            let copy = made_item(self.apply_in(
                gesture,
                Command::DuplicateClips {
                    items: vec![original],
                },
            ))?;
            let copy_id = doc_view::parse::<ItemId>(&copy)?;
            let edits = move_copy(self.doc(), source, copy_id, lane, start);
            self.apply_in(gesture, Command::Batch { commands: edits });
            return Some(copy);
        }
        let lay = lay_copy(self.doc(), source, lane)?;
        let created = made_item(self.apply_in(gesture, lay))?;
        let laid = doc_view::parse::<ItemId>(&created)?;
        let shaping = shape_copy(self.doc(), source, laid, lane, start);
        let before = self.revision;
        self.apply_in(gesture, Command::Batch { commands: shaping });
        if self.revision == before {
            // The shaping was refused (and the reason shown): the whole
            // file is not left lying at the lane's end as if it were the
            // copy.
            self.apply_in(
                gesture,
                Command::RemoveClips {
                    items: vec![laid],
                    ripple: false,
                },
            );
            return None;
        }
        Some(created)
    }

    /// What the tray's sound and word tools may do to the selection: one
    /// clip with sound for Captions to start on, one title for Speak to
    /// read. Hints, not gates - both sheets open without them.
    pub(super) fn sound_tools(&self) -> (bool, bool) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return (false, false);
        };
        (self.clip_has_sound(clip), self.kind(clip) == Kind::Text)
    }

    /// What the selection offers the tray's picture verbs: a picture on an
    /// unlocked lane, to mirror or turn; with one clip selected, a video or
    /// a still the playhead is inside of, to freeze there; and one video
    /// or sound clip to reverse, while no reverse is running.
    pub(super) fn transform_tools(&self) -> (bool, bool, bool) {
        let at = doc_view::tick(f64::from(self.playhead));
        let sole = self.selection.len() == 1;
        let mut picture = false;
        let mut freezable = false;
        let mut reversible = false;
        for clip in self.selection.iter().filter_map(|id| self.clip(id)) {
            if self.lane_locked(clip.lane) {
                continue;
            }
            let kind = self.kind(clip);
            let video = kind == Kind::Video;
            if sole && self.reverse_jobs.is_empty() && (video || kind == Kind::Audio) {
                reversible = true;
            }
            if kind == Kind::Audio {
                continue;
            }
            picture = true;
            if sole && (video || kind == Kind::Image) && cuts(clip, at) {
                freezable = true;
            }
        }
        (picture, freezable, reversible)
    }

    /// Where a clip's sound is: whether it is a video clip whose sound is
    /// still on its picture, to take off, and whether it is a picture whose
    /// sound is off it - or that sound itself - to put back.
    pub(super) fn sound_placement(&self, clip: &Item) -> (bool, bool) {
        let detached =
            doc_view::id_of(self.doc(), clip).is_some_and(|id| !self.detached_from(id).is_empty());
        let kind = self.kind(clip);
        let video = kind == Kind::Video;
        let tied = clip
            .link
            .as_ref()
            .is_some_and(|link| link.kind == model::LinkKind::SoundOf);
        (
            video && !detached,
            (video && detached) || (kind == Kind::Audio && tied),
        )
    }

    /// Whether the clip has sound to transcribe: an audio clip, or a video
    /// clip whose file carries an audio stream. A silent video is not a
    /// sound source, however much it looks like one.
    pub(crate) fn clip_has_sound(&self, clip: &Item) -> bool {
        match self.kind(clip) {
            Kind::Audio => true,
            Kind::Video => doc_view::has_sound(self.doc(), clip),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc_view::bench::Bench;
    use concat_doc::model::MediaKind;

    #[test]
    fn delete_left_trims_each_head_up_to_the_playhead_latest_first() {
        let mut bench = Bench::new();
        let lane = bench.main_lane();
        let media = bench.add_media("a.mov", MediaKind::Video, 4.0);
        let first = bench.add_clip(media, lane, 0.0);
        let second = bench.add_clip(media, lane, 4.0);
        let at = doc_view::tick(2.0);
        let command =
            delete_to_playhead(bench.doc(), &[first, second], at, PlayheadSide::Left, false);
        // Only the first runs through the playhead.
        match command {
            Some(Command::TrimClip {
                item, edge, delta, ..
            }) => {
                assert_eq!(item, first);
                assert_eq!(edge, TrimEdge::Head);
                assert_eq!(delta, at - Tick::ZERO);
            }
            other => panic!("cut as {other:?}"),
        }
        bench.apply(&command_or_panic(delete_to_playhead(
            bench.doc(),
            &[first],
            at,
            PlayheadSide::Right,
            true,
        )));
        assert_eq!(
            bench.doc().items.get(first).unwrap().len,
            doc_view::len(2.0)
        );
        // Off the ends, nothing is cut: the second clip closed up behind
        // the first, and the playhead is at its head.
        let head = bench.doc().items.get(second).unwrap().start;
        assert_eq!(head, at);
        assert!(
            delete_to_playhead(bench.doc(), &[second], head, PlayheadSide::Left, false).is_none()
        );
    }

    fn command_or_panic(command: Option<Command>) -> Command {
        command.expect("a cut")
    }

    #[test]
    fn a_stranded_transition_is_swept_with_the_move_that_strands_it() {
        let mut bench = Bench::new();
        let lane = bench.main_lane();
        let media = bench.add_media("a.mov", MediaKind::Video, 4.0);
        let clip = bench.add_clip(media, lane, 0.0);
        let over = bench.lane(0);
        let transition = bench
            .apply(&Command::AddTransitionClip {
                start: doc_view::tick(1.0),
                len: Some(doc_view::len(1.0)),
                package: "concat.cross-fade".to_owned(),
                name: "Fade".to_owned(),
                lane: Some(over),
            })
            .created
            .and_then(|id| id.typed::<ItemId>())
            .expect("a transition");
        let away = Command::MoveClips {
            moves: vec![ClipMove {
                item: clip,
                start: doc_view::tick(10.0),
                lane: None,
            }],
        };
        assert!(bench.try_apply(&away).is_err(), "the move alone is refused");
        let swept = doc_view::sweep_strays(&bench.document, away, &[clip]);
        bench.apply(&swept);
        assert!(bench.doc().items.get(transition).is_none());
        assert_eq!(
            bench.doc().items.get(clip).unwrap().start,
            doc_view::tick(10.0)
        );
    }
}
