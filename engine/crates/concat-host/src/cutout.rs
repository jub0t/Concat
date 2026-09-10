// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Finding the masks a cutout is made of.
//!
//! A clip with a cutout needs a mask for every source instant it shows, on
//! the analysis grid `concat-vision` reads by. This is the job that fills
//! the gaps: it picks the model the clip's subject asks for, fetches it if
//! it is not on disk yet, decodes the media at the model's size along each
//! missing stretch, runs the model frame by frame, and writes what it
//! finds into the media's mask store in the project folder. A still is
//! one frame; footage is [`concat_vision::MASK_RATE`] frames a second of
//! source.
//!
//! Which model: the person model for a person, the object model for an
//! object, and for a subject left on automatic the person model is tried
//! on the first frame and the object model takes over when it finds no
//! one. The choice is written into the store, so the rest of the run and
//! every later one agree. With nothing downloaded and no network, the
//! compiled-in person model answers instead.
//!
//! One job at a time through a [`SingleFlight`], like every long job the
//! host runs; the window queues the next media behind it. A model loads
//! on first use and stays loaded.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use concat_core::time::{FrameRate, Rational};
use concat_media::{DecodeOptions, Decoder, FrameSource};
use concat_project::model::{MediaKind, Project, Subject};
use concat_vision::segment::Kind;
use concat_vision::{MASK_RATE, MaskStore, ModelId, Segmenter, mask_dir, models};

use crate::jobs::SingleFlight;

/// What one analysis covers.
#[derive(Clone, Debug)]
pub struct AnalyseRequest {
    /// The project folder the masks are cached under.
    pub project: PathBuf,
    /// The media file to analyse.
    pub media_path: String,
    /// The media's picture size, for the person model's working size.
    pub media_size: (u32, u32),
    /// A still: one frame answers for every instant.
    pub still: bool,
    /// What to keep.
    pub subject: Subject,
    /// The stretches of source, in seconds, that clips show. Instants
    /// already analysed inside them are skipped.
    pub ranges: Vec<(f64, f64)>,
}

/// How far an analysis has got, as it reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    /// A model is being downloaded: bytes so far, of about this many.
    Fetching {
        /// Bytes received.
        received: u64,
        /// Bytes expected.
        total: u64,
    },
    /// Masks are being found: `0..=1` of the instants wanted.
    Analysing(f32),
}

/// The analysis service: the models, loaded once each, and the one-job
/// slot.
pub struct Cutouts {
    gate: Arc<SingleFlight>,
    /// The app's data directory, where downloaded models live.
    data: PathBuf,
    selfie: OnceLock<Result<Arc<Segmenter>, String>>,
    person: Mutex<Option<Arc<Segmenter>>>,
    object: Mutex<Option<Arc<Segmenter>>>,
}

impl Cutouts {
    /// A service with nothing loaded yet, keeping its models under `data`.
    pub fn new(data: &Path) -> Cutouts {
        Cutouts {
            gate: Arc::new(SingleFlight::new()),
            data: data.to_path_buf(),
            selfie: OnceLock::new(),
            person: Mutex::new(None),
            object: Mutex::new(None),
        }
    }

    /// Whether an analysis is running.
    pub fn is_busy(&self) -> bool {
        self.gate.is_busy()
    }

    /// Asks the running analysis to stop after the frame in hand.
    pub fn cancel(&self) {
        self.gate.cancel();
    }

    /// Whether a downloadable model is on disk.
    pub fn installed(&self, id: ModelId) -> bool {
        models::installed(&self.data, id)
    }

    fn selfie(&self) -> Result<Arc<Segmenter>, String> {
        self.selfie
            .get_or_init(|| Segmenter::selfie().map(Arc::new))
            .clone()
    }

    /// The person or object model, fetched first when it is not on disk.
    fn downloaded(
        &self,
        id: ModelId,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Arc<Segmenter>, String> {
        let slot = match id {
            ModelId::Person => &self.person,
            ModelId::Object => &self.object,
            _ => return Err("not a segmenting model".to_owned()),
        };
        if let Some(loaded) = slot.lock().map_err(|_| "model slot poisoned")?.as_ref() {
            return Ok(Arc::clone(loaded));
        }
        let file = models::model_file(&self.data, id);
        if !models::installed(&self.data, id) {
            fetch(id, &file, cancel, progress)?;
        }
        let segmenter = Arc::new(match id {
            ModelId::Person => Segmenter::person(&file)?,
            _ => Segmenter::object(&file)?,
        });
        *slot.lock().map_err(|_| "model slot poisoned")? = Some(Arc::clone(&segmenter));
        Ok(segmenter)
    }

    /// The person model, or the compiled-in one when the download cannot
    /// happen: a cutout with no network is a worse cutout, not none.
    fn person_or_selfie(
        &self,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Arc<Segmenter>, String> {
        match self.downloaded(ModelId::Person, cancel, progress) {
            Ok(model) => Ok(model),
            Err(error) if error.contains("cancelled") => Err(error),
            Err(_) => self.selfie(),
        }
    }

    /// What the active timeline's cutouts need analysed: one request per
    /// media and subject, since two clips of one file that keep different
    /// things need different masks, each covering the stretches of source
    /// its clips show. Paired with the media's id, for a caller that keys
    /// its bookkeeping by it. The window asks after every change and the
    /// API asks before an export; both get the same list.
    pub fn requests(project: &Project, project_dir: &Path) -> Vec<(String, AnalyseRequest)> {
        let mut wanted: Vec<(String, AnalyseRequest)> = Vec::new();
        for clip in &project.active().clips {
            let Some(cutout) = clip.cutout.as_ref() else {
                continue;
            };
            if !clip.kind.is_visual() {
                continue;
            }
            let Some(media) = project.media_by_id(&clip.media_id) else {
                continue;
            };
            // The source the clip shows: its in-point, for as long as it
            // runs at its speed. A curve's mean is its speed, so this
            // covers a curved clip too.
            let range = (
                clip.source_start,
                clip.source_start + clip.duration * clip.speed.max(0.0625),
            );
            match wanted
                .iter_mut()
                .find(|(id, request)| *id == media.id && request.subject == cutout.subject)
            {
                Some((_, request)) => request.ranges.push(range),
                None => wanted.push((
                    media.id.clone(),
                    AnalyseRequest {
                        project: project_dir.to_path_buf(),
                        media_path: media.path.clone(),
                        media_size: (media.width.unwrap_or(0), media.height.unwrap_or(0)),
                        still: media.kind == MediaKind::Image,
                        subject: cutout.subject,
                        ranges: vec![range],
                    },
                )),
            }
        }
        wanted
    }

    /// How many instants `request` still needs, without doing anything.
    /// A store made by a model the subject would not pick now needs all
    /// of them again.
    pub fn outstanding(request: &AnalyseRequest) -> usize {
        let store = MaskStore::open(&mask_dir(
            &request.project,
            &request.media_path,
            request.subject,
        ));
        let stale = match (request.subject, store.model()) {
            (_, None) => false,
            (Subject::Person, Some(model)) => model == ModelId::Object.name(),
            (Subject::Object, Some(model)) => model != ModelId::Object.name(),
            (Subject::Auto, _) => false,
        };
        if stale {
            return usize::MAX;
        }
        if request.still {
            return usize::from(store.is_empty());
        }
        missing(&store, &request.ranges).len()
    }

    /// Fills the masks `request` is missing. Blocks for the whole run, so
    /// run it on its own thread; `progress` is called as it goes. Returns
    /// how many masks were written.
    pub fn analyse(
        &self,
        request: &AnalyseRequest,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<usize, String> {
        let job = self.gate.begin("cutout analysis")?;
        let cancel = job.cancel_handle();
        let mut store = MaskStore::open(&mask_dir(
            &request.project,
            &request.media_path,
            request.subject,
        ));
        if Self::outstanding(request) == usize::MAX {
            store.clear();
        }

        // The model the subject names, or for automatic the one the
        // store already records; the first frame decides otherwise.
        let mut segmenter = match request.subject {
            Subject::Person => Some(self.person_or_selfie(&cancel, progress)?),
            Subject::Object => Some(self.downloaded(ModelId::Object, &cancel, progress)?),
            Subject::Auto => match store.model() {
                Some(model) if model == ModelId::Object.name() => {
                    Some(self.downloaded(ModelId::Object, &cancel, progress)?)
                }
                Some(_) => Some(self.person_or_selfie(&cancel, progress)?),
                None => None,
            },
        };

        let (width, height) = request.media_size;
        let size_for = |segmenter: &Segmenter| segmenter.input_size(width, height);

        if request.still {
            if !store.is_empty() {
                return Ok(0);
            }
            let frame_with = |segmenter: &Segmenter| {
                let (w, h) = size_for(segmenter);
                let options = DecodeOptions::default().scaled_to(w, h).limited_to(1);
                let mut decoder = Decoder::open(&request.media_path, &options)
                    .map_err(|error| error.to_string())?;
                decoder
                    .next_frame()
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("{}: no picture to analyse", request.media_path))
            };
            let segmenter = match segmenter {
                Some(segmenter) => segmenter,
                None => {
                    let person = self.person_or_selfie(&cancel, progress)?;
                    person.begin();
                    let mask = person.mask(&frame_with(&person)?)?;
                    if mask.is_blank() {
                        self.downloaded(ModelId::Object, &cancel, progress)?
                    } else {
                        store.set_model(person.name())?;
                        store.put(0, &mask)?;
                        progress(Progress::Analysing(1.0));
                        return Ok(1);
                    }
                }
            };
            segmenter.begin();
            let mask = segmenter.mask(&frame_with(&segmenter)?)?;
            store.set_model(segmenter.name())?;
            store.put(0, &mask)?;
            progress(Progress::Analysing(1.0));
            return Ok(1);
        }

        let wanted = missing(&store, &request.ranges);
        let total = wanted.len();
        if total == 0 {
            return Ok(0);
        }
        let step_ms = 1000 / u64::from(MASK_RATE);
        let mut done = 0usize;
        progress(Progress::Analysing(0.0));

        // Each unbroken run of missing instants is one pass of a decoder
        // paced to the analysis grid, so no frame is sought twice, and one
        // run of the person model's memory.
        for run in runs(&wanted, step_ms) {
            let start = Rational::new(run[0] as i64, 1000);
            let mut active = match &segmenter {
                Some(segmenter) => Arc::clone(segmenter),
                None => self.person_or_selfie(&cancel, progress)?,
            };
            let open = |segmenter: &Segmenter| {
                let (w, h) = size_for(segmenter);
                let options = DecodeOptions::default()
                    .starting_at(start)
                    .scaled_to(w, h)
                    .at_rate(FrameRate::new(Rational::new(i64::from(MASK_RATE), 1)))
                    .limited_to(run.len() as u64);
                Decoder::open(&request.media_path, &options).map_err(|error| error.to_string())
            };
            let mut decoder = open(&active)?;
            active.begin();
            let mut next = 0usize;
            while next < run.len() {
                if job.cancelled() {
                    return Err("cutout analysis cancelled".to_owned());
                }
                let Some(frame) = decoder.next_frame().map_err(|error| error.to_string())? else {
                    // The file ran out before its stated length: what was
                    // found is what there is.
                    break;
                };
                let mut mask = active.mask(&frame)?;
                if segmenter.is_none() {
                    // Automatic, undecided: the person model has just
                    // looked at the first frame. Nobody there means the
                    // object model takes the whole run, from this frame.
                    if mask.is_blank() && active.kind() != Kind::Object {
                        active = self.downloaded(ModelId::Object, &cancel, progress)?;
                        decoder = open(&active)?;
                        active.begin();
                        let Some(again) =
                            decoder.next_frame().map_err(|error| error.to_string())?
                        else {
                            break;
                        };
                        mask = active.mask(&again)?;
                    }
                    store.set_model(active.name())?;
                    segmenter = Some(Arc::clone(&active));
                } else if store.model().is_none() {
                    store.set_model(active.name())?;
                }
                store.put(run[next], &mask)?;
                next += 1;
                done += 1;
                progress(Progress::Analysing(done as f32 / total as f32));
            }
        }
        progress(Progress::Analysing(1.0));
        Ok(done)
    }
}

/// Streams a model into `file`, by way of a `.part` beside it, reporting
/// every couple of megabytes and stopping when `cancel` is set.
pub(crate) fn fetch(
    id: ModelId,
    file: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), String> {
    use std::io::{Read, Write};

    let spec = id.spec();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let partial = file.with_extension("part");
    progress(Progress::Fetching {
        received: 0,
        total: spec.bytes,
    });

    // A stalled connection blocks in `read` with the cancel flag
    // unreachable; thirty seconds without a byte means it is dead.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(30))
        .build();
    let response = agent
        .get(spec.url)
        .call()
        .map_err(|error| format!("could not fetch the {} model: {error}", spec.file))?;
    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(spec.bytes);

    let result = (|| {
        let mut out = std::fs::File::create(&partial)
            .map_err(|error| format!("could not create {}: {error}", partial.display()))?;
        let mut reader = response.into_reader();
        let mut buffer = [0u8; 64 * 1024];
        let mut received = 0u64;
        let mut reported = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err("cutout analysis cancelled".to_owned());
            }
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("model download interrupted: {error}"))?;
            if read == 0 {
                break;
            }
            out.write_all(&buffer[..read])
                .map_err(|error| format!("could not write the model: {error}"))?;
            received += read as u64;
            if received - reported >= 2 * 1024 * 1024 {
                reported = received;
                progress(Progress::Fetching { received, total });
            }
        }
        out.flush()
            .map_err(|error| format!("could not write the model: {error}"))?;
        if received == 0 {
            return Err(format!("the {} model came back empty", spec.file));
        }
        Ok(())
    })();
    match result {
        Ok(()) => std::fs::rename(&partial, file)
            .map_err(|error| format!("could not finish {}: {error}", file.display())),
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(error)
        }
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
