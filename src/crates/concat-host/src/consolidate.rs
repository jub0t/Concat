// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! A frozen selection rendered through the normal exporter before an atomic
//! `ConsolidateClips` edit. Source files are never rewritten.

use std::path::Path;

use concat_export::{ExportRequest, flatten::flatten_timeline_in};
use concat_project::commands::NewMedia;
use concat_project::model::{Clip, MediaItem, Project, VideoSettings};
use concat_project::{Command, consolidation_clips};

/// The exact clips and media a render reads. Rechecked before replacement so
/// editing, undo, relinking or switching timelines cannot adopt stale work.
pub struct Consolidation {
    timeline_id: String,
    video: VideoSettings,
    clips: Vec<Clip>,
    media: Vec<MediaItem>,
}

impl Consolidation {
    /// Captures a valid selection without changing the project.
    pub fn new(project: &Project, ids: &[String]) -> Result<Self, String> {
        let clips =
            consolidation_clips(project.active(), ids).map_err(|error| error.to_string())?;
        let media = clips
            .iter()
            .map(|clip| {
                project
                    .media_by_id(&clip.media_id)
                    .cloned()
                    .ok_or_else(|| "A selected clip's media is missing.".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            timeline_id: project.active().id.clone(),
            video: project.active().video,
            clips,
            media,
        })
    }

    /// The lane the replacement belongs on, for the window's lock check.
    pub fn track_id(&self) -> &str {
        &self.clips[0].track_id
    }

    /// The host-rendered media may replace the captured clips only while
    /// their inputs and the timeline's output settings still match.
    pub fn replacement(&self, project: &Project, item: NewMedia) -> Result<Command, String> {
        let ids: Vec<_> = self.clips.iter().map(|clip| clip.id.clone()).collect();
        if project.active().id != self.timeline_id
            || project.active().video != self.video
            || consolidation_clips(project.active(), &ids).ok().as_ref() != Some(&self.clips)
            || self
                .media
                .iter()
                .any(|media| project.media_by_id(&media.id) != Some(media))
        {
            return Err("The selected clips changed while merging. Please try again.".into());
        }
        Ok(Command::ConsolidateClips {
            clip_ids: ids,
            item,
        })
    }

    /// A render of the selection alone, moved to zero. Timeline gaps stay
    /// gaps; source gaps left by deleting pauses are never put back. Track
    /// visibility and mute stay live on the replacement rather than baking
    /// silence or black into a file that could later be unmuted/unhidden.
    pub fn request(&self, project: &Project, folder: &Path, output: &Path) -> ExportRequest {
        let mut alone = project.clone();
        let timeline = alone.active_mut();
        let start = self.clips[0].start;
        timeline.clips = self
            .clips
            .iter()
            .cloned()
            .map(|mut clip| {
                clip.start -= start;
                std::sync::Arc::new(clip)
            })
            .collect();
        for track in &mut timeline.tracks {
            track.visible = true;
            track.muted = false;
        }
        let mut clips = flatten_timeline_in(&alone, None, Some(folder));
        // The exporter rounds its end to frames. A hidden, silent marker
        // makes it round up for a source file: even a fractional last frame
        // must be available when the replacement is trimmed to the exact
        // original span. Do not stretch clips/keys/fades to achieve this.
        let end = clips
            .iter()
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max);
        let padded = (end * self.video.rate() - 1e-9).ceil() / self.video.rate();
        if padded > end + 1e-9 {
            let mut marker = concat_export::ExportClip::blank(
                concat_export::ClipKind::Layer,
                0.0,
                padded,
                alone.active().tracks.len(),
            );
            marker.hidden = true;
            marker.muted = true;
            marker.has_audio = Some(false);
            clips.push(marker);
        }
        let hdr = self.video.color_space.is_hdr();
        ExportRequest {
            output: output.to_string_lossy().into_owned(),
            canvas: self.video.canvas,
            width: self.video.width,
            height: self.video.height,
            rate_num: self.video.rate_num,
            rate_den: self.video.rate_den,
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
            color_space: self.video.color_space,
            hdr,
            effect_time_offset: start,
            clips,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::model::{ClipKind, MediaKind};

    fn selection() -> (Project, Vec<String>) {
        let mut project = Project::default();
        let track = project.active().tracks[0].id.clone();
        project.media.push(MediaItem {
            id: "m1".into(),
            path: "/original.mp4".into(),
            kind: MediaKind::Video,
            duration: Some(30.0),
            has_audio: true,
            ..Default::default()
        });
        let mut ids = Vec::new();
        for (id, start, source_start) in [("c1", 5.0, 0.0), ("c2", 6.0, 10.0), ("c3", 8.0, 20.0)] {
            let mut clip = Clip::blank(id, &track, ClipKind::Video, id, start, 1.0);
            clip.media_id = "m1".into();
            clip.source_start = source_start;
            clip.volume = 0.5;
            project.active_mut().clips.push(std::sync::Arc::new(clip));
            ids.push(id.to_owned());
        }
        (project, ids)
    }

    #[test]
    fn render_keeps_source_cuts_audio_settings_and_timeline_gaps() {
        let (project, ids) = selection();
        let plan = Consolidation::new(&project, &ids).unwrap();
        let request = plan.request(
            &project,
            Path::new("/project"),
            Path::new("/project/merged.mp4"),
        );
        assert_eq!(
            request
                .clips
                .iter()
                .map(|clip| clip.start)
                .collect::<Vec<_>>(),
            [0.0, 1.0, 3.0]
        );
        assert_eq!(
            request
                .clips
                .iter()
                .map(|clip| clip.source_start)
                .collect::<Vec<_>>(),
            [0.0, 10.0, 20.0]
        );
        assert!(
            request
                .clips
                .iter()
                .all(|clip| clip.volume == 0.5 && clip.has_audio == Some(true))
        );
    }

    #[test]
    fn rejects_stale_clips_and_relinked_sources() {
        let (mut project, ids) = selection();
        let plan = Consolidation::new(&project, &ids).unwrap();
        // Obtain a NewMedia independently of a real file for the adoption check.
        let item: NewMedia = serde_json::from_value(serde_json::json!({
            "path":"/merged.mp4", "name":"Merged", "duration":4.0,
            "kind":"video", "width":1920, "height":1080,
            "frameRate":30.0, "frameRateFraction":"30/1", "videoCodec":"h264",
            "audioCodec":"aac", "hasAudio":true
        }))
        .unwrap();
        assert!(plan.replacement(&project, item.clone()).is_ok());
        project.active_mut().clip_mut("c2").unwrap().source_start = 11.0;
        assert!(plan.replacement(&project, item.clone()).is_err());
        project.active_mut().clip_mut("c2").unwrap().source_start = 10.0;
        project.media[0].path = "/relinked.mp4".into();
        assert!(plan.replacement(&project, item).is_err());
    }
}
