// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Measuring how the camera moved, for the clips that are stabilized.
//!
//! A stabilized clip needs its media's movement measured over the stretch
//! of source it shows, and a little either side for the smoothing to read.
//! This is the job that measures it: the media decoded small and grey
//! along each stretch not measured yet, every frame tracked against the one
//! before (see `concat_vision::stabilize::track`), and the movement merged
//! into the media's cache in the project folder. Only the stretches clips
//! use are read: a clip of twenty seconds from a twenty-minute recording
//! costs twenty seconds, not twenty minutes.
//!
//! Nothing here is the correction itself. That is worked out from the
//! cache whenever the timeline is built, from the clip's own settings, so
//! moving a slider never runs this again; trimming a clip longer does, for
//! the new part only.
//!
//! One job at a time through a [`SingleFlight`], like the cutout's, and
//! guarded by a [`Tripwire`] so footage the app died measuring is not
//! measured again by itself on the next open.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use concat_core::time::Rational;
use concat_media::{DecodeOptions, Decoder, FrameSource};
use concat_project::model::{ClipKind, Project};
use concat_vision::stabilize::track::{Gray, Measured, Tracker};
use concat_vision::stabilize::{ANALYSIS_SIDE, FrameMotion, Motion, Similarity, Step, motion_dir};

use crate::jobs::{SingleFlight, Tripwire};

/// What one analysis covers: one media file, the stretches of it clips
/// show.
#[derive(Clone, Debug, PartialEq)]
pub struct StabilizeRequest {
    /// The project folder the movement is cached under.
    pub project: PathBuf,
    /// The media file to measure.
    pub media_path: String,
    /// The media's picture size, when the document knows it: what the
    /// analysis size is worked out from.
    pub media_size: Option<(u32, u32)>,
    /// The stretches of source, in seconds, to measure, margins included.
    pub ranges: Vec<(f64, f64)>,
}

/// Source read either side of what a clip shows, for the smoothing to
/// see where the camera was going: a second and a half at the clip's own
/// pace, more for a clip sped up.
fn margin(speed: f64) -> f64 {
    (1.5 * speed.max(1.0)).min(10.0)
}

/// The analysis service: the one-job slot.
pub struct Stabilizers {
    gate: Arc<SingleFlight>,
}

impl Default for Stabilizers {
    fn default() -> Self {
        Stabilizers::new()
    }
}

impl Stabilizers {
    /// A service with nothing running.
    pub fn new() -> Stabilizers {
        Stabilizers {
            gate: Arc::new(SingleFlight::new()),
        }
    }

    /// Whether an analysis is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }

    /// Asks the running analysis to stop after the frame in hand. What it
    /// measured so far is kept.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    /// What the active timeline's stabilized clips need measured: one
    /// request per media, covering every stretch its stabilized clips
    /// show, paired with the media's id. The window asks after every
    /// change and the API before an export; both get the same list.
    pub fn requests(project: &Project, project_dir: &Path) -> Vec<(String, StabilizeRequest)> {
        let mut wanted: Vec<(String, StabilizeRequest)> = Vec::new();
        for clip in &project.active().clips {
            if clip.stabilize.is_none() || clip.kind != ClipKind::Video {
                continue;
            }
            let Some(media) = project.media_by_id(&clip.media_id) else {
                continue;
            };
            let speed = clip.speed.max(0.0625);
            let pad = margin(speed);
            let range = (
                (clip.source_start - pad).max(0.0),
                clip.source_start + clip.duration * speed + pad,
            );
            match wanted.iter_mut().find(|(id, _)| *id == media.id) {
                Some((_, request)) => request.ranges.push(range),
                None => wanted.push((
                    media.id.clone(),
                    StabilizeRequest {
                        project: project_dir.to_path_buf(),
                        media_path: media.path.clone(),
                        media_size: media.width.zip(media.height),
                        ranges: vec![range],
                    },
                )),
            }
        }
        wanted
    }

    /// Where `request`'s movement is kept.
    pub fn dir(request: &StabilizeRequest) -> PathBuf {
        motion_dir(&request.project, &request.media_path)
    }

    /// Whether the app died measuring `request` last time, so it is not to
    /// be measured again by itself. See [`Tripwire`].
    pub fn tripped(request: &StabilizeRequest) -> bool {
        Tripwire::tripped(&Self::dir(request))
    }

    /// The stretches `request` still needs measured, without measuring.
    pub fn outstanding(request: &StabilizeRequest) -> Vec<(f64, f64)> {
        match Motion::read(&Self::dir(request)) {
            Some(motion) => motion.missing(&request.ranges),
            None => Motion::default().missing(&request.ranges),
        }
    }

    /// Measures what `request` is missing. Blocks for the whole run, so
    /// run it on its own thread; `progress` is told `0..=1` of the source
    /// seconds wanted as it goes. Returns how many frames were measured.
    pub fn analyse(
        &self,
        request: &StabilizeRequest,
        progress: &mut dyn FnMut(f32),
    ) -> Result<usize, String> {
        let job = self.gate.begin("stabilization analysis")?;
        let dir = Self::dir(request);
        let wanted = Self::outstanding(request);
        let total: f64 = wanted.iter().map(|(a, b)| b - a).sum();
        if wanted.is_empty() || total <= 0.0 {
            return Ok(0);
        }
        let _wire = Tripwire::arm(&dir);
        let mut motion = Motion::read(&dir)
            .map(|motion| (*motion).clone())
            .unwrap_or_default();
        let size = analysis_size(request)?;
        if motion.aspect <= 0.0 {
            motion.aspect = f64::from(size.0) / f64::from(size.1);
        }
        progress(0.0);

        let mut done_seconds = 0.0;
        let mut measured = 0usize;
        let mut tracker = Tracker::new();
        for &(from, to) in &wanted {
            // From a little before the stretch, so its first frame is one
            // already measured where there is one, and the stretch joins
            // what is there rather than starting afresh.
            let lead = (from - 0.1).max(0.0);
            let options = DecodeOptions::default()
                .starting_at(Rational::approximate(lead).unwrap_or(Rational::ZERO))
                .scaled_to(size.0, size.1);
            let mut decoder = Decoder::open(&request.media_path, &options)
                .map_err(|error| format!("{}: {error}", request.media_path))?;
            tracker.reset();
            let mut run: Vec<FrameMotion> = Vec::new();
            let mut reached = from;
            let mut cancelled = false;
            loop {
                if job.cancelled() {
                    cancelled = true;
                    break;
                }
                let Some(frame) = decoder.next_frame().map_err(|error| error.to_string())? else {
                    // The file ran out before its stated length: what was
                    // measured is what there is, and the rest counts as
                    // covered so it is not asked for again.
                    reached = to;
                    break;
                };
                let at = decoder
                    .position()
                    .map(|position| position.as_f64())
                    .unwrap_or_else(|| run.last().map_or(lead, |last| last.at + 1.0 / 30.0));
                let (step, movement) = match tracker.push(Gray::of_frame(&frame)) {
                    Measured::First => (Step::Start, Similarity::IDENTITY),
                    Measured::Moved(similarity) => (Step::Tracked, similarity),
                    Measured::Flat => (Step::Flat, Similarity::IDENTITY),
                };
                run.push(FrameMotion {
                    at,
                    step,
                    motion: movement,
                });
                measured += 1;
                reached = at.min(to);
                progress(((done_seconds + (reached - from).max(0.0)) / total).min(1.0) as f32);
                if at >= to {
                    break;
                }
            }
            if let Some(first) = run.first() {
                let span = (first.at.min(from), if cancelled { reached } else { to });
                motion.merge(&run, span);
                motion.write(&dir)?;
            } else if !cancelled {
                // Nothing to read in the stretch at all: past the end.
                motion.merge(&[], (from, to));
                motion.write(&dir)?;
            }
            if cancelled {
                return Err("stabilization analysis cancelled".to_owned());
            }
            done_seconds += to - from;
        }
        progress(1.0);
        Ok(measured)
    }
}

/// The size the analysis decodes `request`'s media at: its longest side
/// [`ANALYSIS_SIDE`], or its own when smaller, even on both sides. The
/// file is opened to ask when the document does not know.
fn analysis_size(request: &StabilizeRequest) -> Result<(u32, u32), String> {
    let (width, height) = match request.media_size.filter(|(w, h)| *w > 0 && *h > 0) {
        Some(size) => size,
        None => {
            let decoder =
                Decoder::open(&request.media_path, &DecodeOptions::default().limited_to(1))
                    .map_err(|error| format!("{}: {error}", request.media_path))?;
            (decoder.width(), decoder.height())
        }
    };
    if width == 0 || height == 0 {
        return Err(format!("{}: no picture to measure", request.media_path));
    }
    let longest = width.max(height);
    let scale = (f64::from(ANALYSIS_SIDE) / f64::from(longest)).min(1.0);
    let even = |side: u32| (((f64::from(side) * scale).round() as u32).max(16)) & !1;
    Ok((even(width), even(height)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_analysis_size_keeps_the_aspect_and_stays_even() {
        let request = |size| StabilizeRequest {
            project: PathBuf::new(),
            media_path: String::new(),
            media_size: Some(size),
            ranges: Vec::new(),
        };
        assert_eq!(analysis_size(&request((1920, 1080))), Ok((480, 270)));
        assert_eq!(analysis_size(&request((1080, 1920))), Ok((270, 480)));
        assert_eq!(analysis_size(&request((320, 240))), Ok((320, 240)));
    }

    #[test]
    fn a_faster_clip_reads_a_wider_margin() {
        assert_eq!(margin(1.0), 1.5);
        assert_eq!(margin(0.5), 1.5);
        assert_eq!(margin(4.0), 6.0);
        assert_eq!(margin(100.0), 10.0);
    }
}
