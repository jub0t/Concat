// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A song's beats in the editor: heard once in the background when a sound
//! file is in the project, kept in the project's cache, drawn as marks
//! along its clips and snapped to. See `concat_speech::beats` for the
//! listening.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use concat_speech::beats::{Beats, VERSION};

use super::*;

/// One file's beats, as the job hands them back.
pub(crate) struct HeardBeats {
    /// The media item they are of.
    pub id: String,
    /// `None` when the file could not be listened to.
    pub beats: Option<Arc<Beats>>,
}

/// The cache entry's name: the path, and the file's size and change time
/// so a file replaced in place is heard again, and the analysis version so
/// a better one is too.
fn beats_key(path: &str) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let changed = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    Some(format!(
        "{:016x}-{}-{changed}-v{VERSION}.beats",
        concat_host::media::fnv1a(path.as_bytes()),
        meta.len()
    ))
}

/// Hears `path`'s beats, from the project's cache when they are there.
pub(crate) fn heard_beats(id: String, path: String, project: String) -> HeardBeats {
    let cached =
        beats_key(&path).and_then(|key| concat_host::media::artwork_file(&project, &key).ok());
    if let Some(file) = &cached
        && let Ok(bytes) = std::fs::read(file)
        && let Some(beats) = Beats::decode(&bytes)
    {
        return HeardBeats {
            id,
            beats: Some(Arc::new(beats)),
        };
    }
    let begun = std::time::Instant::now();
    let heard = concat_speech::beats::analyse_file(
        std::path::Path::new(&path),
        None,
        &AtomicBool::new(false),
        |_| {},
    );
    match heard {
        Ok(beats) => {
            log::info!(
                "beats: {} heard in {}: {} beats, {}",
                file_name_of(&path),
                concat_log::span(begun.elapsed()),
                beats.beats.len(),
                beats
                    .tempo
                    .map_or("no tempo".to_owned(), |tempo| format!("{tempo:.1} BPM"))
            );
            if let Some(file) = &cached {
                if let Some(parent) = file.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(file, beats.encode());
            }
            HeardBeats {
                id,
                beats: Some(Arc::new(beats)),
            }
        }
        Err(error) => {
            log::warn!("beats: {} could not be heard: {error}", file_name_of(&path));
            HeardBeats { id, beats: None }
        }
    }
}

impl Studio {
    /// Starts hearing the beats of every sound file in the project that
    /// has none yet, one job each on the long lane: a song is a couple of
    /// seconds of every core, and nothing waits on it.
    pub(super) fn request_beats(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let project = session.path().to_owned();
        let wanted: Vec<(String, String)> = self
            .project()
            .media
            .iter()
            .filter(|item| {
                item.kind == model::MediaKind::Audio
                    && !item.placeholder
                    && !item.path.is_empty()
                    && !self.beats.contains_key(&item.id)
                    && !self.beats_pending.contains(&item.id)
                    && !self.beats_failed.contains(&item.id)
            })
            .map(|item| (item.id.clone(), item.path.clone()))
            .collect();
        for (id, path) in wanted {
            self.beats_pending.insert(id.clone());
            let project = project.clone();
            crate::host::spawn_long_in_project(
                move || heard_beats(id, path, project),
                |studio, _, _, heard: HeardBeats| studio.beats_landed(heard),
            );
        }
    }

    fn beats_landed(&mut self, heard: HeardBeats) {
        self.beats_pending.remove(&heard.id);
        match heard.beats {
            Some(beats) => {
                self.beats.insert(heard.id, beats);
            }
            None => {
                self.beats_failed.insert(heard.id);
            }
        }
    }

    /// The beats a clip plays, as timeline seconds and whether each starts
    /// a bar: its file's beats inside its window, through its speed. None
    /// for a clip with a speed curve, until the one source-to-timeline map
    /// (`reports/Beat detection 2026-11-19.md`) carries curves too: no
    /// marks rather than marks in the wrong place.
    pub(super) fn clip_beats(&self, clip: &Clip) -> Vec<(f64, bool)> {
        if clip.kind != model::ClipKind::Audio || clip.speed_curve.is_some() || clip.speed <= 0.0 {
            return Vec::new();
        }
        let Some(beats) = self.beats.get(&clip.media_id) else {
            return Vec::new();
        };
        placed(
            beats,
            clip.start,
            clip.source_start,
            clip.duration,
            clip.speed,
        )
    }

    /// A clip's beat marks as path commands in a 1x1 box its width: the
    /// beats, then the first beats of bars, each a vertical tick on the
    /// rail along the clip's floor. How much is drawn follows the zoom, as
    /// a beat grid does: every beat while beats are [`LEAST_GAP`] pixels
    /// apart or more, else the bars alone, else every fourth bar - never a
    /// wall of ticks that reads as a second waveform.
    pub(super) fn beat_marks(&self, clip: &Clip) -> (SharedString, SharedString) {
        if clip.duration <= 0.0 {
            return Default::default();
        }
        let placed = self.clip_beats(clip);
        let (every, bars) = marks_at(&placed, f64::from(self.lanes.seconds_per_pixel));
        let path = |times: &[f64]| -> SharedString {
            let mut path = String::new();
            for time in times {
                let x = ((time - clip.start) / clip.duration).clamp(0.0, 1.0);
                path.push_str(&format!("M {x:.5} 0 L {x:.5} 1 "));
            }
            path.into()
        };
        (path(&every), path(&bars))
    }

    /// Every beat of the timeline's sound clips, in timeline seconds: what
    /// a dragged edge, a split or the playhead snaps to beside the cuts.
    pub(super) fn timeline_beats(&self) -> Vec<f64> {
        self.timeline()
            .clips
            .iter()
            .filter(|clip| clip.muted != Some(true))
            .flat_map(|clip| self.clip_beats(clip).into_iter().map(|(time, _)| time))
            .collect()
    }
}

/// `beats` of a file as a clip at `start` plays them: those from
/// `source_start` for `duration` timeline seconds at `speed`, in timeline
/// seconds, with whether each starts a bar.
fn placed(
    beats: &Beats,
    start: f64,
    source_start: f64,
    duration: f64,
    speed: f64,
) -> Vec<(f64, bool)> {
    let to = source_start + duration * speed;
    beats
        .beats
        .iter()
        .filter(|beat| beat.time >= source_start && beat.time < to)
        .map(|beat| (start + (beat.time - source_start) / speed, beat.downbeat))
        .collect()
}

/// The fewest pixels between two ticks that still read as two.
const LEAST_GAP: f64 = 6.0;

/// Which of `placed` beats to draw at `seconds_per_pixel`: the plain
/// beats (none when they would crowd) and the bars' first beats (every
/// fourth when even those would). Gaps are the beats' own median, so a
/// skipped beat does not change the level.
fn marks_at(placed: &[(f64, bool)], seconds_per_pixel: f64) -> (Vec<f64>, Vec<f64>) {
    let median_gap = |times: &[f64]| {
        let mut gaps: Vec<f64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
        gaps.sort_by(f64::total_cmp);
        gaps.get(gaps.len() / 2).copied()
    };
    let pixels =
        |gap: Option<f64>| gap.map_or(f64::INFINITY, |gap| gap / seconds_per_pixel.max(1e-9));
    let all: Vec<f64> = placed.iter().map(|(time, _)| *time).collect();
    let bars: Vec<f64> = placed
        .iter()
        .filter(|(_, bar)| *bar)
        .map(|(time, _)| *time)
        .collect();
    if pixels(median_gap(&all)) >= LEAST_GAP {
        let plain = placed
            .iter()
            .filter(|(_, bar)| !*bar)
            .map(|(time, _)| *time)
            .collect();
        return (plain, bars);
    }
    if pixels(median_gap(&bars)) >= LEAST_GAP {
        return (Vec::new(), bars);
    }
    (Vec::new(), bars.into_iter().step_by(4).collect())
}

fn file_name_of(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(
        || path.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use concat_speech::beats::{Beat, Beats};

    fn every_half_second() -> Beats {
        Beats {
            beats: (0..20)
                .map(|index| Beat {
                    time: 0.25 + index as f64 * 0.5,
                    confidence: 0.9,
                    downbeat: index % 4 == 0,
                })
                .collect(),
            tempo: Some(120.0),
            seconds: 10.0,
        }
    }

    /// A clip plays the beats inside its trim, where it sits: trimmed in
    /// by a second and placed at 10 s, the beat at 1.25 s of the file is
    /// at 10.25 s; at double speed the gaps halve; beats outside the trim
    /// are not the clip's.
    #[test]
    fn a_clip_plays_the_beats_inside_its_trim() {
        let beats = every_half_second();
        let plain = super::placed(&beats, 10.0, 1.0, 2.0, 1.0);
        assert_eq!(
            plain,
            vec![
                (10.25, false),
                (10.75, false),
                (11.25, true),
                (11.75, false)
            ]
        );
        let fast = super::placed(&beats, 0.0, 0.0, 2.0, 2.0);
        assert_eq!(fast.len(), 8);
        assert!((fast[1].0 - fast[0].0 - 0.25).abs() < 1e-12);
        assert!(super::placed(&beats, 0.0, 20.0, 5.0, 1.0).is_empty());
    }

    /// The marks thin out with the zoom: every beat when beats are six
    /// pixels apart or more, the bars alone when only they are, every
    /// fourth bar when even bars would crowd.
    #[test]
    fn the_marks_thin_out_with_the_zoom() {
        let placed = super::placed(&every_half_second(), 0.0, 0.0, 10.0, 1.0);
        // 0.5 s a beat, 2 s a bar.
        let (plain, bars) = super::marks_at(&placed, 0.05); // 10 px a beat
        assert_eq!((plain.len(), bars.len()), (15, 5));
        let (plain, bars) = super::marks_at(&placed, 0.2); // 2.5 px a beat, 10 a bar
        assert_eq!((plain.len(), bars.len()), (0, 5));
        let (plain, bars) = super::marks_at(&placed, 1.0); // 2 px a bar
        assert_eq!((plain.len(), bars.len()), (0, 2));
    }
}
