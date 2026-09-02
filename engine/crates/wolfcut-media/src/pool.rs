// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The reader pool and frame cache: random access to any frame of any file.
//!
//! Everything before this decoded *forward*: open a decoder, pull frames in
//! order, drop it. That is exactly right for export and exactly wrong for
//! interactive use - scrubbing asks for arbitrary (media, time) pairs, and
//! respawning a process per request is why playback could not exist. This
//! module is the piece the decision log has called "the next real piece of
//! work" since the first CLI commit:
//!
//! - **One reader per (file, size)**, kept warm between requests. A request
//!   near the reader's current position rolls forward (cheap, exact); a
//!   request elsewhere seeks - frame-accurately through the FFI decoder when
//!   the `ffi` feature is on, by respawning the subprocess decoder at the
//!   target otherwise.
//! - **A byte-budgeted LRU frame cache** in front of the readers. Scrubbing
//!   back over ground just covered, or dwelling on one frame, costs a hash
//!   lookup. Frames roll into the cache as decoding passes them, so rolling
//!   forward to frame N caches N-1 frames of the path there.
//!
//! The pool is deliberately not used by export: export decodes every frame
//! exactly once in order, and a cache in that path is pure overhead. This is
//! playback infrastructure - see `docs/decisions/0002`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wolfcut_core::frame::Frame;
use wolfcut_core::time::{FrameRate, Rational};

use crate::decode::{DecodeOptions, FfmpegDecoder, FrameSource};
use crate::error::Result;
use crate::probe;

/// How far ahead of a reader's position a request may be and still be worth
/// decoding forward to, rather than seeking. Two seconds of 30fps material is
/// sixty decodes - cheaper than a subprocess respawn, comparable to a seek.
const ROLL_FORWARD_FRAMES: i64 = 60;

/// What one pooled frame request asks for.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct FrameKey {
    path: PathBuf,
    width: u32,
    height: u32,
    /// The clip's effect chain, part of the frame's identity: the same source
    /// frame through two different chains is two different pictures.
    chain: Option<String>,
    index: i64,
}

/// A byte-budgeted LRU of decoded frames.
///
/// Plain and measurable on purpose: a `HashMap` plus a logical clock, evicting
/// the least-recently-touched frame until the budget holds. At preview sizes a
/// frame is ~2-4 MB, so the default budget holds a few hundred frames - many
/// seconds of scrub history.
pub struct FrameCache {
    budget: usize,
    held: usize,
    tick: u64,
    frames: HashMap<FrameKey, (Arc<Frame>, u64)>,
}

impl FrameCache {
    /// An empty cache holding at most `budget` bytes of frames.
    pub fn new(budget: usize) -> Self {
        Self { budget, held: 0, tick: 0, frames: HashMap::new() }
    }

    fn get(&mut self, key: &FrameKey) -> Option<Arc<Frame>> {
        self.tick += 1;
        let tick = self.tick;
        self.frames.get_mut(key).map(|(frame, touched)| {
            *touched = tick;
            Arc::clone(frame)
        })
    }

    fn insert(&mut self, key: FrameKey, frame: Arc<Frame>) {
        let bytes = frame.pixels().len();
        // A frame larger than the whole budget would evict everything and
        // still not fit; hold it once without caching it.
        if bytes > self.budget {
            return;
        }
        self.tick += 1;
        if let Some((previous, _)) = self.frames.insert(key, (frame, self.tick)) {
            self.held -= previous.pixels().len();
        }
        self.held += bytes;
        while self.held > self.budget {
            let Some(oldest) = self
                .frames
                .iter()
                .min_by_key(|(_, (_, touched))| *touched)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some((evicted, _)) = self.frames.remove(&oldest) {
                self.held -= evicted.pixels().len();
            }
        }
    }

    /// Bytes currently held.
    pub fn held(&self) -> usize {
        self.held
    }

    /// Frames currently held.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

/// How a reader should satisfy a request for `target`, given where it is.
///
/// Pure, so the seek-versus-roll policy is testable without a decoder: the
/// bug this class of code grows is "scrubbing backwards is mysteriously slow",
/// and that is a policy bug, not a decoder bug.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Access {
    /// Decode forward this many frames from the current position.
    Roll(i64),
    /// Jump: the target is behind, or too far ahead to roll to.
    Seek,
}

fn plan_access(current_next: Option<i64>, target: i64) -> Access {
    match current_next {
        Some(next) if target >= next && target - next <= ROLL_FORWARD_FRAMES => {
            Access::Roll(target - next)
        }
        _ => Access::Seek,
    }
}

/// One warm reader: a decoder plus where its *next* frame will land.
struct Reader {
    backend: Backend,
    /// The frame index `next_frame` will produce, in the media's own rate.
    next_index: i64,
}

enum Backend {
    /// Frame-accurate seeks, real timestamps. Only with the `ffi` feature.
    #[cfg(feature = "ffi")]
    Linked(crate::ffi::FfiDecoder),
    /// The subprocess pipe: no seeking, so "seek" means respawn at the
    /// target's keyframe-fast `-ss` and count ordinally from there. Exact for
    /// constant-frame-rate material, the pipe's known limit otherwise.
    Pipe { decoder: FfmpegDecoder, path: PathBuf, width: u32, height: u32, chain: Option<String> },
}

impl Reader {
    fn open(
        path: &Path,
        width: u32,
        height: u32,
        chain: Option<&str>,
        rate: FrameRate,
        index: i64,
    ) -> Result<Self> {
        #[cfg(feature = "ffi")]
        // The linked decoder produces raw frames; a clip with an effect chain
        // needs FFmpeg's filters, so it stays on the pipe.
        if chain.is_none() {
            if let Ok(mut linked) = crate::ffi::FfiDecoder::open(path, width, height) {
                use crate::decode::SeekableSource;
                linked.seek(rate.time_of_frame(index))?;
                // The linked decoder lands on the keyframe at or before the
                // target; rolling forward from there is what `frame_at` does,
                // guided by real timestamps. Report where decoding resumes.
                return Ok(Self { backend: Backend::Linked(linked), next_index: i64::MIN });
            }
            // A file the linked build cannot open falls through to the pipe.
        }

        let mut options = DecodeOptions::default()
            .starting_at(rate.time_of_frame(index))
            .scaled_to(width, height)
            .at_rate(rate);
        if let Some(chain) = chain {
            options = options.filtered(chain);
        }
        let decoder = FfmpegDecoder::open(path, &options)?;
        Ok(Self {
            backend: Backend::Pipe {
                decoder,
                path: path.to_path_buf(),
                width,
                height,
                chain: chain.map(str::to_owned),
            },
            next_index: index,
        })
    }

    #[cfg_attr(not(feature = "ffi"), allow(unused_variables))]
    fn next_frame(&mut self, rate: FrameRate) -> Result<Option<(i64, Frame)>> {
        match &mut self.backend {
            #[cfg(feature = "ffi")]
            Backend::Linked(decoder) => {
                let Some(frame) = decoder.next_frame()? else { return Ok(None) };
                // The linked decoder tells us where the frame really sits; a
                // half-frame nudge keeps exact boundary timestamps from
                // rounding down a frame.
                let index = decoder
                    .position()
                    .map(|position| rate.frame_at(position + rate.frame_duration() / Rational::from_int(2)))
                    .unwrap_or(self.next_index.max(0));
                self.next_index = index + 1;
                Ok(Some((index, frame)))
            }
            Backend::Pipe { decoder, .. } => {
                let Some(frame) = decoder.next_frame()? else { return Ok(None) };
                let index = self.next_index;
                self.next_index = index + 1;
                Ok(Some((index, frame)))
            }
        }
    }

    fn seek(&mut self, rate: FrameRate, index: i64) -> Result<()> {
        match &mut self.backend {
            #[cfg(feature = "ffi")]
            Backend::Linked(decoder) => {
                use crate::decode::SeekableSource;
                decoder.seek(rate.time_of_frame(index))?;
                self.next_index = i64::MIN;
                Ok(())
            }
            Backend::Pipe { decoder, path, width, height, chain } => {
                // The pipe cannot seek; a new process at the target is the
                // seek. This is the cost the FFI backend exists to remove.
                let mut options = DecodeOptions::default()
                    .starting_at(rate.time_of_frame(index))
                    .scaled_to(*width, *height)
                    .at_rate(rate);
                if let Some(chain) = chain {
                    options = options.filtered(chain.clone());
                }
                *decoder = FfmpegDecoder::open(path.as_path(), &options)?;
                self.next_index = index;
                Ok(())
            }
        }
    }
}

/// The media facts the pool needs per file, probed once.
struct MediaFacts {
    rate: FrameRate,
    /// A still image: one frame, served for every requested time.
    still: bool,
    /// Whole frames the container claims to hold, when it states a duration.
    /// Requests past this clamp to the last frame *before* any seek happens -
    /// a seek past the end produces zero frames, not an error and not a
    /// picture.
    frames: Option<i64>,
}

/// Random access to frames across many files, cache in front, warm readers
/// behind. `&mut self` throughout: callers serialise access, which is also
/// the useful property - one scrub request at a time, in order.
pub struct ReaderPool {
    cache: FrameCache,
    readers: HashMap<(PathBuf, u32, u32, Option<String>), Reader>,
    facts: HashMap<PathBuf, MediaFacts>,
    /// Readers kept warm before the least-recently-used is dropped.
    max_readers: usize,
    /// Recency order for reader eviction, oldest first.
    reader_order: Vec<(PathBuf, u32, u32, Option<String>)>,
}

impl ReaderPool {
    /// A pool with `cache_bytes` of frame cache and up to `max_readers` warm
    /// decoders.
    pub fn new(cache_bytes: usize, max_readers: usize) -> Self {
        Self {
            cache: FrameCache::new(cache_bytes),
            readers: HashMap::new(),
            facts: HashMap::new(),
            max_readers: max_readers.max(1),
            reader_order: Vec::new(),
        }
    }

    /// 512 MB of frames, eight warm readers - enough for a busy timeline.
    pub fn with_defaults() -> Self {
        Self::new(512 * 1024 * 1024, 8)
    }

    /// The frame of `path` on screen at `time` (in the media's own clock),
    /// scaled to `width` x `height`.
    ///
    /// `still` marks image files: a one-frame stream served for every time,
    /// which the caller knows (from its own model) and the pool must not
    /// guess from probing.
    pub fn frame_at(
        &mut self,
        path: &Path,
        time: Rational,
        width: u32,
        height: u32,
        still: bool,
        chain: Option<&str>,
    ) -> Result<Arc<Frame>> {
        let facts = self.facts_for(path, still)?;
        let rate = facts.rate;
        let mut target = if facts.still { 0 } else { rate.frame_at(time).max(0) };
        // A clip can outlive its media - an end-trim past the file's length -
        // and the honest picture for any time past the end is the last frame,
        // exactly what the roll-forward case already serves. Clamp before
        // seeking: `-ss` past the end decodes nothing at all.
        if let Some(frames) = facts.frames {
            target = target.min((frames - 1).max(0));
        }
        let chain_key = chain.map(str::to_owned);

        let key = FrameKey {
            path: path.to_path_buf(),
            width,
            height,
            chain: chain_key.clone(),
            index: target,
        };
        if let Some(frame) = self.cache.get(&key) {
            return Ok(frame);
        }

        self.touch_reader(path, width, height, chain, rate, target)?;
        let reader_key = (path.to_path_buf(), width, height, chain_key.clone());
        let reader = self.readers.get_mut(&reader_key).expect("just ensured");

        // Roll forward to the target, caching everything passed on the way -
        // the next scrub over this span is then free.
        let mut latest: Option<Arc<Frame>> = None;
        // Stops at end of stream too: a clip trimmed past its media's end,
        // where the last real frame is the honest answer.
        while let Some((index, frame)) = reader.next_frame(rate)? {
            let frame = Arc::new(frame);
            self.cache.insert(
                FrameKey {
                    path: path.to_path_buf(),
                    width,
                    height,
                    chain: chain_key.clone(),
                    index,
                },
                Arc::clone(&frame),
            );
            latest = Some(frame);
            if index >= target {
                break;
            }
        }

        // Nothing at all means the seek itself landed past the end of the
        // picture stream - a container whose video ends before its audio, or
        // a stated duration that lied past the clamp above. The last real
        // frame is still the honest answer; it just has to be found by
        // seeking backwards until something decodes. Each retry doubles the
        // step, and the last one starts from zero, so a file with any
        // decodable picture at all cannot fail here.
        if latest.is_none() {
            for step in [30i64, 240, i64::MAX] {
                let from = target.saturating_sub(step).max(0);
                reader.seek(rate, from)?;
                while let Some((index, frame)) = reader.next_frame(rate)? {
                    let frame = Arc::new(frame);
                    self.cache.insert(
                        FrameKey {
                            path: path.to_path_buf(),
                            width,
                            height,
                            chain: chain_key.clone(),
                            index,
                        },
                        Arc::clone(&frame),
                    );
                    latest = Some(frame);
                    if index >= target {
                        break;
                    }
                }
                if latest.is_some() || from == 0 {
                    break;
                }
            }
            if let Some(frame) = &latest {
                // Remember the answer under the index that was asked for, so
                // dwelling on a time past the end costs one lookup, not a
                // respawn-and-decode per request.
                self.cache.insert(key, Arc::clone(frame));
            }
        }

        latest.ok_or_else(|| crate::error::Error::PartialFrame {
            path: path.to_path_buf(),
            got: 0,
            want: Frame::byte_len(width, height),
        })
    }

    fn facts_for(&mut self, path: &Path, still: bool) -> Result<&MediaFacts> {
        if !self.facts.contains_key(path) {
            let facts = if still {
                MediaFacts { rate: FrameRate::THIRTY, still: true, frames: None }
            } else {
                let info = probe::probe(path)?;
                let rate = info
                    .video
                    .as_ref()
                    .map(|video| video.frame_rate)
                    .unwrap_or(FrameRate::THIRTY);
                // Ceil rather than floor: undercounting by one would clamp
                // legitimate requests for the true last frame.
                let frames = info
                    .duration
                    .map(|duration| (duration * rate.fps()).ceil())
                    .filter(|count| *count > 0);
                MediaFacts { rate, still: false, frames }
            };
            self.facts.insert(path.to_path_buf(), facts);
        }
        Ok(self.facts.get(path).expect("just inserted"))
    }

    /// Ensures a reader positioned to reach `target`, evicting the coldest
    /// reader when the pool is full.
    fn touch_reader(
        &mut self,
        path: &Path,
        width: u32,
        height: u32,
        chain: Option<&str>,
        rate: FrameRate,
        target: i64,
    ) -> Result<()> {
        let key = (path.to_path_buf(), width, height, chain.map(str::to_owned));

        self.reader_order.retain(|entry| entry != &key);
        self.reader_order.push(key.clone());

        if let Some(reader) = self.readers.get_mut(&key) {
            let next = (reader.next_index != i64::MIN).then_some(reader.next_index);
            if plan_access(next, target) == Access::Seek {
                reader.seek(rate, target)?;
            }
            return Ok(());
        }

        while self.readers.len() >= self.max_readers && !self.reader_order.is_empty() {
            let coldest = self.reader_order.remove(0);
            if coldest == key {
                self.reader_order.push(coldest);
                break;
            }
            self.readers.remove(&coldest);
        }

        let reader = Reader::open(path, width, height, chain, rate, target)?;
        self.readers.insert(key, reader);
        Ok(())
    }

    /// Drops every warm reader and cached frame - for when the media set
    /// changes wholesale, like closing a project.
    pub fn clear(&mut self) {
        self.readers.clear();
        self.reader_order.clear();
        self.facts.clear();
        self.cache = FrameCache::new(self.cache.budget);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::{EncodeOptions, FfmpegEncoder, FrameSink};

    #[test]
    fn the_access_policy_rolls_forward_and_seeks_backward() {
        assert_eq!(plan_access(Some(10), 10), Access::Roll(0), "the very next frame");
        assert_eq!(plan_access(Some(10), 30), Access::Roll(20), "a short hop ahead");
        assert_eq!(plan_access(Some(10), 9), Access::Seek, "behind means seek");
        assert_eq!(
            plan_access(Some(10), 10 + ROLL_FORWARD_FRAMES + 1),
            Access::Seek,
            "too far ahead means seek"
        );
        assert_eq!(plan_access(None, 5), Access::Seek, "unknown position means seek");
    }

    #[test]
    fn the_cache_evicts_least_recently_used_within_budget() {
        // Frames are 16 bytes each (2x2); budget holds exactly two.
        let mut cache = FrameCache::new(32);
        let key = |index: i64| FrameKey {
            path: PathBuf::from("a.mp4"),
            width: 2,
            height: 2,
            chain: None,
            index,
        };
        cache.insert(key(0), Arc::new(Frame::black(2, 2)));
        cache.insert(key(1), Arc::new(Frame::black(2, 2)));
        assert_eq!(cache.len(), 2);

        // Touch 0 so 1 is the coldest, then overflow.
        assert!(cache.get(&key(0)).is_some());
        cache.insert(key(2), Arc::new(Frame::black(2, 2)));

        assert_eq!(cache.len(), 2);
        assert!(cache.get(&key(0)).is_some(), "recently touched survives");
        assert!(cache.get(&key(1)).is_none(), "coldest was evicted");
        assert!(cache.get(&key(2)).is_some());
        assert_eq!(cache.held(), 32);
    }

    #[test]
    fn an_oversized_frame_is_served_but_never_cached() {
        let mut cache = FrameCache::new(8);
        cache.insert(
            FrameKey { path: PathBuf::from("a"), width: 2, height: 2, chain: None, index: 0 },
            Arc::new(Frame::black(2, 2)),
        );
        assert!(cache.is_empty(), "16 bytes cannot fit an 8 byte budget");
    }

    /// End to end against a real FFmpeg: encode a tiny video whose frames are
    /// identifiable by colour, then read arbitrary frames back out of order.
    /// Skips silently on machines without FFmpeg, like the encoder's own test.
    #[test]
    fn random_access_returns_the_right_frames() {
        let path = std::env::temp_dir().join("wolfcut-pool-test.mp4");
        let Ok(mut encoder) =
            FfmpegEncoder::create(&path, 64, 64, FrameRate::THIRTY, &EncodeOptions::default())
        else {
            return; // no ffmpeg here
        };
        // 90 frames; the red channel encodes the frame index (x2 to survive
        // compression rounding).
        for index in 0..90u32 {
            let mut frame = Frame::black(64, 64);
            frame.fill([(index * 2).min(255) as u8, 40, 40, 255]);
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");

        let mut pool = ReaderPool::new(64 * 1024 * 1024, 4);
        let rate = FrameRate::THIRTY;
        let red_at = |pool: &mut ReaderPool, index: i64| -> i64 {
            let frame = pool
                .frame_at(&path, rate.time_of_frame(index), 64, 64, false, None)
                .expect("frame decodes");
            i64::from(frame.pixel(32, 32).expect("in bounds")[0])
        };

        // Forward, backward, far jump, and revisit - the scrub shapes.
        let near = |got: i64, index: i64| (got - index * 2).abs() <= 8;
        let at_10 = red_at(&mut pool, 10);
        assert!(near(at_10, 10), "frame 10 read {at_10}");
        let at_50 = red_at(&mut pool, 50);
        assert!(near(at_50, 50), "frame 50 read {at_50}");
        let back_at_20 = red_at(&mut pool, 20);
        assert!(near(back_at_20, 20), "backward to 20 read {back_at_20}");
        let again_at_50 = red_at(&mut pool, 50);
        assert_eq!(again_at_50, at_50, "revisit must come from cache, identical");
        assert!(!pool.cache.is_empty(), "rolling forward populated the cache");

        // Far past the end - a clip that outlives its media. The last real
        // frame is the answer, not an error: a seek there decodes nothing,
        // which used to surface as PartialFrame(0 bytes) and a black monitor.
        let past_end = red_at(&mut pool, 300);
        assert!(near(past_end, 89), "past the end read {past_end}, wanted the last frame");

        let _ = std::fs::remove_file(&path);
    }
}
