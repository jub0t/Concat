// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A frozen selection rendered through the normal exporter before an atomic
//! `ConsolidateClips` edit. Source files are never rewritten.

use std::path::Path;
use std::sync::Arc;

use concat_doc::commands::Command;
use concat_doc::commands::consolidation::consolidation;
use concat_doc::model::{Content, Frame, Item, ItemId, LaneId, Media, MediaId, Model, SequenceId};
use concat_export::ExportRequest;

/// The exact clips and files a render reads. Rechecked before replacement
/// so editing, undo, relinking or switching timelines cannot adopt stale
/// work.
pub struct Consolidation {
    sequence: SequenceId,
    frame: Frame,
    items: Vec<(ItemId, Arc<Item>)>,
    media: Vec<(MediaId, Arc<Media>)>,
}

impl Consolidation {
    /// Captures a valid selection without changing the project.
    pub fn new(project: &Model, ids: &[ItemId]) -> Result<Self, String> {
        let sequence = project
            .view
            .showing
            .ok_or_else(|| "There is no timeline open.".to_owned())?;
        let frame = project
            .sequences
            .get(sequence)
            .map(|sequence| sequence.frame.clone())
            .ok_or_else(|| "There is no timeline open.".to_owned())?;
        let ordered = consolidation(project, ids)?;
        let mut items = Vec::with_capacity(ordered.len());
        let mut media: Vec<(MediaId, Arc<Media>)> = Vec::new();
        for id in ordered {
            let item = project
                .items
                .get(id)
                .cloned()
                .ok_or_else(|| "A selected clip no longer exists.".to_owned())?;
            if let Content::Media(cut) = &item.content {
                let file = project
                    .media
                    .get(cut.media)
                    .cloned()
                    .ok_or_else(|| "A selected clip's media is missing.".to_owned())?;
                if !media.iter().any(|(kept, _)| *kept == cut.media) {
                    media.push((cut.media, file));
                }
            }
            items.push((id, item));
        }
        Ok(Self {
            sequence,
            frame,
            items,
            media,
        })
    }

    /// The lane the replacement belongs on, for the window's lock check.
    pub fn lane(&self) -> LaneId {
        self.items[0].1.lane
    }

    /// The edit that puts the rendered `file` in the clips' place, while
    /// their inputs and the timeline's frame still match what was rendered.
    pub fn replacement(&self, project: &Model, file: Media) -> Result<Command, String> {
        let ids: Vec<ItemId> = self.items.iter().map(|(id, _)| *id).collect();
        let same_items = self
            .items
            .iter()
            .all(|(id, item)| project.items.get(*id).is_some_and(|now| now == item));
        let same_media = self
            .media
            .iter()
            .all(|(id, file)| project.media.get(*id).is_some_and(|now| now == file));
        let same_frame = project
            .sequences
            .get(self.sequence)
            .is_some_and(|sequence| sequence.frame == self.frame);
        if project.view.showing != Some(self.sequence) || !same_items || !same_media || !same_frame
        {
            return Err("The selected clips changed while merging. Please try again.".into());
        }
        Ok(Command::ConsolidateClips { items: ids, file })
    }

    /// A render of the selection alone, moved to zero. Timeline gaps stay
    /// gaps; source gaps left by deleting pauses are never put back. A
    /// lane's hide and mute stay live on the replacement rather than
    /// baking silence or black into a file that could later be unmuted.
    pub fn request(&self, project: &Model, folder: &Path, output: &Path) -> ExportRequest {
        let ids: Vec<ItemId> = self.items.iter().map(|(id, _)| *id).collect();
        let mut clips =
            concat_export::flatten::clips_of(project, self.sequence, Some(&ids), Some(folder));
        let start = self.items[0].1.start.display_seconds();
        for (clip, (_, item)) in clips.iter_mut().zip(&self.items) {
            clip.start -= start;
            clip.hidden = false;
            clip.muted = item.sound.muted;
        }
        let rate = f64::from(self.frame.rate.num()) / f64::from(self.frame.rate.den().max(1));
        // The exporter rounds its end to frames. A hidden, silent marker
        // makes it round up for a source file: even a fractional last frame
        // must be there when the replacement is trimmed to the exact
        // original span. No clip, key or fade is stretched for it.
        let end = clips
            .iter()
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max);
        let padded = (end * rate - 1e-9).ceil() / rate;
        if padded > end + 1e-9 {
            let top = clips.iter().map(|clip| clip.track + 1).max().unwrap_or(0);
            let mut marker =
                concat_export::ExportClip::blank(concat_export::ClipKind::Layer, 0.0, padded, top);
            marker.hidden = true;
            marker.muted = true;
            marker.has_audio = Some(false);
            clips.push(marker);
        }
        let color_space = concat_export::ColorSpace::of(&self.frame.color);
        let hdr = color_space != concat_export::ColorSpace::Sdr;
        let canvas = self.frame.canvas;
        ExportRequest {
            output: output.to_string_lossy().into_owned(),
            canvas: [canvas.r, canvas.g, canvas.b, canvas.a],
            width: self.frame.width.get(),
            height: self.frame.height.get(),
            rate_num: i64::from(self.frame.rate.num()),
            rate_den: i64::from(self.frame.rate.den()),
            crf: 16,
            preset: "fast".into(),
            codec: if hdr {
                crate::export::VideoCodec::Hevc
            } else {
                crate::export::VideoCodec::H264
            },
            ten_bit: hdr,
            rate_mode: Default::default(),
            bitrate_kbps: 0,
            color_range: Default::default(),
            color_space,
            hdr,
            effect_time_offset: start,
            clips,
            resume: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_doc::{Doc, at, video};

    /// Three clips of one file at 5, 6 and 8 seconds, each a second long,
    /// cut from 0, 10 and 20 seconds in, at half volume.
    fn selection() -> (Doc, Vec<ItemId>) {
        let mut doc = Doc::new();
        let media = doc.media(video("/original.mp4", 30.0));
        let sequence = doc.model().view.showing.expect("a timeline");
        let lane = doc
            .model()
            .lanes
            .index()
            .of(sequence)
            .next()
            .expect("a lane")
            .0;
        let mut ids = Vec::new();
        for (start, source) in [(5.0, 0.0), (6.0, 10.0), (8.0, 20.0)] {
            let id = doc.item(Command::AddClip {
                media,
                lane,
                start: at(start),
                ripple: false,
            });
            // On the lowest lane, a second long, from `source` on.
            let item = doc.model().items.get(id).expect("placed").as_ref().clone();
            let mut item = item;
            item.lane = lane;
            item.len = crate::test_doc::long(1.0);
            if let Content::Media(cut) = &mut item.content {
                cut.time = concat_doc::model::TimeMap::from_in(at(source));
            }
            item.sound.volume = concat_doc::model::Animated::fixed(0.5);
            let mut draft = doc.0.begin("set up", concat_doc::Origin(0));
            draft.items.replace(id, item).expect("replaced");
            doc.0 = draft.commit().expect("valid").into_document();
            ids.push(id);
        }
        (doc, ids)
    }

    #[test]
    fn render_keeps_source_cuts_audio_settings_and_timeline_gaps() {
        let (doc, ids) = selection();
        let plan = Consolidation::new(doc.model(), &ids).expect("a valid selection");
        let request = plan.request(
            doc.model(),
            Path::new("/project"),
            Path::new("/project/merged.mp4"),
        );
        let starts: Vec<f64> = request.clips.iter().map(|clip| clip.start).collect();
        assert_eq!(starts, [0.0, 1.0, 3.0]);
        let sources: Vec<f64> = request.clips.iter().map(|clip| clip.source_start).collect();
        assert_eq!(sources, [0.0, 10.0, 20.0]);
        assert!(
            request
                .clips
                .iter()
                .all(|clip| clip.volume == 0.5 && clip.has_audio == Some(true))
        );
        assert!((request.effect_time_offset - 5.0).abs() < 1e-9);
    }

    #[test]
    fn a_changed_clip_or_a_relinked_file_is_not_replaced() {
        let (mut doc, ids) = selection();
        let plan = Consolidation::new(doc.model(), &ids).expect("valid");
        let merged = video("/project/media/merged.mp4", 4.0);
        assert!(plan.replacement(doc.model(), merged.clone()).is_ok());
        let media = doc.model().media.ids().next().expect("a file");
        doc.apply(Command::UpdateMediaPath {
            media,
            path: "/relinked.mp4".to_owned(),
        });
        assert!(plan.replacement(doc.model(), merged).is_err());
    }

    #[test]
    fn one_clip_is_no_selection_to_merge() {
        let (doc, ids) = selection();
        let error = Consolidation::new(doc.model(), &ids[..1])
            .err()
            .expect("refused");
        assert!(error.contains("two or more"), "{error}");
    }
}
