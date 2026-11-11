// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Edits from the menus and the tray.

use super::*;

impl Studio {
    // ── edits from menus and the tray ──

    /// Split at `at`: every selected clip the instant runs through, or every
    /// clip at all when nothing is selected.
    pub fn split_at(&mut self, at: f32, only_selected: bool) {
        let selection = self.selection.clone();
        let at = f64::from(at);
        let victims: Vec<String> = self
            .timeline()
            .clips
            .iter()
            .filter(|clip| {
                clip.start + f64::from(MIN_DURATION) < at
                    && at < clip.start + clip.duration - f64::from(MIN_DURATION)
                    && (!only_selected || selection.is_empty() || selection.contains(&clip.id))
                    && !self.locked(&clip.track_id)
            })
            .map(|clip| clip.id.clone())
            .collect();
        if !victims.is_empty() {
            self.apply(Command::SplitClips {
                clip_ids: victims,
                time: at,
            });
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
        let at = f64::from(self.playhead);
        let edge = f64::from(MIN_DURATION);
        let under: Vec<String> = self
            .timeline()
            .clips
            .iter()
            .filter(|clip| {
                clip.start + edge < at
                    && at < clip.start + clip.duration - edge
                    && !self.locked(&clip.track_id)
            })
            .map(|clip| clip.id.clone())
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
        let at = f64::from(self.playhead);
        let ids: Vec<String> = ids
            .iter()
            .filter(|id| {
                self.clip(id)
                    .is_some_and(|clip| !self.locked(&clip.track_id))
            })
            .cloned()
            .collect();
        let Some(command) = concat_project::delete_to_playhead(
            self.timeline(),
            &ids,
            at,
            side,
            self.closes_gap(&ids),
        ) else {
            self.notify(&t("studio.parkPlayheadInsideClip"), true);
            return;
        };
        self.flush_commit();
        self.apply(command);
        if side == PlayheadSide::Left {
            let start = ids
                .iter()
                .filter_map(|id| self.clip(id))
                .map(|clip| clip.start)
                .fold(f64::INFINITY, f64::min);
            if start.is_finite() && start < at {
                self.seek(start as f32);
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
        let spec = concat_host::preview::FrameSpec {
            time: f64::from(self.playhead),
            width,
            height,
            moving: false,
            proxy: false,
            color_space: self.project().active().video.color_space,
            canvas: self.project().active().video.canvas,
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
                if cfg!(any(target_os = "android", target_os = "ios")) {
                    return Some(out_dir.join(&suggested));
                }
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

    /// The volume line on clip `id` dragged to `gain`: written the way the
    /// inspector's knob writes it - the constant, or a key at the playhead
    /// once the volume rides - and committed as the knob's moves are, one
    /// undo step a drag. The clip need not be selected.
    pub fn clip_volume(&mut self, id: &str, gain: f32) {
        let Some(clip) = self.clip(id) else {
            return;
        };
        if self.locked(&clip.track_id) {
            return;
        }
        let at = place_in(clip, self.playhead);
        // An edit still waiting to land on other clips - words typed into a
        // title a moment ago - lands first; otherwise this drag took over
        // its echo and its targets, and the words went with them (audit
        // 2026-10-04, #9). The drag's own moves share one commit.
        if self.commit_pending && self.commit_targets != [id] {
            self.flush_commit();
        }
        self.commit_targets = vec![id.to_owned()];
        self.begin_echo();
        if let Some(clip) = self.echo_clip_mut(id) {
            write_keyable(
                clip,
                model::KeyProperty::Volume,
                f64::from(gain).clamp(0.0, 2.0),
                at,
            );
        }
        self.clip_commit();
    }

    /// A fade knob on clip `id` dragged: its fade in, or with `out` its fade
    /// out, to `seconds`, up to half the clip as the inspector's knob holds
    /// it. Committed as the volume line's drag is, one undo step a drag.
    pub fn clip_fade(&mut self, id: &str, out: bool, seconds: f32) {
        let Some(clip) = self.clip(id) else {
            return;
        };
        if self.locked(&clip.track_id) {
            return;
        }
        if self.commit_pending && self.commit_targets != [id] {
            self.flush_commit();
        }
        self.commit_targets = vec![id.to_owned()];
        self.begin_echo();
        if let Some(clip) = self.echo_clip_mut(id) {
            let seconds = f64::from(seconds).clamp(0.0, clip.duration / 2.0);
            if out {
                clip.fade_out = seconds;
            } else {
                clip.fade_in = seconds;
            }
        }
        self.clip_commit();
    }

    /// The split tool pressed clip `id` at `seconds`: one cut, on the
    /// frame nearest the press, unless it is within a frame of either end
    /// or the lane is locked.
    /// https://github.com/jub0t/Concat/issues/241
    pub fn split_clip_at(&mut self, id: &str, seconds: f32) {
        let Some(clip) = self.clip(id).cloned() else {
            return;
        };
        if self.locked(&clip.track_id) {
            return;
        }
        let fps = f64::from(self.frame_rate().max(1.0));
        let at = (f64::from(seconds) * fps).round() / fps;
        let edge = f64::from(MIN_DURATION);
        if at <= clip.start + edge || at >= clip.start + clip.duration - edge {
            return;
        }
        self.flush_commit();
        self.apply(Command::SplitClips {
            clip_ids: vec![id.to_owned()],
            time: at,
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
        let main = self.timeline().background_lane();
        !ids.is_empty()
            && ids.iter().all(|id| {
                self.clip(id)
                    .is_some_and(|clip| main == Some(clip.track_id.as_str()))
            })
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
        let doomed: Vec<String> = self
            .selection
            .iter()
            .filter(|id| {
                self.clip(id)
                    .is_some_and(|clip| !self.locked(&clip.track_id))
            })
            .cloned()
            .collect();
        if !doomed.is_empty() {
            self.apply(Command::RemoveClips {
                clip_ids: doomed,
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
        let at = f64::from(self.playhead);
        let edge = f64::from(MIN_DURATION);
        let target = self
            .menu_target
            .clone()
            .or_else(|| self.sole_selection())
            .and_then(|id| self.clip(&id).cloned())
            .filter(|clip| {
                (clip.kind == model::ClipKind::Video || clip.kind == model::ClipKind::Image)
                    && !self.locked(&clip.track_id)
                    && at > clip.start + edge
                    && at < clip.start + clip.duration - edge
            });
        let Some(clip) = target else {
            self.notify(&t("studio.parkPlayheadToFreeze"), true);
            return;
        };

        let still = if clip.kind == model::ClipKind::Video {
            let Some(media) = self
                .project()
                .media
                .iter()
                .find(|item| item.id == clip.media_id)
            else {
                return;
            };
            let Some(session) = self.session.as_ref() else {
                return;
            };
            let project_path = session.path().to_owned();
            let source_time = clip.source_start + (at - clip.start) * clip.speed;
            let frame = match media::still_at(&media.path, source_time, 1280) {
                Ok(frame) => frame,
                Err(error) => {
                    self.notify(&error, true);
                    return;
                }
            };
            let bytes = match jpeg(&frame, 4) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.notify(&format!("{error}"), true);
                    return;
                }
            };
            let key = format!(
                "freeze-{}-{}.jpg",
                clip.id,
                (source_time * 1000.0).round() as i64
            );
            if let Err(error) = media::write_artwork(&project_path, &key, &bytes) {
                self.notify(&error, true);
                return;
            }
            let path = format!("{project_path}/cache/{key}");
            match media::probe(&path) {
                Ok(summary) => Some(summary.to_new_media()),
                Err(error) => {
                    self.notify(&error, true);
                    return;
                }
            }
        } else {
            None
        };

        let created = self.apply(Command::FreezeFrame {
            clip_id: clip.id,
            time: at,
            duration: Some(1.0),
            still,
        });
        if let Some(id) = created {
            self.selection = vec![id];
        }
    }

    /// A copy of `source` laid after it. Three commands, because a clip's
    /// in-point and length are set by trims, not by placement.
    /// Duplicate every unlocked selected clip (right-to-left by start so
    /// neighbours do not stack). Falls back to the menu target when the
    /// selection is empty.
    pub fn duplicate_selected(&mut self) {
        let mut sources: Vec<Clip> = self
            .selection
            .iter()
            .filter_map(|id| self.clip(id).cloned())
            .filter(|clip| !self.locked(&clip.track_id))
            .collect();
        #[allow(clippy::collapsible_if)]
        if sources.is_empty() {
            if let Some(id) = self.menu_target.clone() {
                if let Some(clip) = self.clip(&id).cloned() {
                    if !self.locked(&clip.track_id) {
                        sources.push(clip);
                    }
                }
            }
        }
        if sources.is_empty() {
            return;
        }
        sources.sort_by(|left, right| right.start.total_cmp(&left.start));
        let mut last = None;
        for source in &sources {
            self.duplicate(source);
            last = self.selection.first().cloned();
        }
        if let Some(id) = last {
            self.selection = vec![id];
        }
    }

    /// A copy of `source` laid right after it on its own lane, selected.
    pub fn duplicate(&mut self, source: &Clip) {
        let end = source.start + source.duration;
        let track_id = source.track_id.clone();
        if let Some(created) = self.place_copy(source, &track_id, end, None) {
            self.selection = vec![created];
        }
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
        let tracks = &self.timeline().tracks;
        let mut held: Vec<Held> = ids
            .iter()
            .filter_map(|id| {
                let clip = self.clip(id)?.clone();
                let row = tracks.iter().position(|track| track.id == clip.track_id)?;
                let media = self
                    .project()
                    .media
                    .iter()
                    .find(|item| item.id == clip.media_id)
                    .cloned();
                Some(Held { row, clip, media })
            })
            .collect();
        if held.is_empty() {
            return;
        }
        held.sort_by(|left, right| {
            left.clip
                .start
                .total_cmp(&right.clip.start)
                .then(left.row.cmp(&right.row))
        });
        self.clipboard = held;
    }

    /// Paste: the held clips as a group, its shape kept, as one undo step.
    /// After `after` and on its lane - the clip a menu was opened on, or
    /// the one selected - with the rest of the group on the rows around it
    /// as they were copied; or, with nothing to follow, at the playhead on
    /// the rows they were copied from. In another timeline the rows are
    /// matched by number, which is what a paste between timelines wanted
    /// and never got: the held lane's id was the other timeline's, and the
    /// add was refused without a word (#290). The new clips are selected.
    pub fn paste_clips(&mut self, after: Option<&str>) {
        if self.clipboard.is_empty() {
            return;
        }
        let held = self.clipboard.clone();
        let first_row = held[0].row;
        let tracks: Vec<String> = self
            .timeline()
            .tracks
            .iter()
            .map(|track| track.id.clone())
            .collect();
        let (base, shift) = match after.and_then(|id| self.clip(id).cloned()) {
            Some(anchor) => {
                let row = tracks
                    .iter()
                    .position(|id| *id == anchor.track_id)
                    .unwrap_or(first_row);
                let shift =
                    isize::try_from(row).unwrap_or(0) - isize::try_from(first_row).unwrap_or(0);
                (anchor.start + anchor.duration, shift)
            }
            None => (f64::from(self.playhead), 0),
        };
        let plan = paste_plan(&held, base, shift, tracks.len(), &|row| {
            tracks.get(row).is_some_and(|id| self.locked(id))
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
            if let Some(id) = self.place_copy(&source, &tracks[row], start, Some("paste")) {
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

    /// The held clip as this project knows it: playing this project's
    /// media item for its file - the same one when it was copied here, the
    /// one with the same path when it was copied elsewhere, or the file
    /// added to the bin when this project has not got it.
    fn held_here(&mut self, held: &Held) -> Option<Clip> {
        let mut clip = held.clip.clone();
        let Some(item) = &held.media else {
            return Some(clip);
        };
        let here = self
            .project()
            .media
            .iter()
            .find(|mine| mine.path == item.path)
            .map(|mine| mine.id.clone());
        clip.media_id = match here {
            Some(id) => id,
            None => self.apply_in(
                Some("paste"),
                Command::AddMedia {
                    item: concat_project::commands::NewMedia {
                        path: item.path.clone(),
                        name: item.name.clone(),
                        duration: item.duration,
                        kind: item.kind,
                        width: item.width,
                        height: item.height,
                        frame_rate: item.frame_rate,
                        frame_rate_fraction: item.frame_rate_fraction.clone(),
                        video_codec: item.video_codec.clone(),
                        audio_codec: item.audio_codec.clone(),
                        has_audio: item.has_audio,
                        audio_tracks: item.audio_tracks.clone(),
                        origin: item.origin,
                        color_space: item.color_space,
                    },
                },
            )?,
        };
        Some(clip)
    }

    /// Lays a copy of `source` on `track_id` starting at `start` - its
    /// trims, mix, placement, speed, keys and effects with it - and says
    /// the new clip's id. `gesture` folds the commands into one undo step
    /// with whatever else names it; `None` leaves them steps of their own.
    /// Footage is first laid past the lane's last clip, then shaped by
    /// [`copy_commands`].
    fn place_copy(
        &mut self,
        source: &Clip,
        track_id: &str,
        start: f64,
        gesture: Option<&str>,
    ) -> Option<String> {
        let drawn = match source.kind {
            model::ClipKind::Text => Some(Command::AddTextClip {
                above: false,
                track_id: Some(track_id.to_owned()),
                start,
                style: source.text.clone(),
                duration: Some(source.duration),
                offset_y: Some(source.offset_y),
            }),
            model::ClipKind::Shape => Some(Command::AddShapeClip {
                above: false,
                track_id: Some(track_id.to_owned()),
                start,
                style: source.shape.clone(),
                duration: Some(source.duration),
                name: source.name.clone(),
            }),
            // A layer has no file to lay: it is laid as a layer, and the
            // match then gives it the source's whole chain. It went down
            // the file's path and was refused, so a layer could be neither
            // copied nor duplicated.
            model::ClipKind::Layer | model::ClipKind::SoundLayer => {
                let sound = source.kind == model::ClipKind::SoundLayer;
                let chain = if sound {
                    &source.filters
                } else {
                    &source.video_effects
                };
                Some(Command::AddLayerClip {
                    track_id: Some(track_id.to_owned()),
                    start,
                    duration: Some(source.duration),
                    effect_id: chain
                        .first()
                        .map(|link| link.id.clone())
                        .unwrap_or_default(),
                    name: source.name.clone(),
                    sound,
                })
            }
            // A transition clip is laid as one, its package first; the
            // match then gives it the source's knobs.
            model::ClipKind::Transition => Some(Command::AddTransitionClip {
                track_id: Some(track_id.to_owned()),
                start,
                duration: Some(source.duration),
                transition_id: source
                    .video_effects
                    .first()
                    .map(|link| link.id.clone())
                    .unwrap_or_default(),
                name: source.name.clone(),
            }),
            _ => None,
        };
        if let Some(add) = drawn {
            let created = self.apply_in(gesture, add)?;
            self.apply_in(gesture, match_command(source, &created));
            return Some(created);
        }
        let lane_end = self
            .timeline()
            .clips
            .iter()
            .filter(|clip| clip.track_id == track_id)
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max);
        let created = self.apply_in(
            gesture,
            Command::AddClip {
                media_id: source.media_id.clone(),
                track_id: track_id.to_owned(),
                start: lane_end,
                ripple: false,
            },
        )?;
        let placed = self.clip(&created).cloned()?;
        let commands = copy_commands(source, &placed, track_id, start);
        self.apply_in(gesture, Command::Batch { commands });
        Some(created)
    }

    /// What the tray's sound and word tools may do to the selection: one
    /// clip with sound for Captions to start on, one title for Speak to
    /// read. Hints, not gates - both sheets open without them.
    pub(super) fn sound_tools(&self) -> (bool, bool) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)) else {
            return (false, false);
        };
        (
            self.clip_has_sound(clip),
            clip.kind == model::ClipKind::Text,
        )
    }

    /// What the selection offers the tray's picture verbs: a picture on an
    /// unlocked lane, to mirror or turn; with one clip selected, a video or
    /// a still the playhead is inside of, to freeze there; and one video
    /// or sound clip to reverse, while no reverse is running.
    pub(super) fn transform_tools(&self) -> (bool, bool, bool) {
        let at = f64::from(self.playhead);
        let edge = f64::from(MIN_DURATION);
        let sole = self.selection.len() == 1;
        let mut picture = false;
        let mut freezable = false;
        let mut reversible = false;
        for clip in self.selection.iter().filter_map(|id| self.clip(id)) {
            if self.locked(&clip.track_id) {
                continue;
            }
            let video = clip.kind == model::ClipKind::Video;
            if sole
                && self.reverse_jobs.is_empty()
                && (video || clip.kind == model::ClipKind::Audio)
            {
                reversible = true;
            }
            if clip.kind == model::ClipKind::Audio {
                continue;
            }
            picture = true;
            if sole
                && (video || clip.kind == model::ClipKind::Image)
                && at > clip.start + edge
                && at < clip.start + clip.duration - edge
            {
                freezable = true;
            }
        }
        (picture, freezable, reversible)
    }

    /// Where a clip's sound is: whether it is a video clip whose sound is
    /// still on its picture, to take off, and whether it is a picture whose
    /// sound is off it - or that sound itself - to put back.
    pub(super) fn sound_placement(&self, clip: &Clip) -> (bool, bool) {
        let detached = self
            .timeline()
            .clips
            .iter()
            .any(|other| other.detached_from.as_deref() == Some(clip.id.as_str()));
        let video = clip.kind == model::ClipKind::Video;
        (
            video && !detached,
            (video && detached)
                || (clip.kind == model::ClipKind::Audio && clip.detached_from.is_some()),
        )
    }

    /// Whether the clip has sound to transcribe: an audio clip, or a video
    /// clip whose file carries an audio stream. A silent video is not a
    /// sound source, however much it looks like one.
    pub(crate) fn clip_has_sound(&self, clip: &Clip) -> bool {
        match clip.kind {
            model::ClipKind::Audio => true,
            model::ClipKind::Video => self
                .project()
                .media_by_id(&clip.media_id)
                .is_some_and(|media| media.has_audio),
            _ => false,
        }
    }
}
