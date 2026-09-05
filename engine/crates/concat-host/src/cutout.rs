// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Finding the masks a cutout is made of.
//!
//! A clip with a cutout needs a mask for every source instant it shows, on
//! the analysis grid `concat-vision` reads by. This is the job that fills
//! the gaps: it decodes the media at the model's size along each missing
//! stretch, runs the model on several frames at once, and writes what it
//! finds into the media's mask store in the project folder. A still is one
//! frame; footage is [`concat_vision::MASK_RATE`] frames a second of source.
//!
//! One job at a time through a [`SingleFlight`], like every long job the
//! host runs; the window queues the next media behind it. The model loads
//! on first use and stays loaded.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use concat_core::frame::Frame;
use concat_core::time::{FrameRate, Rational};
use concat_media::{DecodeOptions, Decoder, FrameSource};
use concat_vision::{MASK_RATE, MODEL_SIZE, Mask, MaskStore, Segmenter, mask_dir};

use crate::jobs::{Job, SingleFlight};

/// What one analysis covers.
#[derive(Clone, Debug)]
pub struct AnalyseRequest {
    /// The project folder the masks are cached under.
    pub project: PathBuf,
    /// The media file to analyse.
    pub media_path: String,
    /// A still: one frame answers for every instant.
    pub still: bool,
    /// The stretches of source, in seconds, that clips show. Instants
    /// already analysed inside them are skipped.
    pub ranges: Vec<(f64, f64)>,
}

/// The analysis service: the model, loaded once, and the one-job slot.
pub struct Cutouts {
    gate: Arc<SingleFlight>,
    segmenter: OnceLock<Result<Arc<Segmenter>, String>>,
}

impl Default for Cutouts {
    fn default() -> Self {
        Self::new()
    }
}

impl Cutouts {
    /// A service with nothing loaded yet.
    pub fn new() -> Cutouts {
        Cutouts {
            gate: Arc::new(SingleFlight::new()),
            segmenter: OnceLock::new(),
        }
    }

    /// Whether an analysis is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }

    /// Asks the running analysis to stop after the frames in hand.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    fn segmenter(&self) -> Result<Arc<Segmenter>, String> {
        self.segmenter
            .get_or_init(|| Segmenter::load().map(Arc::new))
            .clone()
    }

    /// How many instants `request` still needs, without doing anything.
    pub fn outstanding(request: &AnalyseRequest) -> usize {
        let store = MaskStore::open(&mask_dir(&request.project, &request.media_path));
        if request.still {
            return usize::from(store.is_empty());
        }
        missing(&store, &request.ranges).len()
    }

    /// Fills the masks `request` is missing. Blocks for the whole run, so
    /// run it on its own thread; `progress` is called with `0..=1` as it
    /// goes. Returns how many masks were written.
    pub fn analyse(
        &self,
        request: &AnalyseRequest,
        progress: &mut dyn FnMut(f32),
    ) -> Result<usize, String> {
        let job = self.gate.begin("cutout analysis")?;
        let segmenter = self.segmenter()?;
        let mut store = MaskStore::open(&mask_dir(&request.project, &request.media_path));

        if request.still {
            if !store.is_empty() {
                return Ok(0);
            }
            let options = DecodeOptions::default()
                .scaled_to(MODEL_SIZE, MODEL_SIZE)
                .limited_to(1);
            let mut decoder =
                Decoder::open(&request.media_path, &options).map_err(|error| error.to_string())?;
            let frame = decoder
                .next_frame()
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("{}: no picture to analyse", request.media_path))?;
            store.put(0, &segmenter.mask(&frame)?)?;
            progress(1.0);
            return Ok(1);
        }

        let wanted = missing(&store, &request.ranges);
        let total = wanted.len();
        if total == 0 {
            return Ok(0);
        }
        let step_ms = 1000 / u64::from(MASK_RATE);
        let workers = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
            .clamp(1, 8);
        let mut done = 0usize;

        // Each unbroken run of missing instants is one pass of a decoder
        // paced to the analysis grid, so no frame is sought twice.
        for run in runs(&wanted, step_ms) {
            let start = Rational::new(run[0] as i64, 1000);
            let options = DecodeOptions::default()
                .starting_at(start)
                .scaled_to(MODEL_SIZE, MODEL_SIZE)
                .at_rate(FrameRate::new(Rational::new(i64::from(MASK_RATE), 1)))
                .limited_to(run.len() as u64);
            let mut decoder =
                Decoder::open(&request.media_path, &options).map_err(|error| error.to_string())?;
            let mut next = 0usize;
            while next < run.len() {
                if job.cancelled() {
                    return Err("cutout analysis cancelled".to_owned());
                }
                // A batch of frames, one per core, found together.
                let mut batch: Vec<(u64, Frame)> = Vec::with_capacity(workers);
                while batch.len() < workers && next < run.len() {
                    match decoder.next_frame().map_err(|error| error.to_string())? {
                        Some(frame) => batch.push((run[next], frame)),
                        // The file ran out before its stated length: what
                        // was found is what there is.
                        None => break,
                    }
                    next += 1;
                }
                if batch.is_empty() {
                    break;
                }
                for (millis, mask) in find_masks(&segmenter, &batch, &job)? {
                    store.put(millis, &mask)?;
                    done += 1;
                }
                progress(done as f32 / total as f32);
            }
        }
        progress(1.0);
        Ok(done)
    }
}

/// Every instant the ranges need that the store lacks, ascending, once.
fn missing(store: &MaskStore, ranges: &[(f64, f64)]) -> Vec<u64> {
    let mut wanted: Vec<u64> = ranges
        .iter()
        .flat_map(|&(from, to)| store.missing(from, to))
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    wanted
}

/// The instants split into runs a step apart.
fn runs(instants: &[u64], step: u64) -> Vec<Vec<u64>> {
    let mut out: Vec<Vec<u64>> = Vec::new();
    for &at in instants {
        match out.last_mut() {
            Some(run) if run.last().is_some_and(|&last| last + step == at) => run.push(at),
            _ => out.push(vec![at]),
        }
    }
    out
}

/// The masks of a batch of frames, one thread each.
fn find_masks(
    segmenter: &Arc<Segmenter>,
    batch: &[(u64, Frame)],
    job: &Job,
) -> Result<Vec<(u64, Mask)>, String> {
    if job.cancelled() {
        return Err("cutout analysis cancelled".to_owned());
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = batch
            .iter()
            .map(|(millis, frame)| {
                let segmenter = Arc::clone(segmenter);
                scope.spawn(move || segmenter.mask(frame).map(|mask| (*millis, mask)))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| "cutout analysis panicked".to_owned())?
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_group_into_runs_a_step_apart() {
        assert_eq!(
            runs(&[0, 100, 200, 500, 600, 900], 100),
            vec![vec![0, 100, 200], vec![500, 600], vec![900]]
        );
        assert!(runs(&[], 100).is_empty());
    }
}
