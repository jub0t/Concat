// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Effect chains, and the effect libraries they draw from.

use super::*;

impl Studio {
    // ── the chains ──
    //
    // The picture's chain and the sound's, edited by index from the
    // inspector's stacks. Adding, removing, reordering and bypassing are
    // each one command; a knob is a stream and goes through the echo, the
    // way every other inspector control does.

    /// One edit to the selected clips' chains, as one command. `links` are
    /// rows of the chain the inspector shows - the first clip's - and each
    /// other clip of a batch is handed its own rows for the same links (see
    /// [`peer_link`]), or passed by when it lacks one. `edit` returns false
    /// to say it changed nothing.
    fn chain_edit(
        &mut self,
        audio: bool,
        links: &[i32],
        edit: impl Fn(&mut Vec<AppliedFilter>, &[usize]) -> bool,
    ) {
        let ids = self.edit_targets();
        let chain_of = |clip: &Clip| {
            if audio {
                clip.filters.clone()
            } else {
                clip.video_effects.clone()
            }
        };
        let Some(shown) = ids.first().and_then(|id| self.clip(id)).map(chain_of) else {
            return;
        };
        let Some(rows) = links
            .iter()
            .map(|&row| usize::try_from(row).ok().filter(|&row| row < shown.len()))
            .collect::<Option<Vec<usize>>>()
        else {
            return;
        };
        let mut commands = Vec::new();
        for clip_id in &ids {
            let Some(clip) = self.clip(clip_id) else {
                continue;
            };
            let mut chain = chain_of(clip);
            let Some(own) = rows
                .iter()
                .map(|&row| peer_link(&shown, row, &chain))
                .collect::<Option<Vec<usize>>>()
            else {
                continue;
            };
            if !edit(&mut chain, &own) {
                continue;
            }
            let patch = if audio {
                ClipPatch {
                    filters: Some(chain),
                    ..ClipPatch::default()
                }
            } else {
                ClipPatch {
                    video_effects: Some(chain),
                    ..ClipPatch::default()
                }
            };
            commands.push(Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch,
            });
        }
        let command = match commands.len() {
            0 => return,
            1 => commands.remove(0),
            _ => Command::Batch { commands },
        };
        self.apply(command);
    }

    pub fn chain_add(&mut self, audio: bool, id: &str) {
        // Added from the chain in view: stay there, unlike a library card (#462).
        let jump = self.inspector_jump;
        self.apply_catalogue(id, !audio);
        self.inspector_jump = jump;
    }

    pub fn chain_toggle(&mut self, audio: bool, index: i32) {
        // The link shown sets the state for all of a batch, so a batch
        // whose links disagree comes out agreeing rather than flipped.
        let on = self
            .edit_targets()
            .first()
            .and_then(|id| self.clip(id))
            .and_then(|clip| {
                let chain = if audio {
                    &clip.filters
                } else {
                    &clip.video_effects
                };
                usize::try_from(index).ok().and_then(|at| chain.get(at))
            })
            .is_some_and(|entry| !entry.enabled);
        self.chain_edit(audio, &[index], move |chain, at| {
            let entry = &mut chain[at[0]];
            let changed = entry.enabled != on;
            entry.enabled = on;
            changed
        });
    }

    pub fn chain_move(&mut self, audio: bool, index: i32, delta: i32) {
        if delta == 0 {
            return;
        }
        self.chain_edit(audio, &[index, index + delta], |chain, at| {
            chain.swap(at[0], at[1]);
            true
        });
    }

    pub fn chain_remove(&mut self, audio: bool, index: i32) {
        self.chain_edit(audio, &[index], |chain, at| {
            chain.remove(at[0]);
            true
        });
    }

    /// One knob of one link, on the echo. `clip_commit` makes it real.
    pub fn chain_set_param(&mut self, audio: bool, index: i32, key: &str, value: f32) {
        self.edit_links(audio, index, |link, _| {
            link.params.insert(key.to_owned(), f64::from(value));
        });
    }

    /// Link `index` of the shown clip's chain, and the same link of every
    /// other clip of a batch (see [`peer_link`]), on the echo; `edit` is told
    /// which one is the shown clip's. `clip_commit` makes it real.
    fn edit_links(&mut self, audio: bool, index: i32, edit: impl Fn(&mut AppliedFilter, bool)) {
        let ids = self.edit_targets();
        let Some(row) = usize::try_from(index).ok() else {
            return;
        };
        let Some(shown) = ids.first().and_then(|id| self.clip(id)).map(|clip| {
            if audio {
                clip.filters.clone()
            } else {
                clip.video_effects.clone()
            }
        }) else {
            return;
        };
        self.commit_targets = ids.clone();
        self.begin_echo();
        for (turn, id) in ids.iter().enumerate() {
            let Some(clip) = self.echo_clip_mut(id) else {
                continue;
            };
            let chain = if audio {
                &mut clip.filters
            } else {
                &mut clip.video_effects
            };
            if let Some(at) = peer_link(&shown, row, chain) {
                edit(&mut chain[at], turn == 0);
            }
        }
    }

    /// One knob of the colour panel, on the echo. The adjust package joins
    /// the head of the picture's chain the first time a knob moves, so an
    /// untouched clip carries nothing. A wheel's puck and master are knobs
    /// of their own here, `<key>.x`, `<key>.y` and `<key>.m`.
    pub fn adjust_set(&mut self, key: &str, value: f32) {
        let point = self.key_point().map(|(_, at)| at);
        self.edit_adjust_link(|link, shown| match point {
            // A knob that rides is edited where the playhead is: the value
            // becomes the key there, put on if there was none. Setting the
            // constant under a ride would change nothing on screen.
            Some(at) if shown && link.is_keyed(key) => {
                let ease = ease_before(link.keys_on(key), at);
                link.set_key(key, at, f64::from(value), ease);
            }
            // Another clip of a batch keeps its own ride: the playhead's
            // place is the shown clip's, and means nothing in that one.
            _ if !shown && link.is_keyed(key) => {}
            _ => {
                link.params.insert(key.to_owned(), f64::from(value));
            }
        });
    }

    // ── the Speed tab's curve ──

    /// The clip the Speed tab's curve belongs to: the one selected, when it
    /// is a video or a sound clip. With several selected, or a still, a
    /// title or a shape, the curve's controls change nothing.
    pub(super) fn speed_clip(&self) -> Option<String> {
        let id = self.sole_selection()?;
        let kind = self.clip(&id)?.kind;
        matches!(kind, model::ClipKind::Video | model::ClipKind::Audio).then_some(id)
    }

    /// The Speed tab's clip's curve, edited on the echo by `edit`, which is
    /// handed the curve - None for a constant rate - and the clip's rate.
    ///
    /// The rate and the length then follow the curve the way the command
    /// will make them, so the timeline shows the new length as the graph
    /// is dragged: the footage the clip covers is kept, the rate becomes
    /// the curve's mean, and the length is that footage at that rate. A
    /// curve taken away leaves the rate it averaged and the length.
    fn edit_speed_curve(&mut self, edit: impl FnOnce(&mut Option<Vec<model::SpeedPoint>>, f64)) {
        use model::ranges::{MAX_SPEED, MIN_CLIP_DURATION, MIN_SPEED};

        let Some(id) = self.speed_clip() else {
            return;
        };
        self.commit_targets = vec![id.clone()];
        self.begin_echo();
        let Some(clip) = self.echo_clip_mut(&id) else {
            return;
        };
        let footage = clip.duration * clip.speed;
        edit(&mut clip.speed_curve, clip.speed);
        if let Some(points) = clip.speed_curve.as_deref() {
            let mean = concat_project::speed::mean_of(points).clamp(MIN_SPEED, MAX_SPEED);
            clip.speed = mean;
            clip.duration = (footage / mean).max(MIN_CLIP_DURATION);
        }
    }

    /// Curve (`on`) or Constant. Curve gives a clip without one a level,
    /// smooth curve at its rate, so nothing changes until a point moves;
    /// Constant takes the curve away and leaves the rate it averaged.
    pub fn speed_curve_mode(&mut self, on: bool) {
        self.edit_speed_curve(|points, rate| {
            if !on {
                *points = None;
            } else if points.is_none() {
                *points = Some(speed_ramp::flat(rate));
            }
        });
        self.set_speed_chosen(Vec::new());
    }

    /// A point of the curve moved to `x` across the clip and `y` up the
    /// graph, or put down there when `index` is -1; see
    /// `speed_ramp::set_point`.
    pub fn speed_point(&mut self, index: i32, x: f32, y: f32) {
        // A point put down is the one chosen, so it can be dragged on or
        // taken straight off again.
        let mut placed = None;
        self.edit_speed_curve(|points, _| {
            if let Some(points) = points.as_mut() {
                let at = speed_ramp::set_point(points, index, x.into(), y.into());
                if index < 0 {
                    placed = at;
                }
            }
        });
        if let Some(at) = placed {
            self.set_speed_chosen(vec![at]);
        }
    }

    /// A point of the curve taken off; the two ends stay. The points after
    /// it move up a place, so none stays chosen.
    pub fn speed_remove(&mut self, index: i32) {
        self.edit_speed_curve(|points, _| {
            if let Some(points) = points.as_mut() {
                speed_ramp::remove_point(points, index);
            }
        });
        self.set_speed_chosen(Vec::new());
    }

    /// Smooth (`smooth`) or Straight, for every point of the curve.
    pub fn speed_smooth(&mut self, smooth: bool) {
        self.edit_speed_curve(|points, _| {
            if let Some(points) = points.as_mut() {
                speed_ramp::set_bends(points, smooth);
            }
        });
    }

    /// The Fastest slider (`faster`) or the Slowest, moved to `target`; see
    /// `speed_ramp::stretch_side`.
    pub fn speed_extreme(&mut self, faster: bool, target: f32) {
        self.edit_speed_curve(|points, _| {
            if let Some(points) = points.as_mut() {
                speed_ramp::stretch_side(points, faster, target.into());
            }
        });
    }

    /// The preset at `index` in the Shape list: its points replace the
    /// curve's, the clip's own from then on. An index that is not a preset
    /// changes nothing.
    pub fn speed_preset(&mut self, index: i32) {
        let Some(preset) = speed_ramp::preset_at(index) else {
            return;
        };
        self.edit_speed_curve(|points, _| *points = Some(preset));
        self.set_speed_chosen(Vec::new());
    }

    /// The Speed graph's chosen points, if they are on the selected clip.
    pub(super) fn speed_chosen_now(&self) -> &[usize] {
        match self.sole_selection() {
            Some(id) if id == self.speed_chosen.0 => &self.speed_chosen.1,
            _ => &[],
        }
    }

    /// The selected clip's curve as it stands, echo or document.
    fn speed_points_now(&self) -> Vec<model::SpeedPoint> {
        self.sole_selection()
            .and_then(|id| self.clip(&id))
            .and_then(|clip| clip.speed_curve.clone())
            .unwrap_or_default()
    }

    /// Sets the chosen points, on the selected clip.
    fn set_speed_chosen(&mut self, mut chosen: Vec<usize>) {
        chosen.sort_unstable();
        chosen.dedup();
        // The graph is published again after every press it hands over.
        self.speed_chosen = (self.sole_selection().unwrap_or_default(), chosen);
    }

    /// A point pressed: with `add` (Shift, or the platform's command key)
    /// it joins the chosen or leaves them; without, it becomes the one
    /// chosen - unless it is chosen already, so a press on one of several
    /// starts dragging them all.
    pub fn speed_choose(&mut self, index: i32, add: bool) {
        let Ok(index) = usize::try_from(index) else {
            self.set_speed_chosen(Vec::new());
            return;
        };
        let mut chosen = self.speed_chosen_now().to_vec();
        if add {
            match chosen.iter().position(|held| *held == index) {
                Some(at) => {
                    chosen.remove(at);
                }
                None => chosen.push(index),
            }
        } else if !chosen.contains(&index) {
            chosen = vec![index];
        }
        self.set_speed_chosen(chosen);
    }

    /// The points inside a box dragged across the graph, `[x0, y0, x1, y1]`
    /// on its scale, chosen - joining the chosen with `add`.
    pub fn speed_choose_box(&mut self, corners: [f32; 4], add: bool) {
        let [x0, y0, x1, y1] = corners.map(f64::from);
        let (left, right) = (x0.min(x1), x0.max(x1));
        let (low, high) = (y0.min(y1), y0.max(y1));
        let inside = self
            .speed_points_now()
            .iter()
            .enumerate()
            .filter(|(_, point)| {
                let y = speed_ramp::height_of(point.speed);
                (left..=right).contains(&point.at) && (low..=high).contains(&y)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let mut chosen = if add {
            self.speed_chosen_now().to_vec()
        } else {
            Vec::new()
        };
        chosen.extend(inside);
        self.set_speed_chosen(chosen);
    }

    /// Every point chosen, or none.
    pub fn speed_choose_all(&mut self, all: bool) {
        let count = if all {
            self.speed_points_now().len()
        } else {
            0
        };
        self.set_speed_chosen((0..count).collect());
    }

    /// A drag of the chosen points begins: the curve as it is now is what
    /// each move is measured from.
    pub fn speed_grab(&mut self) {
        let points = self.speed_points_now();
        self.speed_grabbed = (!points.is_empty()).then_some(points);
    }

    /// The chosen points moved together, `dx` across the clip and `dy` up
    /// the graph from where the drag began; see `speed_ramp::move_chosen`.
    pub fn speed_drag_chosen(&mut self, dx: f32, dy: f32) {
        let Some(from) = self.speed_grabbed.clone() else {
            return;
        };
        let chosen = self.speed_chosen_now().to_vec();
        self.edit_speed_curve(|points, _| {
            if let Some(points) = points.as_mut() {
                speed_ramp::move_chosen(points, &from, &chosen, dx.into(), dy.into());
            }
        });
    }

    /// The chosen points taken off; the two ends stay.
    pub fn speed_remove_chosen(&mut self) {
        let chosen = std::mem::take(&mut self.speed_chosen.1);
        self.edit_speed_curve(|points, _| {
            if let Some(points) = points.as_mut() {
                for index in chosen.iter().rev() {
                    speed_ramp::remove_point(points, *index as i32);
                }
            }
        });
    }

    /// One point of one of the colour panel's curves, on the echo: moved,
    /// or put down when `point` is negative.
    pub fn adjust_curve_point(&mut self, key: &str, point: i32, x: f32, y: f32) {
        self.edit_adjust_link(|link, _| {
            grading::set_curve_point(&mut link.params, key, point, x.into(), y.into());
        });
    }

    /// One point of one of the colour panel's curves taken off, on the echo.
    pub fn adjust_curve_remove(&mut self, key: &str, point: i32) {
        self.edit_adjust_link(|link, _| grading::remove_curve_point(&mut link.params, key, point));
    }

    /// The selected clips' colour links on the echo, each put at the head of
    /// its picture's chain if it is not there yet, handed to `edit` with
    /// whether it is the shown clip's. Nothing for a clip that is not a
    /// picture.
    fn edit_adjust_link(&mut self, edit: impl Fn(&mut AppliedFilter, bool)) {
        let ids = self.edit_targets();
        if ids.is_empty() {
            return;
        }
        self.commit_targets = ids.clone();
        self.begin_echo();
        for (turn, id) in ids.iter().enumerate() {
            let Some(clip) = self.echo_clip_mut(id) else {
                continue;
            };
            if !graded(clip.kind) {
                continue;
            }
            let entry = match clip
                .video_effects
                .iter()
                .position(|entry| entry.id == ADJUST_ID)
            {
                Some(entry) => entry,
                None => {
                    clip.video_effects.insert(0, AppliedFilter::new(ADJUST_ID));
                    0
                }
            };
            edit(&mut clip.video_effects[entry], turn == 0);
        }
    }

    /// One point of a curve of the picture chain's link `index`, on the
    /// echo: moved, or put down when `point` is negative.
    pub fn chain_curve_point(&mut self, index: i32, key: &str, point: i32, x: f32, y: f32) {
        self.edit_chain_link(index, |link| {
            grading::set_curve_point(&mut link.params, key, point, x.into(), y.into());
        });
    }

    /// One point of a curve of the picture chain's link `index` taken off,
    /// on the echo.
    pub fn chain_curve_remove(&mut self, index: i32, key: &str, point: i32) {
        self.edit_chain_link(index, |link| {
            grading::remove_curve_point(&mut link.params, key, point);
        });
    }

    /// A colour knob of the picture chain's link `index`, on the echo:
    /// packed into the knob's number (see `packed`).
    pub fn chain_set_colour(&mut self, index: i32, key: &str, colour: slint::Color) {
        self.edit_chain_link(index, |link| {
            link.params.insert(key.to_owned(), packed(colour));
        });
    }

    /// A colour knob's eyedropper, pressed: armed for the selected clip's
    /// link `index` and knob `key`, so the next press on the stage takes
    /// the colour there off the clip's own picture (see `pick_colour_at`),
    /// or put down again when it already was. Only a clip with a file's
    /// picture of its own offers one.
    pub fn chain_pick_colour(&mut self, index: i32, key: &str) {
        let Ok(entry) = usize::try_from(index) else {
            return;
        };
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)).cloned() else {
            return;
        };
        let pick = ColourPick {
            clip: clip.id.clone(),
            entry,
            key: key.to_owned(),
        };
        if self.picking.as_ref() == Some(&pick) {
            self.picking = None;
            return;
        }
        if !picks_colour(clip.kind) {
            self.notify(&t("studio.pickColourNoPicture"), true);
            return;
        }
        self.flush_commit();
        self.picking = Some(pick);
        self.notify(&t("studio.pickColourHint"), false);
    }

    /// The eyedropper put down without a pick: Escape, or the project
    /// going. Whether one was armed.
    pub fn cancel_colour_pick(&mut self) -> bool {
        self.picking.take().is_some()
    }

    /// A colour knob of the selected clip's link `index` back to its
    /// default - the colour its package finds by itself - as one edit.
    pub fn chain_auto_colour(&mut self, index: i32, key: &str) {
        let Some(clip) = self.sole_selection().and_then(|id| self.clip(&id)).cloned() else {
            return;
        };
        let Some(link) = usize::try_from(index)
            .ok()
            .and_then(|index| clip.video_effects.get(index))
        else {
            return;
        };
        if !link.params.contains_key(key) {
            return;
        }
        let mut effects = clip.video_effects.clone();
        if let Some(link) = usize::try_from(index)
            .ok()
            .and_then(|index| effects.get_mut(index))
        {
            link.params.remove(key);
        }
        self.flush_commit();
        self.apply(Command::UpdateClip {
            clip_id: clip.id,
            patch: ClipPatch {
                video_effects: Some(effects),
                ..ClipPatch::default()
            },
        });
    }

    /// A press on the stage while an eyedropper is armed: the colour of the
    /// clip's own picture under it - before its effects, through its crop,
    /// flips, turn and stabilization, the way a brush stroke finds its
    /// pixel - read off its file at the playhead on a worker, and laid on
    /// the knob as one edit. A press off the picture says so and keeps the
    /// eyedropper armed; a pick disarms it.
    pub(super) fn pick_colour_at(&mut self, pick: ColourPick, x: f64, y: f64) {
        let Some(clip) = self
            .stage_clips()
            .into_iter()
            .find(|shown| shown.id == pick.clip)
            .cloned()
        else {
            self.picking = None;
            self.notify(&t("studio.pickColourGone"), true);
            return;
        };
        let [u, v] = self.stage_to_source(&clip, x, y);
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            self.notify(&t("studio.pickColourOffPicture"), false);
            return;
        }
        let Some(media) = self.project().media_by_id(&clip.media_id) else {
            self.picking = None;
            return;
        };
        let path = media.path.clone();
        let seconds = self.source_at_playhead(&clip);
        self.picking = None;
        spawn_in_project(
            move || {
                concat_host::media::still_at(&path, seconds, 960)
                    .and_then(|frame| colour_about(&frame, u, v).ok_or_else(String::new))
            },
            move |studio, _, _, picked| match picked {
                Ok(colour) => studio.lay_picked_colour(&pick, colour),
                Err(error) if error.is_empty() => {}
                Err(error) => studio.notify(&error, true),
            },
        );
    }

    /// A colour taken off the picture laid on the knob it was picked for,
    /// wherever the selection has gone since, as one edit. A key told a
    /// grey - nothing to key - says so, and where a black backdrop goes.
    fn lay_picked_colour(&mut self, pick: &ColourPick, [r, g, b]: [u8; 3]) {
        let Some(clip) = self.clip(&pick.clip).cloned() else {
            return;
        };
        let mut effects = clip.video_effects.clone();
        let Some(link) = effects.get_mut(pick.entry) else {
            return;
        };
        let package = Catalogue::builtin()
            .packages()
            .find(|package| package.answers_to(&link.id));
        let Some(package) = package.filter(|package| {
            package
                .manifest
                .param(&pick.key)
                .is_some_and(|param| param.kind == ParamType::Color)
        }) else {
            return;
        };
        let keys = package.category().eq_ignore_ascii_case("Cutout");
        link.params.insert(
            pick.key.clone(),
            packed(slint::Color::from_argb_u8(255, r, g, b)),
        );
        self.flush_commit();
        self.apply(Command::UpdateClip {
            clip_id: clip.id,
            patch: ClipPatch {
                video_effects: Some(effects),
                ..ClipPatch::default()
            },
        });
        if keys && !has_hue([r, g, b]) {
            self.notify(&t("studio.pickColourGrey"), false);
        }
    }

    /// The picture chain's link `index` of the selected clip, on the echo.
    fn edit_chain_link(&mut self, index: i32, edit: impl Fn(&mut AppliedFilter)) {
        self.edit_links(false, index, |link, _| edit(link));
    }

    /// The colour link of the selected clip - the one the Adjust panel's
    /// keys go on - by index, with the clip and the playhead's place in it.
    /// None when nothing is selected, it is not a picture, or the playhead
    /// is outside it.
    fn adjust_point(&self) -> Option<(Clip, f64, Option<usize>)> {
        let (clip, at) = self.key_point()?;
        if !graded(clip.kind) {
            return None;
        }
        let entry = clip
            .video_effects
            .iter()
            .position(|entry| entry.id == ADJUST_ID);
        Some((clip.clone(), at, entry))
    }

    /// Puts a key on one Adjust knob at the playhead, or takes off the one
    /// there. Like `toggle_key`, the new key holds what the knob is worth
    /// at that instant, so pressing the diamond never moves the picture. A
    /// wheel - named by itself or by one of its own keys - is one knob: its
    /// puck and master are keyed, and unkeyed, together.
    pub fn toggle_adjust_key(&mut self, key: &str) {
        let Some((clip, at, entry)) = self.adjust_point() else {
            return;
        };
        let names = adjust_key_names(key);
        let clip_id = clip.id.clone();
        let Some(entry) = entry else {
            // No colour link yet: one goes on, then the keys on it, as one
            // edit, so an undo takes all of it back.
            let mut effects = clip.video_effects.clone();
            effects.insert(0, AppliedFilter::new(ADJUST_ID));
            let mut commands = vec![Command::UpdateClip {
                clip_id: clip_id.clone(),
                patch: ClipPatch {
                    video_effects: Some(effects),
                    ..ClipPatch::default()
                },
            }];
            commands.extend(names.iter().map(|(name, default)| Command::SetEffectKey {
                clip_id: clip_id.clone(),
                entry: 0,
                key: name.clone(),
                at,
                value: *default,
                ease: model::KeyEase::LINEAR,
            }));
            self.apply(Command::Batch { commands });
            return;
        };
        let link = &clip.video_effects[entry];
        // The knob is keyed here when its own key - a wheel's master - is.
        let on = names
            .last()
            .is_some_and(|(name, _)| link.key_at(name, at).is_some());
        let mut commands: Vec<Command> = names
            .iter()
            .filter_map(|(name, default)| {
                if on {
                    link.key_at(name, at).map(|_| Command::ClearEffectKey {
                        clip_id: clip_id.clone(),
                        entry,
                        key: name.clone(),
                        at,
                    })
                } else {
                    Some(Command::SetEffectKey {
                        clip_id: clip_id.clone(),
                        entry,
                        key: name.clone(),
                        at,
                        value: link.value_at(name, at, *default),
                        ease: ease_before(link.keys_on(name), at),
                    })
                }
            })
            .collect();
        let command = if commands.len() == 1 {
            commands.remove(0)
        } else {
            Command::Batch { commands }
        };
        self.apply(command);
    }

    /// Takes every key off one Adjust knob - a wheel's puck and master
    /// together - leaving it the value it holds.
    pub fn clear_adjust_keys(&mut self, key: &str) {
        let Some((clip, _, Some(entry))) = self.adjust_point() else {
            return;
        };
        let link = &clip.video_effects[entry];
        let mut commands: Vec<Command> = adjust_key_names(key)
            .into_iter()
            .filter(|(name, _)| link.is_keyed(name))
            .map(|(name, _)| Command::ClearEffectKeys {
                clip_id: clip.id.clone(),
                entry,
                key: name,
            })
            .collect();
        match commands.len() {
            0 => {}
            1 => {
                self.apply(commands.remove(0));
            }
            _ => {
                self.apply(Command::Batch { commands });
            }
        }
    }

    /// Moves the playhead to an Adjust knob's previous (-1) or next (+1) key.
    pub fn step_adjust_key(&mut self, key: &str, delta: i32) {
        let Some((clip, at, Some(entry))) = self.adjust_point() else {
            return;
        };
        let Some((name, _)) = adjust_key_names(key).pop() else {
            return;
        };
        let (prev, next) = clip.video_effects[entry].keys_around(&name, at);
        let Some(target) = (if delta < 0 { prev } else { next }) else {
            return;
        };
        self.seek((clip.start + target * clip.duration) as f32);
    }

    // ── the effect libraries ──

    /// The view state of one library, or None for a shelf index the panel
    /// should never have sent.
    fn library_at(&mut self, shelf: i32) -> Option<&mut LibraryView> {
        self.library.get_mut(usize::try_from(shelf).ok()?)
    }

    /// The search box was typed in. A live query searches every shelf, so
    /// the strip is put away while one is running - see `shelves`.
    pub fn library_query(&mut self, shelf: i32, text: &str) {
        if let Some(view) = self.library_at(shelf) {
            view.query = text.to_owned();
        }
    }

    /// A shelf was picked. Typing and then picking a shelf means the shelf,
    /// so the query goes.
    pub fn library_group(&mut self, shelf: i32, index: i32) {
        if let Some(view) = self.library_at(shelf) {
            // -1 is every shelf at once: the phone's All pill.
            view.group = index.max(-1);
            view.category.clear();
            view.query.clear();
            view.favourites = false;
        }
    }

    /// A category pill was picked.
    pub fn library_category(&mut self, shelf: i32, category: &str) {
        if let Some(view) = self.library_at(shelf) {
            view.category = category.to_owned();
            view.group = -1;
            view.query.clear();
        }
    }

    /// The star chip. Turning it on clears the query for the same reason
    /// picking a shelf does: two filters at once is a list nobody can
    /// predict the contents of.
    pub fn library_favourites(&mut self, shelf: i32, on: bool) {
        if let Some(view) = self.library_at(shelf) {
            view.favourites = on;
            view.query.clear();
        }
    }

    /// Star or unstar a package, and remember it. Not per library: a star is
    /// a fact about the package.
    pub fn library_favourite(&mut self, id: &str, on: bool) {
        let held = self.prefs.favourites.iter().any(|held| held == id);
        if on && !held {
            self.prefs.favourites.push(id.to_owned());
        } else if !on && held {
            self.prefs.favourites.retain(|held| held != id);
        } else {
            return;
        }
        self.prefs.save(&self.host.dirs);
    }
}
