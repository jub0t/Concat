// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Reading what a smart stroke lies on.
//!
//! A smart brush or smart eraser names a thing in one frame, and the
//! whole of it is kept or dropped. This is the job that finds the thing:
//! it decodes the frame the stroke was painted at, runs the brush model's
//! encoder on it - once per frame, the embedding kept for the next stroke
//! on the same one - and its decoder on the stroke's points, and writes
//! the region beside the media's masks, where `concat-vision` paints it
//! in. The model's two files download on first use, like the segmenters'.
//!
//! One region at a time through a [`SingleFlight`]; the window queues the
//! rest behind it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use concat_core::time::Rational;
use concat_media::{DecodeOptions, Decoder, FrameSource};
use concat_project::model::{Stroke, Subject};
use concat_vision::{Brush, Embedding, ModelId, mask_dir, models, region_file};

use crate::cutout::Progress;
use crate::jobs::SingleFlight;

/// One stroke to read.
#[derive(Clone, Debug)]
pub struct RegionRequest {
    /// The project folder the masks are cached under.
    pub project: PathBuf,
    /// The media file the stroke was painted on.
    pub media_path: String,
    /// A still: its one frame, whatever the stroke's instant.
    pub still: bool,
    /// The cutout's subject, which names the masks' directory.
    pub subject: Subject,
    /// The stroke, with its instant.
    pub stroke: Stroke,
}

impl RegionRequest {
    /// Where the region goes.
    pub fn file(&self) -> PathBuf {
        region_file(
            &mask_dir(&self.project, &self.media_path, self.subject),
            &self.stroke,
        )
    }

    /// Whether the region is still to be read.
    pub fn outstanding(&self) -> bool {
        self.stroke.is_smart() && self.stroke.at.is_some() && !self.file().is_file()
    }
}

/// How many frames' embeddings stay in memory. Each is a few megabytes;
/// a few frames is a session of strokes on one shot.
const EMBEDDINGS: usize = 6;

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

    /// Whether a region is being read.
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

    /// Reads the region under `request`'s stroke and writes it. Blocks
    /// for the whole run, so run it on its own thread.
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
        let file = request.file();
        if file.is_file() {
            return Ok(());
        }
        let brush = self.brush(&cancel, progress)?;
        progress(Progress::Analysing(0.0));

        let millis = if request.still {
            0
        } else {
            (at * 1000.0).round() as u64
        };
        let key = (request.media_path.clone(), millis);
        let held = self
            .embeddings
            .lock()
            .map_err(|_| "embeddings poisoned")?
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, embedding)| Arc::clone(embedding));
        let embedding = match held {
            Some(embedding) => embedding,
            None => {
                let facts = concat_media::probe(&request.media_path)
                    .map_err(|error| error.to_string())?;
                let video = facts.require_video().map_err(|error| error.to_string())?;
                let (w, h) = Brush::input_size(video.width, video.height);
                let mut options = DecodeOptions::default().scaled_to(w, h).limited_to(1);
                if !request.still {
                    options = options.starting_at(Rational::new(millis as i64, 1000));
                }
                let mut decoder = Decoder::open(&request.media_path, &options)
                    .map_err(|error| error.to_string())?;
                let frame = decoder
                    .next_frame()
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("{}: no picture at the stroke", request.media_path))?;
                if cancel.load(Ordering::Relaxed) {
                    return Err("brush reading cancelled".to_owned());
                }
                let embedding = Arc::new(brush.embed(&frame)?);
                if let Ok(mut held) = self.embeddings.lock() {
                    held.push_back((key, Arc::clone(&embedding)));
                    while held.len() > EMBEDDINGS {
                        held.pop_front();
                    }
                }
                embedding
            }
        };
        progress(Progress::Analysing(0.5));
        let region = brush.region(&embedding, &request.stroke.points)?;
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        std::fs::write(&file, region.to_png())
            .map_err(|error| format!("could not write {}: {error}", file.display()))?;
        progress(Progress::Analysing(1.0));
        Ok(())
    }
}
