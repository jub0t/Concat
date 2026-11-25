// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! The keyboard, and the menu's verbs.

use super::menus::PlayheadSide;
use super::*;

impl Studio {
    // ── the keyboard, and the menu's verbs ──

    /// A chord from the window's key table; see `Editor.shortcut`. The view
    /// chords - zoom, the playhead's ends, the sheets - go through the app
    /// menu's own handler in lib.rs, so a key and the row that advertises
    /// it are one thing.
    pub fn shortcut(&mut self, action: &str) {
        log::debug!("shortcut: {action}");
        match action {
            // The selected clips when there are any, as the tray's button
            // and the menu split: the chord cut every track under the
            // playhead whatever was selected, titles and music with it.
            "split" => {
                let at = self.playhead;
                self.split_at(at, true);
            }
            "split-selected" => {
                let at = self.playhead;
                self.split_at(at, true);
            }
            // B: the razor, on and off.
            "split-tool" => {
                self.lanes.tool = if self.lanes.tool == TimelineTool::Split {
                    TimelineTool::Select
                } else {
                    TimelineTool::Split
                };
            }
            // Q / W: the clip loses what is left, or right,
            // of the playhead.
            "delete-left" => self.delete_to_playhead(PlayheadSide::Left),
            "delete-right" => self.delete_to_playhead(PlayheadSide::Right),
            "select-all" => self.select_all(),
            // The phone's way out of the clip tools: nothing selected, the
            // way a press on an empty lane leaves it.
            "deselect" => {
                self.flush_commit();
                self.end_audition();
                self.selection.clear();
                self.transition_selected = None;
            }
            // ⇧⌫, from the key table; plain ⌫ comes in as its own callback.
            "ripple-delete" => self.ripple_delete_selected(),
            // Copy takes the whole selection; see `copy_clips`.
            "copy" => self.copy_clips(None),
            "duplicate" | "mute" => {
                if let Some(id) = self.sole_selection() {
                    self.clip_action(&id, action);
                }
            }
            // The tray's picture verbs. Mirror is the flip the H key does,
            // without the setting that gates the key: a button on the tray
            // is not a key someone might press by accident.
            "mirror" => self.toggle_flip_h(),
            "rotate" => self.rotate_selected(),
            "reverse" => {
                if let Some(id) = self.sole_selection() {
                    self.reverse_clip(&id);
                }
            }
            "freeze" => {
                self.menu_target = None;
                self.freeze_at_playhead();
            }
            // After the selected clip, on its lane, as the menu does; with
            // nothing selected, at the playhead on the rows copied from.
            "paste" => {
                let after = self.sole_selection();
                self.paste_clips(after.as_deref());
            }
            _ => {}
        }
    }

    /// The sounds detached from an item, as the panels hold their ids.
    pub(super) fn detached_from(&self, id: ItemId) -> Vec<String> {
        self.doc()
            .items
            .index()
            .tied_to(id)
            .filter(|other| {
                doc_view::item(self.doc(), *other).is_some_and(|item| {
                    item.link
                        .as_ref()
                        .is_some_and(|link| link.kind == model::LinkKind::SoundOf)
                })
            })
            .map(|other| other.to_string())
            .collect()
    }

    /// Takes a video clip's sound off onto a lane of its own, and selects
    /// what came off: the lane it lands on may be a new one, out of view,
    /// and a detach that changed nothing anyone could see read as one that
    /// did nothing. One that cannot happen says why instead of nothing.
    /// https://github.com/jub0t/Concat/issues/310
    fn detach_audio(&mut self, id: &str) {
        let (Some(item_id), Some(clip)) = (doc_view::parse::<ItemId>(id), self.clip(id)) else {
            return;
        };
        let has_audio = doc_view::has_sound(self.doc(), clip);
        let detached = !self.detached_from(item_id).is_empty();
        let plan = detach_plan(self.kind(clip), clip.sound.muted, has_audio, detached);
        let detach = Command::DetachAudio { item: item_id };
        let command = match plan {
            DetachPlan::NoSound => {
                self.notify(&t("studio.detachNoSound"), false);
                return;
            }
            DetachPlan::Already => {
                self.notify(&t("studio.detachAlready"), false);
                return;
            }
            DetachPlan::Detach => detach,
            // Its sound was taken off once and that clip deleted, which
            // left the picture silent with nothing to reattach. Unmuted
            // and taken off again, as one step: the sound comes back as a
            // clip, which is what was asked for.
            DetachPlan::Again => Command::Batch {
                commands: vec![
                    Command::UpdateClip {
                        item: item_id,
                        patch: ItemPatch {
                            muted: Some(false),
                            ..Default::default()
                        },
                    },
                    detach,
                ],
            },
        };
        let token = self.toast.token;
        self.apply(command);
        let sounds = self.detached_from(item_id);
        if sounds.is_empty() {
            // A failure has said so already.
            if self.toast.token == token {
                self.notify(&t("studio.detachNoSound"), false);
            }
            return;
        }
        self.transition_selected = None;
        self.selection = sounds;
    }

    /// One of the clip menu's verbs on clip `id`. The menu's rows and the
    /// keyboard's chords both land here, so a shortcut and the row that
    /// advertises it cannot disagree.
    pub fn clip_action(&mut self, id: &str, action: &str) {
        log::debug!("clip menu: {action} on {id}");
        // The edits this makes say they came from the clip's menu, unless a
        // shortcut brought them here.
        let _asked = Asked::by("clip menu");
        let (Some(item_id), Some(clip)) = (doc_view::parse::<ItemId>(id), self.clip(id).cloned())
        else {
            return;
        };
        match action {
            "copy" => self.copy_clips(Some(id)),
            "duplicate" => self.duplicate_selected(),
            // Pasted after the clip that was right-clicked, on its lane.
            "paste" => self.paste_clips(Some(id)),
            "split" => {
                let at = self.playhead;
                self.flush_commit();
                self.selection = vec![id.to_owned()];
                self.split_at(at, true);
            }
            "freeze" => self.freeze_at_playhead(),
            // The clip right-clicked, or the selection when it is one of
            // it, as Delete takes it.
            "delete-left" | "delete-right" => {
                let ids =
                    if self.selection.len() > 1 && self.selection.iter().any(|held| held == id) {
                        self.selection.clone()
                    } else {
                        vec![id.to_owned()]
                    };
                let side = if action == "delete-left" {
                    PlayheadSide::Left
                } else {
                    PlayheadSide::Right
                };
                self.delete_clips_to_playhead(&ids, side);
            }
            "merge" => self.merge(),
            // On the clip that was right-clicked, which becomes the
            // selection, as Enhance's row makes it.
            "fit" | "fill" => {
                self.flush_commit();
                self.selection = vec![id.to_owned()];
                self.transition_selected = None;
                self.frame_selected(action == "fill");
            }
            // The menu opens the clip's Enhance in the inspector, where its
            // switch starts it and later turns it off and on again.
            "enhance" => {
                self.flush_commit();
                self.selection = vec![id.to_owned()];
                self.transition_selected = None;
                self.inspector_jump = (self.inspector_jump.0 + 1, "Video", "Enhance");
            }
            "export-audio" => self.export_clip_audio(id),
            // One of the two ways, or back to the recording when that way
            // is the one already on; on the selection when the clip is in
            // it, else on the clip alone.
            "voice-keep" | "voice-remove" => {
                self.flush_commit();
                if !self.selection.iter().any(|held| held == id) {
                    self.selection = vec![id.to_owned()];
                    self.transition_selected = None;
                }
                // The kind the clip had; speech apart from noise else.
                let kind = clip
                    .sound
                    .voice
                    .as_ref()
                    .map_or(model::VoiceKind::Speech, |mix| mix.kind.clone());
                let wanted = if action == "voice-keep" {
                    doc_view::keep_voice(kind)
                } else {
                    doc_view::remove_voice(kind)
                };
                let on = clip.sound.voice.as_ref().is_some_and(|mix| {
                    (mix.voice == 0.0) == (wanted.voice == 0.0)
                        && (mix.background == 0.0) == (wanted.background == 0.0)
                });
                self.set_voice((!on).then_some(wanted));
            }
            "detach" => self.detach_audio(id),
            "reattach" => {
                self.apply(Command::ReattachAudio { item: item_id });
            }
            "mute" => {
                let volume = if clip.sound.volume.value <= 0.0 {
                    1.0
                } else {
                    0.0
                };
                self.apply(Command::UpdateClip {
                    item: item_id,
                    patch: ItemPatch {
                        volume: Some(volume),
                        ..Default::default()
                    },
                });
            }
            "lock" => self.toggle_lock(&clip.lane.to_string()),
            // A clip that is part of the selection takes the selection with
            // it: Delete on one of five selected clips means the five.
            "delete" | "ripple-delete" => {
                let whole =
                    self.selection.len() > 1 && self.selection.iter().any(|held| held == id);
                let ripple = action == "ripple-delete"
                    || if whole {
                        self.closes_gap(&self.selection)
                    } else {
                        self.closes_gap(&[id.to_owned()])
                    };
                if whole {
                    self.remove_selected(ripple);
                } else {
                    self.apply(Command::RemoveClips {
                        items: vec![item_id],
                        ripple,
                    });
                }
                self.menu_target = None;
            }
            _ => {}
        }
    }

    /// Every clip on an unlocked lane — or every visible media card when
    /// the bin had the last press (#696).
    pub fn select_all(&mut self) {
        self.flush_commit();
        if self.media_focused {
            self.handle(crate::panes::Msg::Media(
                crate::panes::media_bin::MediaMsg::SelectAll,
            ));
            return;
        }
        self.selection = self
            .clips()
            .into_iter()
            .filter(|(_, clip)| !self.lane_locked(clip.lane))
            .map(|(id, _)| id)
            .collect();
    }

    /// Tab `from` dropped at `slot`, a position counted over the strip as
    /// it stands; the command counts with the tab already removed.
    pub fn move_timeline(&mut self, from: i32, slot: i32) {
        let Some(from) = usize::try_from(from).ok() else {
            return;
        };
        let Some(sequence) = self.doc().view.tabs.get(from).copied() else {
            return;
        };
        let slot = slot.max(0) as usize;
        let index = if from < slot { slot - 1 } else { slot };
        if index == from {
            return;
        }
        self.apply(Command::MoveTimeline { sequence, index });
    }
}
