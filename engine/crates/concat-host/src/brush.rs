// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Reading what a smart stroke lies on.
//!
//! A smart brush or smart eraser names a thing in one frame, and the
//! whole of it is kept or dropped. This is the job that finds the thing
//! and follows it: it decodes the frame the stroke was painted at, runs
//! the brush model's encoder on it and its decoder on the stroke's
//! points, and then walks the clip's analysed instants forward and
//! backward from there, each frame prompted from the region the frame
//! beside it had, so the region moves with its thing. Every region goes
//! beside the media's masks, one per instant, where `concat-vision`
//! paints it in. Where the thing is lost the rest of the run gets an
//! empty region: nothing kept, nothing dropped, and the grid complete so
//! the job is not asked again. The model's two files download on first
//! use, like the segmenters'.
//!
//! One stroke at a time through a [`SingleFlight`]; the window queues the
//! rest behind it. An embedding is kept for a few frames, so a second
//! stroke on the same frame costs the decoder alone.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use concat_core::frame::Frame;
use concat_core::time::{FrameRate, Rational};
use concat_media::{DecodeOptions, Decoder, FrameSource};
use concat_project::model::{Stroke, Subject};
use concat_vision::{Brush, Embedding, MASK_RATE, Mask, MaskStore, ModelId, mask_dir, models, region_dir};

use crate::cutout::Progress;
use crate::jobs::SingleFlight;

/// One stroke to read.
#[derive(Clone, Debug)]
pub struct RegionRequest {
    /// The project folder the masks are cached under.
    pub project: PathBuf,
    /// The media file the stroke was painted on.
    pub media_path: String,
    /// The media's picture size.
    pub media_size: (u32, u32),
    /// A still: its one frame, whatever the stroke's instant.
    pub still: bool,
    /// The cutout's subject, which names the masks' directory.
    pub subject: Subject,
    /// The stretches of source, in seconds, the clip shows: where the
    /// region is followed to.
    pub ranges: Vec<(f64, f64)>,
    /// The stroke, with its instant.
    pub stroke: Stroke,
}

impl RegionRequest {
    /// Where the regions go.
    pub fn dir(&self) -> PathBuf {
        region_dir(
            &mask_dir(&self.project, &self.media_path, self.subject),
            &self.stroke,
        )
    }

    /// Whether any instant the clip shows still wants a region.
    pub fn outstanding(&self) -> bool {
        if !self.stroke.is_smart() || self.stroke.at.is_none() {
            return false;
        }
        let store = MaskStore::open(&self.dir());
        if self.still {
            return store.is_empty();
        }
        !missing(&store, &self.ranges).is_empty()
    }
}

/// How many frames' embeddings stay in memory. Each is a few megabytes;
/// a few frames is a session of strokes on one shot.
const EMBEDDINGS: usize = 6;
/// The backward walk decodes this many frames at a time and holds their
/// embeddings, since a decoder only runs forward.
const BACKWARD_CHUNK: usize = 8;

/// A frame read lately: its media and source millisecond, and what the
/// encoder made of it.
type Read = ((String, u64), Arc<Embedding>);

/// The brush service: the model, loaded once, and the one-job slot.
pub struct Brushes {
    gate: Arc<SingleFlight>,
    data: PathBuf,
    brush: Mutex<Option<Arc<Brush>>>,
    /// Frames read lately, by media and source millisecond.
    embeddings: Mutex<VecDeque<Read>>,
}

impl Brushes {
    /// A service with nothing loaded yet, keeping its models under `data`.
    pub fn new(data: &Path) -> Brushes {
        Brushes {
            gate: Arc::new(SingleFlight::new()),
            data: data.to_path_buf(),
            brush: Mutex::new(None),
            embeddings: Mutex::new(VecDeque::new()),
        }
    }

    /// Whether a stroke is being read.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }

    /// Asks the running job to stop.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    fn brush(
        &self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Arc<Brush>, String> {
        if let Some(loaded) = self.brush.lock().map_err(|_| "brush slot poisoned")?.as_ref() {
            return Ok(Arc::clone(loaded));
        }
        let encoder = models::model_file(&self.data, ModelId::BrushEncoder);
        let decoder = models::model_file(&self.data, ModelId::BrushDecoder);
        if !models::installed(&self.data, ModelId::BrushEncoder) {
            crate::cutout::fetch(ModelId::BrushEncoder, &encoder, cancel, progress)?;
        }
        if !models::installed(&self.data, ModelId::BrushDecoder) {
            crate::cutout::fetch(ModelId::BrushDecoder, &decoder, cancel, progress)?;
        }
        let brush = Arc::new(Brush::load(&encoder, &decoder)?);
        *self.brush.lock().map_err(|_| "brush slot poisoned")? = Some(Arc::clone(&brush));
        Ok(brush)
    }

    /// The encoder's reading of `frame` at `millis` of `media`, from the
    /// recent few when it is there.
    fn embed(
        &self,
        brush: &Brush,
        media: &str,
        millis: u64,
        frame: &Frame,
    ) -> Result<Arc<Embedding>, String> {
        let key = (media.to_owned(), millis);
        if let Some(held) = self
            .embeddings
            .lock()
            .map_err(|_| "embeddings poisoned")?
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, embedding)| Arc::clone(embedding))
        {
            return Ok(held);
        }
        let embedding = Arc::new(brush.embed(frame)?);
        if let Ok(mut held) = self.embeddings.lock() {
            held.push_back((key, Arc::clone(&embedding)));
            while held.len() > EMBEDDINGS {
                held.pop_front();
            }
        }
        Ok(embedding)
    }

    /// Reads the regions under `request`'s stroke, along the clip, and
    /// writes them. Blocks for the whole run, so run it on its own
    /// thread; `progress` is called as it goes.
    pub fn read(
        &self,
        request: &RegionRequest,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(), String> {
        let job = self.gate.begin("brush reading")?;
        let cancel = job.cancel_handle();
        let Some(at) = request.stroke.at else {
            return Err("the stroke has no instant".to_owned());
        };
        let mut store = MaskStore::open(&request.dir());
        let brush = self.brush(&cancel, progress)?;
        progress(Progress::Analysing(0.0));

        let (width, height) = Brush::input_size(request.media_size.0, request.media_size.1);
        let step = 1000 / u64::from(MASK_RATE);
        let stopped = || cancel.load(Ordering::Relaxed);
        let open = |from: u64, count: usize| -> Result<Decoder, String> {
            let mut options = DecodeOptions::default()
                .scaled_to(width, height)
                .limited_to(count as u64);
            if !request.still {
                options = options
                    .starting_at(Rational::new(from as i64, 1000))
                    .at_rate(FrameRate::new(Rational::new(i64::from(MASK_RATE), 1)));
            }
            Decoder::open(&request.media_path, &options).map_err(|error| error.to_string())
        };

        // The seed: the thing under the stroke, in the frame it was
        // painted on, held to the analysis grid.
        let seed_ms = if request.still {
            0
        } else {
            ((at * 1000.0) / step as f64).round() as u64 * step
        };
        if store.mask_at(seed_ms as f64 / 1000.0).is_none() || request.still && store.is_empty() {
            let frame = open(seed_ms, 1)?
                .next_frame()
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("{}: no picture at the stroke", request.media_path))?;
            if stopped() {
                return Err("brush reading cancelled".to_owned());
            }
            let embedding = self.embed(&brush, &request.media_path, seed_ms, &frame)?;
            let region = brush.region(&embedding, &request.stroke.points)?;
            store.put(seed_ms, &region)?;
        }
        if request.still {
            progress(Progress::Analysing(1.0));
            return Ok(());
        }

        // Then the rest of what the clip shows, followed from the seed:
        // forward in one decode, backward in chunks.
        let wanted = missing(&store, &request.ranges);
        let total = wanted.len();
        if total == 0 {
            progress(Progress::Analysing(1.0));
            return Ok(());
        }
        let mut done = 0usize;
        let lost_after = |store: &mut MaskStore, instants: &[u64]| -> Result<(), String> {
            for &millis in instants {
                store.put(millis, &Mask::filled(1, 1, 0))?;
            }
            Ok(())
        };

        let forward: Vec<u64> = wanted.iter().copied().filter(|&m| m > seed_ms).collect();
        for run in runs(&forward, step) {
            let mut decoder = open(run[0], run.len())?;
            let mut previous = store
                .mask_at((run[0] - step) as f64 / 1000.0)
                .ok_or_else(|| "brush: the region before is missing".to_owned())?;
            for (index, &millis) in run.iter().enumerate() {
                if stopped() {
                    return Err("brush reading cancelled".to_owned());
                }
                let Some(frame) = decoder.next_frame().map_err(|error| error.to_string())? else {
                    lost_after(&mut store, &run[index..])?;
                    done += run.len() - index;
                    break;
                };
                let embedding = self.embed(&brush, &request.media_path, millis, &frame)?;
                match brush.track(&embedding, &previous)? {
                    Some(region) => {
                        store.put(millis, &region)?;
                        previous = Arc::new(region);
                        done += 1;
                        progress(Progress::Analysing(done as f32 / total as f32));
                    }
                    None => {
                        lost_after(&mut store, &run[index..])?;
                        done += run.len() - index;
                        break;
                    }
                }
            }
        }

        let backward: Vec<u64> = wanted.iter().copied().filter(|&m| m < seed_ms).collect();
        for run in runs(&backward, step) {
            // Nearest the seed first: the run's tail, in chunks, each
            // decoded forward and walked backward.
            let mut chunk_end = run.len();
            let mut lost = false;
            while chunk_end > 0 && !lost {
                let chunk_start = chunk_end.saturating_sub(BACKWARD_CHUNK);
                let chunk = &run[chunk_start..chunk_end];
                let mut decoder = open(chunk[0], chunk.len())?;
                let mut frames: Vec<(u64, Frame)> = Vec::with_capacity(chunk.len());
                for &millis in chunk {
                    if stopped() {
                        return Err("brush reading cancelled".to_owned());
                    }
                    match decoder.next_frame().map_err(|error| error.to_string())? {
                        Some(frame) => frames.push((millis, frame)),
                        None => break,
                    }
                }
                for &(millis, ref frame) in frames.iter().rev() {
                    let Some(previous) = store.mask_at((millis + step) as f64 / 1000.0) else {
                        lost = true;
                        break;
                    };
                    let embedding = self.embed(&brush, &request.media_path, millis, frame)?;
                    match brush.track(&embedding, &previous)? {
                        Some(region) => {
                            store.put(millis, &region)?;
                            done += 1;
                            progress(Progress::Analysing(done as f32 / total as f32));
                        }
                        None => {
                            lost = true;
                            break;
                        }
                    }
                }
                chunk_end = chunk_start;
            }
            // Whatever the walk did not reach - lost, or the file short -
            // is empty, so the grid is whole.
            let left: Vec<u64> = run
                .iter()
                .copied()
                .filter(|&m| store.mask_at(m as f64 / 1000.0).is_none())
                .collect();
            done += left.len();
            lost_after(&mut store, &left)?;
        }
        progress(Progress::Analysing(1.0));
        Ok(())
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
