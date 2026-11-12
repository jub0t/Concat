// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Reading the export's footage ahead of its drawing.
//!
//! A paced clip's decoder used to be pulled on the drawing thread, layer
//! after layer, so a frame was read, then drawn, then the next one read:
//! the device sat idle while the footage was read, the decoders of two
//! layers took turns, and each cut stopped the export while the next
//! clip's file was opened and sought. Here each run of a paced clip has a
//! [`Feed`] - its decoder on a thread of its own, a few frames ahead in a
//! bounded queue - and [`Feeds`] opens each one a moment before its run
//! begins, so the seek is done by the time the frame is wanted.
//!
//! What comes out is what the drawing thread's own decoder gave, frame for
//! frame: a run is one decoder opened at the run's first frame, at that
//! frame's source time, pulled once per output frame, as before.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread::JoinHandle;

use concat_core::frame::Frame;
use concat_core::timeline::ClipId;
use concat_media::{DecodeOptions, Decoder, FrameSource};
use concat_render::{FramePlan, PlannedLayer};

/// How much decoded picture one feed may hold waiting, in bytes: four
/// 1080p frames, or one 4K frame - the one being decoded is on top.
const QUEUED_BYTES: usize = 48 << 20;

/// Frames a feed holds at most, whatever their size.
const QUEUED_MOST: usize = 4;

/// One run of one clip, decoded on its own thread.
pub(crate) struct Feed {
    frames: Option<Receiver<Result<Option<Frame>, String>>>,
    thread: Option<JoinHandle<()>>,
}

impl Feed {
    /// Starts reading `media` with `options`, `bytes` a frame.
    fn open(media: PathBuf, options: DecodeOptions, bytes: usize) -> Result<Self, String> {
        let depth = (QUEUED_BYTES / bytes.max(1)).clamp(1, QUEUED_MOST);
        let (send, frames) = sync_channel(depth);
        let named = media.display().to_string();
        let thread = std::thread::Builder::new()
            .name("concat export read".to_owned())
            .spawn(move || {
                let mut decoder = match Decoder::open(&media, &options) {
                    Ok(decoder) => decoder,
                    Err(error) => {
                        let _ = send.send(Err(error.to_string()));
                        return;
                    }
                };
                // Until the file runs out, fails, or nobody is reading: a
                // send to a feed that was dropped fails, and that is the
                // thread's signal to go.
                loop {
                    let next = decoder.next_frame().map_err(|error| error.to_string());
                    let last = !matches!(next, Ok(Some(_)));
                    if send.send(next).is_err() || last {
                        return;
                    }
                }
            })
            .map_err(|error| format!("could not start reading {named}: {error}"))?;
        Ok(Self {
            frames: Some(frames),
            thread: Some(thread),
        })
    }

    /// The run's next frame; `None` once its file has run out.
    fn next(&mut self) -> Result<Option<Frame>, String> {
        match self.frames.as_ref().map(Receiver::recv) {
            Some(Ok(next)) => next,
            // The thread has gone after its last frame or its error.
            _ => Ok(None),
        }
    }
}

impl Drop for Feed {
    fn drop(&mut self) {
        // The queue goes first, so a thread waiting to hand over a frame
        // is told at once and one decoding stops after the frame in hand;
        // then it is waited for, so its decoder - and on a phone, its seat
        // on the chip - is let go before the next one is asked for.
        self.frames = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Every paced clip's feeds over one pass of an export.
pub(crate) struct Feeds {
    /// Each clip's runs, earliest first, with the frame each begins at.
    runs: HashMap<ClipId, VecDeque<(i64, Feed)>>,
    /// The last frame looked ahead to, and the paced clips on it.
    looked: i64,
    on_looked: HashSet<ClipId>,
    /// How many frames ahead of the drawing a run is opened.
    lead: i64,
}

impl Feeds {
    /// `lead`: how many frames before its first a run's decoder is opened.
    pub(crate) fn new(lead: i64) -> Self {
        Self {
            runs: HashMap::new(),
            looked: -1,
            on_looked: HashSet::new(),
            lead: lead.max(0),
        }
    }

    /// Opens every run that begins by `index + lead`, given `plan_at` to
    /// plan a frame and `open` to say how a layer's decoder is opened and
    /// how many bytes its frames are.
    pub(crate) fn look_ahead(
        &mut self,
        index: i64,
        total_frames: i64,
        plan_at: impl Fn(i64) -> FramePlan,
        open: impl Fn(&PlannedLayer) -> (DecodeOptions, usize),
    ) -> Result<(), String> {
        let until = (index + self.lead).min(total_frames - 1);
        while self.looked < until {
            self.looked += 1;
            let at = self.looked;
            let plan = plan_at(at);
            let mut on: HashSet<ClipId> = HashSet::new();
            for layer in plan.layers.iter().filter(|layer| layer.paced) {
                on.insert(layer.clip);
                if !self.on_looked.contains(&layer.clip) {
                    self.start(at, layer, &open)?;
                }
            }
            self.on_looked = on;
        }
        Ok(())
    }

    fn start(
        &mut self,
        at: i64,
        layer: &PlannedLayer,
        open: &impl Fn(&PlannedLayer) -> (DecodeOptions, usize),
    ) -> Result<(), String> {
        let (options, bytes) = open(layer);
        let feed = Feed::open(layer.media.clone(), options, bytes)?;
        self.runs
            .entry(layer.clip)
            .or_default()
            .push_back((at, feed));
        Ok(())
    }

    /// The next frame of `layer`'s run at frame `index`. A run not looked
    /// ahead to is opened here, so nothing depends on the lead.
    pub(crate) fn next(
        &mut self,
        index: i64,
        layer: &PlannedLayer,
        open: impl Fn(&PlannedLayer) -> (DecodeOptions, usize),
    ) -> Result<Option<Frame>, String> {
        let ready = self
            .runs
            .get(&layer.clip)
            .and_then(VecDeque::front)
            .is_some_and(|(begins, _)| *begins <= index);
        if !ready {
            let (options, bytes) = open(layer);
            let feed = Feed::open(layer.media.clone(), options, bytes)?;
            self.runs
                .entry(layer.clip)
                .or_default()
                .push_front((index, feed));
        }
        let (_, feed) = self
            .runs
            .get_mut(&layer.clip)
            .and_then(VecDeque::front_mut)
            .expect("ensured above");
        feed.next()
    }

    /// After frame `index`, drawn from `plan`: every run that has begun
    /// and whose clip is not on the frame is over, and its decoder goes.
    pub(crate) fn retire(&mut self, index: i64, plan: &FramePlan) {
        let on: HashSet<ClipId> = plan.layers.iter().map(|layer| layer.clip).collect();
        self.runs.retain(|clip, runs| {
            if !on.contains(clip) {
                while runs.front().is_some_and(|(begins, _)| *begins <= index) {
                    runs.pop_front();
                }
            }
            !runs.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_core::arena::Arena;
    use concat_core::time::{FrameRate, Rational};
    use concat_core::timeline::{Clip, MediaRef};
    use std::sync::Arc;

    /// A run is opened before it begins, read in order, and let go once
    /// its clip has left the frame; a clip that comes back is a new run
    /// from its new place, not the old decoder carried on.
    #[test]
    fn runs_open_ahead_and_close_behind() {
        let path = std::env::temp_dir().join("concat-export-ahead.mp4");
        let rate = FrameRate::THIRTY;
        if !path.is_file() {
            let mut encoder = concat_media::Encoder::create(
                &path,
                64,
                64,
                rate,
                &concat_media::EncodeOptions::default(),
            )
            .expect("an encoder");
            for _ in 0..60 {
                concat_media::FrameSink::write_frame(&mut encoder, &Frame::black(64, 64))
                    .expect("writes");
            }
            concat_media::FrameSink::finish(&mut encoder).expect("finishes");
        }
        let mut arena = Arena::new();
        let mut id = || {
            arena.insert(Clip::new(
                MediaRef::new(&path),
                Rational::ZERO,
                Rational::ONE,
            ))
        };
        let (one, two) = (id(), id());
        let layer = |clip: ClipId, source: i64| PlannedLayer {
            media: path.clone(),
            source_time: Rational::new(source, 30),
            ..PlannedLayer::picture(clip, Arc::new(Frame::black(1, 1)))
        };
        // Clip 1 on frames 0-9 and again from 20; clip 2 from frame 5.
        let plan_at = |index: i64| FramePlan {
            layers: [
                (!(10..20).contains(&index)).then(|| layer(one, index)),
                (index >= 5).then(|| layer(two, index - 5)),
            ]
            .into_iter()
            .flatten()
            .collect(),
            ..FramePlan::empty(64, 64)
        };
        let open = |layer: &PlannedLayer| {
            (
                DecodeOptions::default()
                    .starting_at(layer.source_time)
                    .scaled_to(64, 64)
                    .at_rate(rate),
                64 * 64 * 4,
            )
        };
        let mut feeds = Feeds::new(3);
        let mut pulled: HashMap<ClipId, Vec<i64>> = HashMap::new();
        for index in 0..30 {
            feeds.look_ahead(index, 30, plan_at, open).expect("opens");
            if index == 2 {
                assert!(feeds.runs.contains_key(&two), "clip 2 opened ahead");
            }
            let plan = plan_at(index);
            for layer in &plan.layers {
                feeds
                    .next(index, layer, open)
                    .expect("reads")
                    .expect("a frame");
                pulled.entry(layer.clip).or_default().push(index);
            }
            feeds.retire(index, &plan);
            if (10..17).contains(&index) {
                assert!(
                    !feeds.runs.contains_key(&one),
                    "clip 1's first run is gone at {index}"
                );
            }
            if index == 17 {
                let runs = &feeds.runs[&one];
                assert_eq!(runs.len(), 1);
                assert_eq!(runs[0].0, 20, "the second run begins where it begins");
            }
        }
        assert_eq!(pulled[&one].len(), 20);
        assert_eq!(pulled[&two].len(), 25);
    }
}
