// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The reader pool and frame cache: random access to any frame of any file.
//!
//! A decoder on its own reads *forward*: open it, pull frames in order,
//! drop it. That is exactly right for export and exactly wrong for
//! interactive use, where scrubbing asks for arbitrary (media, time) pairs.
//! This module is the answer:
//!
//! - **One reader per (file, size, chain)**, kept warm between requests. A
//!   request near the reader's current position rolls forward (cheap,
//!   exact); a request elsewhere seeks - frame-accurately, guided by the
//!   real timestamps the linked decoder reports.
//! - **A byte-budgeted LRU frame cache** in front of the readers. Scrubbing
//!   back over ground just covered, or dwelling on one frame, costs a hash
//!   lookup. Frames roll into the cache as decoding passes them, so rolling
//!   forward to frame N caches N-1 frames of the path there.
//!
//! The pool is deliberately not used by export: export decodes every frame
//! exactly once in order, and a cache in that path is pure overhead. This is
//! playback infrastructure.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use concat_core::frame::Frame;
use concat_core::time::{FrameRate, Rational};

use crate::decode::{DecodeOptions, Decoder, FrameSource, SeekableSource};
use crate::error::Result;
use crate::probe;

/// How far ahead of a reader's position a request may be and still be worth
/// decoding forward to, rather than seeking. Two seconds of 30fps material is
/// sixty decodes - comparable to a seek, and exact.
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
        Self {
            budget,
            held: 0,
            tick: 0,
            frames: HashMap::new(),
        }
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
    decoder: Decoder,
    /// The frame index `next_frame` will produce, in the media's own rate,
    /// or `i64::MIN` right after a seek, when only the timestamps know.
    next_index: i64,
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
        // Unpaced, so every source frame comes out with its own timestamp
        // and the index is read from that; a seek lands on the keyframe at
        // or before the target and `frame_at` rolls forward from there.
        let mut options = DecodeOptions::default()
            .starting_at(rate.time_of_frame(index))
            .scaled_to(width, height);
        if let Some(chain) = chain {
            options = options.filtered(chain);
        }
        let decoder = Decoder::open(path, &options)?;
        Ok(Self {
            decoder,
            next_index: i64::MIN,
        })
    }

    fn next_frame(&mut self, rate: FrameRate) -> Result<Option<(i64, Frame)>> {
        let Some(frame) = self.decoder.next_frame()? else {
            return Ok(None);
        };
        // The decoder says where the frame really sits; a half-frame nudge
        // keeps exact boundary timestamps from rounding down a frame.
        let index = self
            .decoder
            .position()
            .map(|position| rate.frame_at(position + rate.frame_duration() / Rational::from_int(2)))
            .unwrap_or(self.next_index.max(0));
        self.next_index = index + 1;
        Ok(Some((index, frame)))
    }

    fn seek(&mut self, rate: FrameRate, index: i64) -> Result<()> {
        self.decoder.seek(rate.time_of_frame(index))?;
        self.next_index = i64::MIN;
        Ok(())
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

/// One reader's identity: the file, the decode size, the effect chain.
type ReaderKey = (PathBuf, u32, u32, Option<String>);

/// The warm readers and their recency, behind one short lock. A reader is
/// found here and then used outside this lock, under its own, so a decode
/// on one file never waits on a decode on another.
struct Readers {
    warm: HashMap<ReaderKey, Arc<Mutex<Reader>>>,
    /// Recency order for reader eviction, oldest first.
    order: Vec<ReaderKey>,
    /// Readers kept warm before the least-recently-used is dropped.
    max: usize,
}

/// Random access to frames across many files, cache in front, warm readers
/// behind.
///
/// Shared, not serialised: every method takes `&self`, and the locks inside
/// are held for as little as each step needs. The cache is one short lock;
/// each reader is its own, so the playback stream decoding ahead on one
/// file and the monitor pulling a cached frame of another never queue
/// behind each other. Two callers wanting the *same* reader take turns,
/// which is what a single decoder demands anyway.
pub struct ReaderPool {
    cache: Mutex<FrameCache>,
    readers: Mutex<Readers>,
    facts: Mutex<HashMap<PathBuf, Arc<MediaFacts>>>,
}

impl ReaderPool {
    /// A pool with `cache_bytes` of frame cache and up to `max_readers` warm
    /// decoders.
    pub fn new(cache_bytes: usize, max_readers: usize) -> Self {
        Self {
            cache: Mutex::new(FrameCache::new(cache_bytes)),
            readers: Mutex::new(Readers {
                warm: HashMap::new(),
                order: Vec::new(),
                max: max_readers.max(1),
            }),
            facts: Mutex::new(HashMap::new()),
        }
    }

    /// Bytes of decoded frames currently cached.
    pub fn cached_bytes(&self) -> usize {
        self.cache.lock().map(|cache| cache.held()).unwrap_or(0)
    }

    /// Whether the frame is already decoded, without decoding it.
    fn cached(&self, key: &FrameKey) -> Option<Arc<Frame>> {
        self.cache.lock().ok()?.get(key)
    }

    fn remember(&self, key: FrameKey, frame: Arc<Frame>) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key, frame);
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
        &self,
        path: &Path,
        time: Rational,
        width: u32,
        height: u32,
        still: bool,
        chain: Option<&str>,
    ) -> Result<Arc<Frame>> {
        let facts = self.facts_for(path, still)?;
        let rate = facts.rate;
        let mut target = if facts.still {
            0
        } else {
            rate.frame_at(time).max(0)
        };
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
        if let Some(frame) = self.cached(&key) {
            return Ok(frame);
        }

        let shared = self.reader(path, width, height, chain, rate, target)?;
        let mut reader = shared.lock().map_err(|_| crate::error::Error::NoFrame {
            path: path.to_path_buf(),
        })?;
        // Another caller may have moved this reader while this one waited
        // for it; the answer may even be cached by now.
        if let Some(frame) = self.cached(&key) {
            return Ok(frame);
        }
        let next = (reader.next_index != i64::MIN).then_some(reader.next_index);
        if plan_access(next, target) == Access::Seek {
            reader.seek(rate, target)?;
        }

        // Roll forward to the target, caching everything passed on the way -
        // the next scrub over this span is then free.
        let mut latest: Option<Arc<Frame>> = None;
        // Stops at end of stream too: a clip trimmed past its media's end,
        // where the last real frame is the honest answer.
        while let Some((index, frame)) = reader.next_frame(rate)? {
            let frame = Arc::new(frame);
            self.remember(
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
                    self.remember(
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
                self.remember(key, Arc::clone(frame));
            }
        }

        latest.ok_or_else(|| crate::error::Error::NoFrame {
            path: path.to_path_buf(),
        })
    }

    fn facts_for(&self, path: &Path, still: bool) -> Result<Arc<MediaFacts>> {
        if let Some(facts) = self
            .facts
            .lock()
            .ok()
            .and_then(|facts| facts.get(path).cloned())
        {
            return Ok(facts);
        }
        // Probed outside the lock: a probe opens the file, and the other
        // callers should not wait on that.
        let facts = {
            if still {
                MediaFacts {
                    rate: FrameRate::THIRTY,
                    still: true,
                    frames: None,
                }
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
                MediaFacts {
                    rate,
                    still: false,
                    frames,
                }
            }
        };
        let facts = Arc::new(facts);
        if let Ok(mut known) = self.facts.lock() {
            known
                .entry(path.to_path_buf())
                .or_insert_with(|| Arc::clone(&facts));
        }
        Ok(facts)
    }

    /// The warm reader for this identity, opened at `target` when there is
    /// none, evicting the coldest reader when the pool is full. Positioning
    /// an existing reader is the caller's job, under the reader's own lock.
    fn reader(
        &self,
        path: &Path,
        width: u32,
        height: u32,
        chain: Option<&str>,
        rate: FrameRate,
        target: i64,
    ) -> Result<Arc<Mutex<Reader>>> {
        let key = (path.to_path_buf(), width, height, chain.map(str::to_owned));
        {
            let mut readers = self
                .readers
                .lock()
                .map_err(|_| crate::error::Error::NoFrame {
                    path: path.to_path_buf(),
                })?;
            readers.order.retain(|entry| entry != &key);
            readers.order.push(key.clone());
            if let Some(reader) = readers.warm.get(&key) {
                return Ok(Arc::clone(reader));
            }
        }

        // Opened outside the lock: opening a file and seeking it is the slow
        // part, and nobody else needs to wait for it. A decode in flight on
        // an evicted reader finishes on its own handle.
        let opened = Arc::new(Mutex::new(Reader::open(
            path, width, height, chain, rate, target,
        )?));
        let mut readers = self
            .readers
            .lock()
            .map_err(|_| crate::error::Error::NoFrame {
                path: path.to_path_buf(),
            })?;
        if let Some(reader) = readers.warm.get(&key) {
            // Someone else opened the same reader meanwhile; theirs wins.
            return Ok(Arc::clone(reader));
        }
        while readers.warm.len() >= readers.max && !readers.order.is_empty() {
            let coldest = readers.order.remove(0);
            if coldest == key {
                readers.order.push(coldest);
                break;
            }
            readers.warm.remove(&coldest);
        }
        readers.warm.insert(key, Arc::clone(&opened));
        Ok(opened)
    }

    /// Drops every warm reader and cached frame - for when the media set
    /// changes wholesale, like closing a project.
    pub fn clear(&self) {
        if let Ok(mut readers) = self.readers.lock() {
            readers.warm.clear();
            readers.order.clear();
        }
        if let Ok(mut facts) = self.facts.lock() {
            facts.clear();
        }
        if let Ok(mut cache) = self.cache.lock() {
            let budget = cache.budget;
            *cache = FrameCache::new(budget);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::{EncodeOptions, Encoder, FrameSink};

    #[test]
    fn the_access_policy_rolls_forward_and_seeks_backward() {
        assert_eq!(
            plan_access(Some(10), 10),
            Access::Roll(0),
            "the very next frame"
        );
        assert_eq!(
            plan_access(Some(10), 30),
            Access::Roll(20),
            "a short hop ahead"
        );
        assert_eq!(plan_access(Some(10), 9), Access::Seek, "behind means seek");
        assert_eq!(
            plan_access(Some(10), 10 + ROLL_FORWARD_FRAMES + 1),
            Access::Seek,
            "too far ahead means seek"
        );
        assert_eq!(
            plan_access(None, 5),
            Access::Seek,
            "unknown position means seek"
        );
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
            FrameKey {
                path: PathBuf::from("a"),
                width: 2,
                height: 2,
                chain: None,
                index: 0,
            },
            Arc::new(Frame::black(2, 2)),
        );
        assert!(cache.is_empty(), "16 bytes cannot fit an 8 byte budget");
    }

    /// End to end against the linked FFmpeg: encode a tiny video whose frames
    /// are identifiable by colour, then read arbitrary frames back out of
    /// order.
    #[test]
    fn random_access_returns_the_right_frames() {
        let path = std::env::temp_dir().join("concat-pool-test.mp4");
        let mut encoder =
            Encoder::create(&path, 64, 64, FrameRate::THIRTY, &EncodeOptions::default())
                .expect("the linked FFmpeg encodes h264");
        // 90 frames; the red channel encodes the frame index (x2 to survive
        // compression rounding).
        for index in 0..90u32 {
            let mut frame = Frame::black(64, 64);
            frame.fill([(index * 2).min(255) as u8, 40, 40, 255]);
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");

        let pool = ReaderPool::new(64 * 1024 * 1024, 4);
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
        assert_eq!(
            again_at_50, at_50,
            "revisit must come from cache, identical"
        );
        assert!(
            pool.cached_bytes() > 0,
            "rolling forward populated the cache"
        );

        // Far past the end - a clip that outlives its media. The last real
        // frame is the answer, not an error: a seek there decodes nothing,
        // and nothing must not become a zero-byte frame or a black monitor.
        let past_end = red_at(&mut pool, 300);
        assert!(
            near(past_end, 89),
            "past the end read {past_end}, wanted the last frame"
        );

        let _ = std::fs::remove_file(&path);
    }
}
