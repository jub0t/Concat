// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The background behind the main picture, and Fit and Fill: the panels'
//! side of `model::Background` and `Command::FrameClip`. What is drawn,
//! and behind which clips, is the model's and the renderer's to say; this
//! only turns a press into a command and the document into the rows the
//! panels read off the `Backdrop` global.

use concat_project::model::{Background, Framing, MediaKind};

use super::*;

/// The colour a colour background starts at: white, the other plain
/// background vertical cuts are made with, and one nobody mistakes for none.
const FIRST_COLOUR: &str = "#ffffff";

/// The kind as the panels number it: 0 black, 1 blur, 2 colour, 3 a still.
fn mode_of(background: &Background) -> i32 {
    match background {
        Background::None => 0,
        Background::Blur { .. } => 1,
        Background::Color { .. } => 2,
        Background::Image { .. } => 3,
    }
}

impl Studio {
    /// The bin's stills, in the bin's order: the rows of the still picker,
    /// which a row index is read back against.
    fn stills(&self) -> Vec<&model::MediaItem> {
        self.project()
            .media
            .iter()
            .filter(|item| item.kind == MediaKind::Image && !item.placeholder)
            .collect()
    }

    /// The selected clip, when it is one a background is drawn behind: a
    /// video or a still on the timeline's lowest picture lane.
    fn backdrop_clip(&self) -> Option<&Clip> {
        let id = self.sole_selection()?;
        let timeline = self.timeline();
        let clip = timeline.clip(&id)?;
        (clip.kind.is_visual() && timeline.background_lane() == Some(clip.track_id.as_str()))
            .then_some(clip)
    }

    /// A background as the panels show it; None is a clip following the
    /// timeline's.
    fn backdrop_data(&self, background: Option<&Background>) -> BackdropData {
        let still = |media_id: &str| {
            self.stills()
                .iter()
                .position(|item| item.id == media_id)
                .map_or(-1, |row| row as i32)
        };
        let amount = match background {
            Some(Background::Blur { amount }) => *amount,
            _ => Background::DEFAULT_BLUR,
        };
        BackdropData {
            mode: background.map_or(-1, mode_of),
            amount: amount as f32,
            colour: background
                .and_then(Background::rgb)
                .map_or(colour_of(FIRST_COLOUR), |[r, g, b]| {
                    slint::Color::from_rgb_u8(r, g, b)
                }),
            still: match background {
                Some(Background::Image { media_id }) => still(media_id),
                _ => -1,
            },
        }
    }

    /// The `Backdrop` global: the timeline's background, the selected
    /// clip's, whether it has one to have, and the stills to pick from.
    pub fn publish_backdrop(&self, app: &App) {
        let backdrop = app.global::<Backdrop>();
        backdrop.set_timeline(self.backdrop_data(Some(&self.timeline().background)));
        let clip = self.backdrop_clip();
        backdrop.set_clip_has(clip.is_some());
        // What the clip draws: its own background, or the timeline's it
        // follows. The panels show that, not a "follows" entry of its own:
        // "Timeline's" beside None and Blur read as one more kind of
        // background, and nobody could say which.
        let timeline = &self.timeline().background;
        backdrop.set_clip(clip.map_or_else(BackdropData::default, |clip| {
            self.backdrop_data(Some(clip.background.as_ref().unwrap_or(timeline)))
        }));
        backdrop.set_can_frame(self.framing_target().is_some());
        // Written only when it differs, as the audio tracks are: a fresh
        // model is a change to every binding that reads it.
        let names: Vec<SharedString> = self
            .stills()
            .iter()
            .map(|item| item.name.as_str().into())
            .collect();
        let shown = backdrop.get_stills();
        let same = shown.row_count() == names.len()
            && names
                .iter()
                .enumerate()
                .all(|(row, name)| shown.row_data(row).as_ref() == Some(name));
        if !same {
            backdrop.set_stills(ModelRc::new(VecModel::from(names)));
        }
    }

    /// The background a panel names as it is now: the timeline's, or the
    /// selected clip's own, None while that follows the timeline's.
    fn backdrop_now(&self, clip: bool) -> Option<Background> {
        if clip {
            self.backdrop_clip()?.background.clone()
        } else {
            Some(self.timeline().background.clone())
        }
    }

    /// Writes a background: the timeline's, or the selected clip's own,
    /// None following the timeline's again. A `knob` is a value dragged or
    /// picked through, which folds into the step its last edit made.
    fn backdrop_write(&mut self, clip: bool, background: Option<Background>, knob: Option<&str>) {
        let command = if clip {
            let Some(target) = self.backdrop_clip() else {
                return;
            };
            Command::UpdateClip {
                clip_id: target.id.clone(),
                patch: ClipPatch {
                    background: Some(background),
                    ..ClipPatch::default()
                },
            }
        } else {
            Command::SetTimelineBackground {
                timeline_id: self.timeline().id.clone(),
                background: background.unwrap_or_default(),
            }
        };
        match knob {
            Some(knob) => {
                let whose = if clip { "clip" } else { "timeline" };
                self.apply_knob(format!("background:{whose}:{knob}"), command);
            }
            None => {
                self.apply(command);
            }
        }
    }

    /// Picks the kind of background: `mode` as the panels number it, a
    /// clip's -1 going back to the timeline's. A new kind starts from what
    /// is there - the softness a blur had, the colour, the still - and
    /// from the timeline's for a clip that followed it; a still starts as
    /// the bin's first, or as none yet in a bin without one.
    pub fn backdrop_set_mode(&mut self, clip: bool, mode: i32) {
        let now = self.backdrop_now(clip);
        if clip && self.backdrop_clip().is_none() {
            return;
        }
        let from = now
            .clone()
            .unwrap_or_else(|| self.timeline().background.clone());
        let background = match mode {
            -1 if clip => None,
            1 => Some(Background::Blur {
                amount: match from {
                    Background::Blur { amount } => amount,
                    _ => Background::DEFAULT_BLUR,
                },
            }),
            2 => Some(Background::Color {
                color: match from {
                    Background::Color { color } => color,
                    _ => FIRST_COLOUR.to_owned(),
                },
            }),
            3 => Some(Background::Image {
                media_id: match from {
                    Background::Image { media_id } => media_id,
                    _ => self
                        .stills()
                        .first()
                        .map(|item| item.id.clone())
                        .unwrap_or_default(),
                },
            }),
            _ => Some(Background::None),
        };
        if background != now {
            self.backdrop_write(clip, background, None);
        }
    }

    /// Apply to all, as CapCut has it: the selected clip's background
    /// becomes the timeline's, and every picture on the main lane follows
    /// it, any of their own let go. One undo step.
    pub fn backdrop_apply_to_all(&mut self) {
        let Some(clip) = self.backdrop_clip() else {
            return;
        };
        let background = clip
            .background
            .clone()
            .unwrap_or_else(|| self.timeline().background.clone());
        let timeline = self.timeline();
        let lane = timeline.background_lane().map(str::to_owned);
        let mut commands = vec![Command::SetTimelineBackground {
            timeline_id: timeline.id.clone(),
            background,
        }];
        commands.extend(
            timeline
                .clips
                .iter()
                .filter(|other| {
                    other.background.is_some() && Some(other.track_id.as_str()) == lane.as_deref()
                })
                .map(|other| Command::UpdateClip {
                    clip_id: other.id.clone(),
                    patch: ClipPatch {
                        background: Some(None),
                        ..ClipPatch::default()
                    },
                }),
        );
        self.apply(Command::Batch { commands });
    }

    /// A blur's softness, as its knob turns.
    pub fn backdrop_set_amount(&mut self, clip: bool, amount: f32) {
        let amount = f64::from(amount).clamp(0.0, 1.0);
        self.backdrop_write(clip, Some(Background::Blur { amount }), Some("amount"));
    }

    /// A colour background's colour, as the picker moves.
    pub fn backdrop_set_colour(&mut self, clip: bool, colour: slint::Color) {
        let color = hex_of(colour);
        self.backdrop_write(clip, Some(Background::Color { color }), Some("colour"));
    }

    /// A still background's still, by its row in the picker.
    pub fn backdrop_set_still(&mut self, clip: bool, row: i32) {
        let Some(media_id) = usize::try_from(row)
            .ok()
            .and_then(|row| self.stills().get(row).map(|item| item.id.clone()))
        else {
            return;
        };
        self.backdrop_write(clip, Some(Background::Image { media_id }), None);
    }

    /// The selected picture Fit and Fill can size: one video or still on
    /// a lane that is not locked, from media whose size the bin knows.
    pub(super) fn framing_target(&self) -> Option<&Clip> {
        let id = self.sole_selection()?;
        let clip = self.clip(&id)?;
        if self.locked(&clip.track_id) {
            return None;
        }
        let media = self.project().media_by_id(&clip.media_id)?;
        clip.framing_scale(
            Framing::Fit,
            (media.width, media.height),
            self.output_size(),
            0.0,
        )
        .map(|_| clip)
    }

    /// Sizes the selected picture to the frame: whole inside it, or
    /// covering it. A keyed scale or position takes its key at the
    /// playhead, as the knobs write one.
    pub fn frame_selected(&mut self, fill: bool) {
        let Some(clip) = self.framing_target() else {
            return;
        };
        let (clip_id, at) = (clip.id.clone(), place_in(clip, self.playhead));
        let framing = if fill { Framing::Fill } else { Framing::Fit };
        self.apply(Command::FrameClip {
            clip_id,
            framing,
            at,
        });
    }

    /// One edit of a value that is dragged rather than pressed, folded into
    /// the step the same knob's last edit made within a moment of it, the
    /// way the inspector's commits fold: a drag across the softness is one
    /// undo, not forty.
    fn apply_knob(&mut self, key: String, command: Command) {
        self.flush_commit();
        let now = std::time::Instant::now();
        let continues = self
            .last_commit
            .as_ref()
            .is_some_and(|(last, at)| *last == key && now.duration_since(*at).as_millis() < 900);
        if !continues && let Some(session) = self.session.as_mut() {
            session.end_gesture();
        }
        self.apply_within(&key, command);
        self.last_commit = Some((key, now));
    }
}
