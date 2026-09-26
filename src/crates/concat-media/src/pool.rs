// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The reader pool and frame cache: random access to any frame of any file.
//!
//! A decoder on its own reads *forward*: open it, pull frames in order,
//! drop it. That is exactly right for export and exactly wrong for
//! interactive use, where scrubbing asks for arbitrary (media, time) pairs.
//! This module is the answer:
//!
//! - **One reader per (file, level)**, kept warm between requests. A
//!   request near the reader's current position rolls forward (cheap,
//!   exact); a request elsewhere seeks - frame-accurately, guided by the
//!   real timestamps the linked decoder reports.
//! - **A byte-budgeted LRU of source frames** in front of the readers,
//!   keyed by the file, the level it was decoded at and the frame index -
//!   and nothing else. A source frame is the picture as the file holds
//!   it, turned the right way up, brought into BT.709 and scaled to a
//!   level: the file's own size halved as many times as still covers what
//!   was asked for. The crop and the effect chain are not in the key. They
//!   are applied to the cached picture on the way out, and their results
//!   kept in a second, smaller cache, so turning an effect knob costs a
//!   filter per frame and not a decode per frame, and scrubbing back over
//!   ground already covered costs a hash lookup whatever the knobs say.
//! - **Proxies.** A file can be given a stand-in - a smaller copy of
//!   itself - and a request that says so reads that instead, under its own
//!   key. Playback reads the proxy; the paused monitor reads the original.
//!
//! Frames roll into the cache as decoding passes them, so rolling forward
//! to frame N caches N-1 frames of the path there.
//!
//! Export decodes every frame exactly once in order and does not come
//! here, with one exception: a clip that runs backwards or on a speed
//! curve cannot be followed by a paced decoder, and each of its frames is
//! sought through [`ReaderPool::frame_at`], which bakes the chain into the
//! decoder the way the paced decoders beside it do. The two paths of one
//! export then agree to the pixel.

use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use concat_core::frame::Frame;
use concat_core::time::{FrameRate, Rational};

use crate::decode::{ColorRange, DecodeOptions, Decoder, FrameSource, SeekableSource};
use crate::error::Result;
use crate::probe;

/// How far ahead of a reader's position a request may be and still be worth
/// decoding forward to, rather than seeking. Two seconds of 30fps material is
/// sixty decodes - comparable to a seek, and exact.
const ROLL_FORWARD_FRAMES: i64 = 60;

/// What one frame request asks for; see [`ReaderPool::frame`].
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FrameRequest {
    /// The media file.
    pub path: PathBuf,
    /// The instant, in the media's own clock.
    pub time: Rational,
    /// The size the whole picture would have at the scale asked for, crop
    /// or no crop: what picks the level the source is decoded at. A clip
    /// cropped to its middle half and fitted to 960 across needs the whole
    /// picture at 1920, and this is where a caller says so.
    pub cover: (u32, u32),
    /// The size the frame comes out at.
    pub size: (u32, u32),
    /// An image file: one frame, served for every time, which the caller
    /// knows from its own model and the pool must not guess from probing.
    pub still: bool,
    /// The chain run in the picture's own pixels before the fit: the crop.
    pub pre: Option<String>,
    /// The effect chain, run at the output size after the fit.
    pub chain: Option<String>,
    /// Read the file's proxy where it has one; see
    /// [`ReaderPool::adopt_proxy`].
    pub proxy: bool,
    /// The levels the file is read as, over its own tag; `None` reads the
    /// tag. See [`DecodeOptions::color_range`]. Never applied to a proxy:
    /// a proxy was written from the corrected picture and tagged true.
    pub range: Option<ColorRange>,
    /// The caller draws the frame at whatever size it comes: with no `pre`
    /// and no `chain`, the source frame at its level is served as it is,
    /// rather than fitted to `size` first. The compositor fits a picture of
    /// any size into its place, so for it the fit here is a second scale
    /// and a second cache entry per size; the level is never smaller than
    /// `cover`, so nothing is lost. See [`FrameRequest::at_any_size`].
    pub any_size: bool,
    /// Served deep where the source is wide and nothing treats it here:
    /// its own signal at sixteen bits a channel, for a caller whose frames
    /// go straight to the GPU. Only with `any_size`, since a fit here is
    /// FFmpeg on eight bits. See [`DecodeOptions::deep`].
    pub deep: bool,
}

impl FrameRequest {
    /// A plain request: the frame of `path` at `time`, `width` by `height`,
    /// no crop, no chain, the original file.
    pub fn new(path: impl Into<PathBuf>, time: Rational, width: u32, height: u32) -> Self {
        Self {
            path: path.into(),
            time,
            cover: (width, height),
            size: (width, height),
            still: false,
            pre: None,
            chain: None,
            proxy: false,
            range: None,
            any_size: false,
            deep: false,
        }
    }

    /// Reads the file as `range`, whatever it says. See
    /// [`FrameRequest::range`].
    pub fn in_range(mut self, range: Option<ColorRange>) -> Self {
        self.range = range;
        self
    }

    /// Marks the file an image. See [`FrameRequest::still`].
    pub fn as_still(mut self, still: bool) -> Self {
        self.still = still;
        self
    }

    /// Sets the size the whole picture has at this scale. See
    /// [`FrameRequest::cover`].
    pub fn covering(mut self, width: u32, height: u32) -> Self {
        self.cover = (width, height);
        self
    }

    /// Applies `pre` before the fit; empty for none.
    pub fn prefiltered(mut self, pre: Option<&str>) -> Self {
        self.pre = pre
            .filter(|chain| !chain.trim().is_empty())
            .map(str::to_owned);
        self
    }

    /// Applies `chain` after the fit; empty for none.
    pub fn filtered(mut self, chain: Option<&str>) -> Self {
        self.chain = chain
            .filter(|chain| !chain.trim().is_empty())
            .map(str::to_owned);
        self
    }

    /// Served at the source's level when nothing treats the picture; see
    /// [`FrameRequest::any_size`].
    pub fn at_any_size(mut self, any_size: bool) -> Self {
        self.any_size = any_size;
        self
    }

    /// Served deep where it can be; see [`FrameRequest::deep`].
    pub fn deep_when_untreated(mut self, deep: bool) -> Self {
        self.deep = deep;
        self
    }

    /// Reads the proxy where there is one. See [`FrameRequest::proxy`].
    pub fn from_proxy(mut self, proxy: bool) -> Self {
        self.proxy = proxy;
        self
    }
}

/// A source frame's identity: the file, the level it was decoded at, the
/// frame. The stream is implicit - a file's best video stream is the one
/// every reader opens.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct SourceKey {
    path: PathBuf,
    width: u32,
    height: u32,
    index: i64,
    /// The range the file was read as: the same frame read as full and
    /// as video range is two different pictures.
    range: Option<ColorRange>,
    /// Read deep: a wide source in its own signal, sixteen bits a channel
    /// (see [`DecodeOptions::deep`]) - a different picture from the
    /// tone-mapped eight bits.
    deep: bool,
}

/// A treated frame's identity: the source frame and what was done to it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct TreatedKey {
    source: SourceKey,
    width: u32,
    height: u32,
    pre: Option<String>,
    chain: Option<String>,
    /// Made by a decoder with the chain in its graph rather than from a
    /// cached source frame - export's way, see [`ReaderPool::frame_at`].
    /// The crop then ran in the file's own pixels and not the level's, so
    /// the pictures can differ by a rounding and must not share a key.
    baked: bool,
}

/// A byte-budgeted LRU of decoded frames.
///
/// Plain and measurable on purpose: a `HashMap` plus a logical clock, evicting
/// the least-recently-touched frame until the budget holds. At preview sizes a
/// frame is ~2-4 MB, so the default budget holds a few hundred frames - many
/// seconds of scrub history.
pub struct FrameCache<K> {
    budget: usize,
    held: usize,
    tick: u64,
    frames: HashMap<K, (Arc<Frame>, u64)>,
}

impl<K: Hash + Eq + Clone> FrameCache<K> {
    /// An empty cache holding at most `budget` bytes of frames.
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            held: 0,
            tick: 0,
            frames: HashMap::new(),
        }
    }

    fn get(&mut self, key: &K) -> Option<Arc<Frame>> {
        self.tick += 1;
        let tick = self.tick;
        self.frames.get_mut(key).map(|(frame, touched)| {
            *touched = tick;
            Arc::clone(frame)
        })
    }

    fn insert(&mut self, key: K, frame: Arc<Frame>) {
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

    /// Drops every frame `doomed` says to.
    fn drop_where(&mut self, doomed: impl Fn(&K) -> bool) {
        let gone: Vec<K> = self
            .frames
            .keys()
            .filter(|key| doomed(key))
            .cloned()
            .collect();
        for key in gone {
            if let Some((frame, _)) = self.frames.remove(&key) {
                self.held -= frame.pixels().len();
            }
        }
    }

    /// Bytes of frames held.
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

/// An even size no smaller than two on a side: what the scaler and the
/// codecs are happiest with.
fn even(width: u32, height: u32) -> (u32, u32) {
    ((width & !1).max(2), (height & !1).max(2))
}

/// The level a source of `source` is decoded at to cover `cover`: the
/// source halved as many times as still covers it on both sides. A source
/// no bigger than the cover is decoded at its own size; nothing is ever
/// decoded larger than the file.
fn level_for(source: (u32, u32), cover: (u32, u32)) -> (u32, u32) {
    let (mut width, mut height) = even(source.0, source.1);
    loop {
        let (half_w, half_h) = even(width / 2, height / 2);
        if half_w < cover.0 || half_h < cover.1 || (half_w, half_h) == (width, height) {
            break;
        }
        (width, height) = (half_w, half_h);
    }
    (width, height)
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
        pre: Option<&str>,
        range: Option<ColorRange>,
        deep: bool,
        rate: FrameRate,
        still: bool,
        index: i64,
    ) -> Result<Self> {
        // Unpaced, so every source frame comes out with its own timestamp
        // and the index is read from that; a seek lands on the keyframe at
        // or before the target and the pull rolls forward from there.
        let mut options = DecodeOptions::default()
            .starting_at(rate.time_of_frame(index))
            .scaled_to(width, height)
            .in_range(range)
            .deep(deep);
        if still {
            // One frame, served for every time, and never sought: a seek
            // on a single-image JPEG leaves FFmpeg's image demuxer with
            // nothing left to read, so the picture would never come back.
            // Repeating keeps the frame coming after the end instead.
            options = options.repeating();
        }
        if let Some(chain) = chain {
            options = options.filtered(chain);
        }
        if let Some(pre) = pre {
            options = options.prefiltered(pre);
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
    /// The picture's displayed size, for a file with one: what the levels
    /// are halvings of.
    size: Option<(u32, u32)>,
}

impl MediaFacts {
    /// The frame on screen at `time`: index zero for a still, and never
    /// past the last frame the container claims. A clip can outlive its
    /// media - an end-trim past the file's length - and the honest picture
    /// for any time past the end is the last frame.
    fn index_at(&self, time: Rational) -> i64 {
        if self.still {
            return 0;
        }
        let mut target = self.rate.frame_at(time).max(0);
        if let Some(frames) = self.frames {
            target = target.min((frames - 1).max(0));
        }
        target
    }
}

/// One reader's identity: the file, the decode size, the baked chains.
type ReaderKey = (
    PathBuf,
    u32,
    u32,
    Option<String>,
    Option<String>,
    Option<ColorRange>,
    bool,
);

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

/// What the pool has done so far: the numbers a scrub benchmark asserts.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct CacheStats {
    /// Frames that came out of a decoder.
    pub decoded: u64,
    /// Requests answered from a cached source frame without decoding,
    /// treated on the way out or not.
    pub source_hits: u64,
    /// Requests answered from a cached treated frame: no decode, no filter.
    pub treated_hits: u64,
    /// Source frames put through a crop, a chain or a fit on the way out.
    pub treated: u64,
}

impl CacheStats {
    /// What happened between `earlier` and this snapshot.
    pub fn since(self, earlier: CacheStats) -> CacheStats {
        CacheStats {
            decoded: self.decoded - earlier.decoded,
            source_hits: self.source_hits - earlier.source_hits,
            treated_hits: self.treated_hits - earlier.treated_hits,
            treated: self.treated - earlier.treated,
        }
    }
}

#[derive(Default)]
struct Counters {
    decoded: AtomicU64,
    source_hits: AtomicU64,
    treated_hits: AtomicU64,
    treated: AtomicU64,
}

/// Random access to frames across many files, cache in front, warm readers
/// behind.
///
/// Shared, not serialised: every method takes `&self`, and the locks inside
/// are held for as little as each step needs. The caches are short locks;
/// each reader is its own, so the playback stream decoding ahead on one
/// file and the monitor pulling a cached frame of another never queue
/// behind each other. Two callers wanting the *same* reader take turns,
/// which is what a single decoder demands anyway.
pub struct ReaderPool {
    /// Source frames: the file at a level, untreated.
    sources: Mutex<FrameCache<SourceKey>>,
    /// Source frames after their crop, fit and chain, at the output size.
    treated: Mutex<FrameCache<TreatedKey>>,
    readers: Mutex<Readers>,
    facts: Mutex<HashMap<PathBuf, Arc<MediaFacts>>>,
    /// Stills that exist nowhere but here: a title painted while its words
    /// are being pulled about, handed in as pixels under a name no file
    /// has, and served from these at whatever size and through whatever
    /// chain a plan asks - see [`ReaderPool::hold_still`]. The newest
    /// [`STILLS_HELD`] stay.
    memory: Mutex<HeldStills>,
    /// The stand-in for each file that has one; see
    /// [`ReaderPool::adopt_proxy`].
    proxies: Mutex<HashMap<PathBuf, PathBuf>>,
    counters: Counters,
}

/// The stills held in memory, and the order they came in.
type HeldStills = (
    HashMap<PathBuf, Arc<Frame>>,
    std::collections::VecDeque<PathBuf>,
);

/// How many in-memory stills the pool keeps. Each is a frame, mostly
/// transparent, at the monitor's size; a drag makes a few dozen a second
/// and shows one.
const STILLS_HELD: usize = 24;

impl ReaderPool {
    /// A pool with `cache_bytes` of frame cache and up to `max_readers` warm
    /// decoders. A quarter of the budget goes to treated frames, which are
    /// output-sized and few - the frames under and just ahead of the
    /// playhead - and the rest to source frames, which are the scrub
    /// history.
    pub fn new(cache_bytes: usize, max_readers: usize) -> Self {
        let treated = cache_bytes / 4;
        Self {
            sources: Mutex::new(FrameCache::new(cache_bytes - treated)),
            treated: Mutex::new(FrameCache::new(treated)),
            readers: Mutex::new(Readers {
                warm: HashMap::new(),
                order: Vec::new(),
                max: max_readers.max(1),
            }),
            facts: Mutex::new(HashMap::new()),
            memory: Mutex::new((HashMap::new(), std::collections::VecDeque::new())),
            proxies: Mutex::new(HashMap::new()),
            counters: Counters::default(),
        }
    }

    /// 512 MB of frames, eight warm readers - enough for a busy timeline.
    pub fn with_defaults() -> Self {
        Self::new(512 * 1024 * 1024, 8)
    }

    /// Makes `frame` the still at `path`, a name no file has, until
    /// [`STILLS_HELD`] newer ones have come. Frames already cut from an
    /// earlier still of that name are let go, so the name never shows
    /// stale pixels.
    pub fn hold_still(&self, path: &Path, frame: Arc<Frame>) {
        if let Ok(mut held) = self.memory.lock() {
            let (stills, order) = &mut *held;
            if stills.insert(path.to_path_buf(), frame).is_none() {
                order.push_back(path.to_path_buf());
            }
            while order.len() > STILLS_HELD {
                if let Some(oldest) = order.pop_front() {
                    stills.remove(&oldest);
                }
            }
        }
        self.forget(path);
        if let Ok(mut facts) = self.facts.lock() {
            facts.insert(
                path.to_path_buf(),
                Arc::new(MediaFacts {
                    rate: FrameRate::THIRTY,
                    still: true,
                    frames: None,
                    size: None,
                }),
            );
        }
    }

    /// The still held under `path`, if one is.
    fn held_still(&self, path: &Path) -> Option<Arc<Frame>> {
        self.memory.lock().ok()?.0.get(path).cloned()
    }

    /// Makes `proxy` the file read for `original` by every request that
    /// asks for a proxy: a smaller copy, the same frames at the same
    /// instants. Cached under its own name, so the original's frames stay.
    pub fn adopt_proxy(&self, original: &Path, proxy: PathBuf) {
        if let Ok(mut proxies) = self.proxies.lock() {
            proxies.insert(original.to_path_buf(), proxy);
        }
    }

    /// The proxy adopted for `original`, if one was.
    pub fn proxy_of(&self, original: &Path) -> Option<PathBuf> {
        self.proxies.lock().ok()?.get(original).cloned()
    }

    /// Drops every cached frame cut from `path` and its probed facts: for
    /// a file that changed on disk.
    pub fn forget(&self, path: &Path) {
        if let Ok(mut sources) = self.sources.lock() {
            sources.drop_where(|key| key.path == path);
        }
        if let Ok(mut treated) = self.treated.lock() {
            treated.drop_where(|key| key.source.path == path);
        }
        if let Ok(mut facts) = self.facts.lock() {
            facts.remove(path);
        }
    }

    /// Bytes of decoded frames currently cached, source and treated.
    pub fn cached_bytes(&self) -> usize {
        let sources = self.sources.lock().map(|cache| cache.held()).unwrap_or(0);
        let treated = self.treated.lock().map(|cache| cache.held()).unwrap_or(0);
        sources + treated
    }

    /// What the pool has done so far.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            decoded: self.counters.decoded.load(Ordering::Relaxed),
            source_hits: self.counters.source_hits.load(Ordering::Relaxed),
            treated_hits: self.counters.treated_hits.load(Ordering::Relaxed),
            treated: self.counters.treated.load(Ordering::Relaxed),
        }
    }

    fn count(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn cached_source(&self, key: &SourceKey) -> Option<Arc<Frame>> {
        self.sources.lock().ok()?.get(key)
    }

    fn remember_source(&self, key: SourceKey, frame: Arc<Frame>) {
        if let Ok(mut cache) = self.sources.lock() {
            cache.insert(key, frame);
        }
    }

    fn cached_treated(&self, key: &TreatedKey) -> Option<Arc<Frame>> {
        self.treated.lock().ok()?.get(key)
    }

    fn remember_treated(&self, key: TreatedKey, frame: Arc<Frame>) {
        if let Ok(mut cache) = self.treated.lock() {
            cache.insert(key, frame);
        }
    }

    /// The file a request reads: its proxy when it asks for one and the
    /// file has one, the file itself otherwise.
    fn path_for(&self, request: &FrameRequest) -> PathBuf {
        if request.proxy
            && let Some(proxy) = self.proxy_of(&request.path)
        {
            return proxy;
        }
        request.path.clone()
    }

    /// The source frame a request needs and where it sits: the file it
    /// reads, its facts, and the key of the frame at the level that covers
    /// the request.
    fn locate(&self, request: &FrameRequest) -> Result<(Arc<MediaFacts>, SourceKey)> {
        let mut path = self.path_for(request);
        let facts = match self.facts_for(&path, request.still) {
            Ok(facts) => facts,
            // A proxy that cannot be read - the cache cleared under the
            // pool, a folder moved - is forgotten, and the original read
            // instead: a stand-in that has gone is not a missing picture.
            Err(error) if path != request.path => {
                log::warn!(
                    "{}: proxy {} unreadable ({error}); reading the original",
                    request.path.display(),
                    path.display()
                );
                if let Ok(mut proxies) = self.proxies.lock() {
                    proxies.remove(&request.path);
                }
                path = request.path.clone();
                self.facts_for(&path, request.still)?
            }
            Err(error) => return Err(error),
        };
        let index = facts.index_at(request.time);
        // A still is cut to the size asked for: one decode, kept, and the
        // levels of a photograph are not worth a photograph's worth of
        // memory each.
        let (width, height) = match facts.size {
            Some(size) if !facts.still => level_for(size, request.cover),
            _ => even(request.cover.0, request.cover.1),
        };
        // A proxy is read as it is tagged: it was written from the
        // corrected picture, so the correction applied twice would undo it.
        let range = (path == request.path).then_some(request.range).flatten();
        // Deep only where the frame is served as it is: a fit or a chain
        // here is FFmpeg, which takes eight bits.
        let deep =
            request.deep && request.any_size && request.pre.is_none() && request.chain.is_none();
        Ok((
            facts,
            SourceKey {
                path,
                width,
                height,
                index,
                range,
                deep,
            },
        ))
    }

    /// The source frame a request needs: the file at the level that covers
    /// it, untreated. What a compositor that applies the crop, the chain
    /// and the placement itself asks for.
    pub fn source(&self, request: &FrameRequest) -> Result<Arc<Frame>> {
        let (facts, key) = self.locate(request)?;
        self.source_frame(&facts, &key)
    }

    /// The frame a request describes: the source frame at its level,
    /// cropped, fitted to the size asked for, and run through the chain.
    /// Cached twice over - the source frame under its own key, the treated
    /// picture under the source's key plus what was done to it - so an
    /// effect edit costs a filter and a scrub over covered ground costs a
    /// lookup. The same `Arc` comes back for the same request while it is
    /// cached, so a compositor that remembers what it uploaded by the
    /// frame's identity uploads it once.
    pub fn frame(&self, request: &FrameRequest) -> Result<Arc<Frame>> {
        let (facts, source_key) = self.locate(request)?;
        let (width, height) = request.size;
        let plain = request.pre.is_none()
            && request.chain.is_none()
            && (request.any_size || (source_key.width, source_key.height) == (width, height));
        if plain {
            return self.source_frame(&facts, &source_key);
        }
        let key = TreatedKey {
            source: source_key,
            width,
            height,
            pre: request.pre.clone(),
            chain: request.chain.clone(),
            baked: false,
        };
        if let Some(frame) = self.cached_treated(&key) {
            Self::count(&self.counters.treated_hits);
            return Ok(frame);
        }
        let source = self.source_frame(&facts, &key.source)?;
        let treated = Arc::new(crate::treat::fit(
            &source,
            request.pre.as_deref(),
            width,
            height,
            request.chain.as_deref(),
        )?);
        Self::count(&self.counters.treated);
        self.remember_treated(key, Arc::clone(&treated));
        Ok(treated)
    }

    /// The frame of `path` on screen at `time` (in the media's own clock),
    /// scaled to `width` x `height`, with `pre` and `chain` baked into the
    /// decoder's own graph: the crop in the file's pixels, the fit, the
    /// chain at the output size.
    ///
    /// Export's way of reading a frame, and the pixels a paced decoder with
    /// the same options makes. The monitor asks through
    /// [`ReaderPool::frame`] instead, whose cache the chain does not
    /// invalidate. Without a crop or a chain the two are the same request.
    /// `deep` reads a wide source in its own signal where there is no crop
    /// and no chain; see [`DecodeOptions::deep`].
    #[allow(clippy::too_many_arguments)]
    pub fn frame_at(
        &self,
        path: &Path,
        time: Rational,
        width: u32,
        height: u32,
        still: bool,
        chain: Option<&str>,
        pre: Option<&str>,
        range: Option<ColorRange>,
        deep: bool,
    ) -> Result<Arc<Frame>> {
        let chain = chain.filter(|chain| !chain.trim().is_empty());
        let pre = pre.filter(|pre| !pre.trim().is_empty());
        let facts = self.facts_for(path, still)?;
        let index = facts.index_at(time);
        let source_key = SourceKey {
            path: path.to_path_buf(),
            width,
            height,
            index,
            range,
            deep: deep && chain.is_none() && pre.is_none(),
        };
        if chain.is_none() && pre.is_none() {
            return self.source_frame(&facts, &source_key);
        }
        let key = TreatedKey {
            source: source_key,
            width,
            height,
            pre: pre.map(str::to_owned),
            chain: chain.map(str::to_owned),
            baked: true,
        };
        if let Some(frame) = self.cached_treated(&key) {
            Self::count(&self.counters.treated_hits);
            return Ok(frame);
        }
        // A still held in memory has no file to open: it is cut to the size
        // and through the chain asked for, here, and kept like any frame.
        if let Some(source) = self.held_still(path) {
            let spec: String = [pre, chain]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(",");
            let frame = Arc::new(crate::treat::treat_to(&source, width, height, &spec)?);
            Self::count(&self.counters.treated);
            self.remember_treated(key, Arc::clone(&frame));
            return Ok(frame);
        }
        let shared = self.reader(path, width, height, chain, pre, range, false, &facts, index)?;
        let mut reader = shared.lock().map_err(|_| crate::error::Error::NoFrame {
            path: path.to_path_buf(),
        })?;
        // Another caller may have moved this reader while this one waited
        // for it; the answer may even be cached by now.
        if let Some(frame) = self.cached_treated(&key) {
            Self::count(&self.counters.treated_hits);
            return Ok(frame);
        }
        let remember = |at: i64, frame: Arc<Frame>| {
            let mut key = key.clone();
            key.source.index = at;
            self.remember_treated(key, frame);
        };
        self.pull(&mut reader, &facts, index, &remember)?
            .ok_or_else(|| crate::error::Error::NoFrame {
                path: path.to_path_buf(),
            })
    }

    /// The source frame `key` names, from the cache or a reader.
    fn source_frame(&self, facts: &MediaFacts, key: &SourceKey) -> Result<Arc<Frame>> {
        if let Some(frame) = self.cached_source(key) {
            Self::count(&self.counters.source_hits);
            return Ok(frame);
        }
        let path = key.path.as_path();
        // A still held in memory has no file to open: it is cut to the
        // level asked for, here, and kept like any frame.
        if let Some(source) = self.held_still(path) {
            let frame = Arc::new(crate::treat::treat_to(&source, key.width, key.height, "")?);
            self.remember_source(key.clone(), Arc::clone(&frame));
            return Ok(frame);
        }
        let shared = self.reader(
            path, key.width, key.height, None, None, key.range, key.deep, facts, key.index,
        )?;
        let mut reader = shared.lock().map_err(|_| crate::error::Error::NoFrame {
            path: path.to_path_buf(),
        })?;
        // Another caller may have moved this reader while this one waited
        // for it; the answer may even be cached by now.
        if let Some(frame) = self.cached_source(key) {
            Self::count(&self.counters.source_hits);
            return Ok(frame);
        }
        let remember = |at: i64, frame: Arc<Frame>| {
            let mut key = key.clone();
            key.index = at;
            self.remember_source(key, frame);
        };
        self.pull(&mut reader, facts, key.index, &remember)?
            .ok_or_else(|| crate::error::Error::NoFrame {
                path: path.to_path_buf(),
            })
    }

    /// Brings `reader` to `target` and returns the frame there, handing
    /// every frame decoded on the way to `remember` under its own index -
    /// the next scrub over this span is then free. `None` only when the
    /// file has no decodable picture at all.
    fn pull(
        &self,
        reader: &mut Reader,
        facts: &MediaFacts,
        target: i64,
        remember: &dyn Fn(i64, Arc<Frame>),
    ) -> Result<Option<Arc<Frame>>> {
        let rate = facts.rate;
        let next = (reader.next_index != i64::MIN).then_some(reader.next_index);
        if !facts.still && plan_access(next, target) == Access::Seek {
            reader.seek(rate, target)?;
        }

        // Roll forward to the target, caching everything passed on the way.
        // Stops at end of stream too: a clip trimmed past its media's end,
        // where the last real frame is the honest answer.
        let mut latest: Option<Arc<Frame>> = None;
        let mut reached = i64::MIN;
        while let Some((index, frame)) = reader.next_frame(rate)? {
            Self::count(&self.counters.decoded);
            let frame = Arc::new(frame);
            remember(index, Arc::clone(&frame));
            latest = Some(frame);
            reached = index;
            if index >= target {
                break;
            }
        }
        // The stream ended short of the target: the last frame is the
        // answer, and it is remembered under the index asked for, so a
        // container that claims a frame more than it holds does not cost a
        // seek and a walk every time the end is asked for.
        if reached < target
            && let Some(frame) = &latest
        {
            remember(target, Arc::clone(frame));
        }

        // Nothing at all means the seek itself landed past the end of the
        // picture stream - a container whose video ends before its audio, or
        // a stated duration that lied past the clamp. The last real frame is
        // still the honest answer; it just has to be found by seeking
        // backwards until something decodes. Each retry doubles the step,
        // and the last one starts from zero, so a file with any decodable
        // picture at all cannot fail here.
        if latest.is_none() && !facts.still {
            for step in [30i64, 240, i64::MAX] {
                let from = target.saturating_sub(step).max(0);
                reader.seek(rate, from)?;
                while let Some((index, frame)) = reader.next_frame(rate)? {
                    Self::count(&self.counters.decoded);
                    let frame = Arc::new(frame);
                    remember(index, Arc::clone(&frame));
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
                remember(target, Arc::clone(frame));
            }
        }
        Ok(latest)
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
                    size: None,
                }
            } else {
                let info = probe::probe(path)?;
                let rate = info
                    .video
                    .as_ref()
                    .map(|video| video.frame_rate)
                    .unwrap_or(FrameRate::THIRTY);
                let size = info
                    .video
                    .as_ref()
                    .map(|video| (video.width, video.height))
                    .filter(|(width, height)| *width > 0 && *height > 0);
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
                    size,
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
        pre: Option<&str>,
        range: Option<ColorRange>,
        deep: bool,
        facts: &MediaFacts,
        target: i64,
    ) -> Result<Arc<Mutex<Reader>>> {
        let key = (
            path.to_path_buf(),
            width,
            height,
            chain.map(str::to_owned),
            pre.map(str::to_owned),
            range,
            deep,
        );
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
            path,
            width,
            height,
            chain,
            pre,
            range,
            deep,
            facts.rate,
            facts.still,
            target,
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

    /// Drops every warm reader, cached frame and adopted proxy - for when
    /// the media set changes wholesale, like closing a project.
    pub fn clear(&self) {
        if let Ok(mut readers) = self.readers.lock() {
            readers.warm.clear();
            readers.order.clear();
        }
        if let Ok(mut facts) = self.facts.lock() {
            facts.clear();
        }
        if let Ok(mut cache) = self.sources.lock() {
            let budget = cache.budget;
            *cache = FrameCache::new(budget);
        }
        if let Ok(mut cache) = self.treated.lock() {
            let budget = cache.budget;
            *cache = FrameCache::new(budget);
        }
        if let Ok(mut proxies) = self.proxies.lock() {
            proxies.clear();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
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

    /// The level is the source halved while it still covers the request,
    /// never smaller than the request and never larger than the source.
    #[test]
    fn a_level_is_the_source_halved_while_it_still_covers() {
        assert_eq!(level_for((1920, 1080), (960, 540)), (960, 540));
        assert_eq!(level_for((3840, 2160), (960, 540)), (960, 540));
        assert_eq!(level_for((1920, 1080), (1280, 720)), (1920, 1080));
        assert_eq!(level_for((1920, 1080), (1000, 500)), (1920, 1080));
        assert_eq!(
            level_for((640, 360), (1280, 720)),
            (640, 360),
            "never upscaled"
        );
        assert_eq!(level_for((1080, 1920), (270, 480)), (270, 480));
        assert_eq!(level_for((7, 5), (2, 2)), (2, 2), "even, and at least two");
    }

    #[test]
    fn the_cache_evicts_least_recently_used_within_budget() {
        // Frames are 16 bytes each (2x2); budget holds exactly two.
        let mut cache = FrameCache::new(32);
        let key = |index: i64| SourceKey {
            path: PathBuf::from("a.mp4"),
            width: 2,
            height: 2,
            index,
            range: None,
            deep: false,
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

    /// A still held in memory is served under its name at the size asked
    /// for, and a newer still under the same name replaces it, frames cut
    /// from the old one included.
    #[test]
    fn a_held_still_is_served_at_any_size_and_replaced_whole() {
        let pool = ReaderPool::new(64 * 1024 * 1024, 2);
        let path = Path::new("memory://titles/test");
        let mut first = Frame::black(64, 32);
        first.fill([255, 0, 0, 255]);
        pool.hold_still(path, Arc::new(first));
        let same = pool
            .frame_at(path, Rational::ZERO, 64, 32, true, None, None, None, false)
            .expect("its own size");
        assert_eq!((same.width(), same.height()), (64, 32));
        assert_eq!(&same.pixels()[..4], &[255, 0, 0, 255]);
        let half = pool
            .frame(&FrameRequest::new(path, Rational::ZERO, 32, 16).as_still(true))
            .expect("half size");
        assert_eq!((half.width(), half.height()), (32, 16));
        assert_eq!(half.pixels()[0], 255, "still red when scaled");
        let mut second = Frame::black(64, 32);
        second.fill([0, 0, 255, 255]);
        pool.hold_still(path, Arc::new(second));
        let again = pool
            .frame(&FrameRequest::new(path, Rational::ZERO, 32, 16).as_still(true))
            .expect("half size again");
        assert_eq!(
            again.pixels()[2],
            255,
            "the old cut is gone with the old still"
        );
    }

    #[test]
    fn an_oversized_frame_is_served_but_never_cached() {
        let mut cache = FrameCache::new(8);
        cache.insert(
            SourceKey {
                path: PathBuf::from("a"),
                width: 2,
                height: 2,
                index: 0,
                range: None,
                deep: false,
            },
            Arc::new(Frame::black(2, 2)),
        );
        assert!(cache.is_empty(), "16 bytes cannot fit an 8 byte budget");
    }

    /// A short video whose frames are identifiable by colour: the red
    /// channel encodes the frame index (x2 to survive compression
    /// rounding).
    pub(crate) fn counting_video(name: &str, size: u32, frames: u32) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("concat-pool-{name}-{}.mp4", std::process::id()));
        let mut encoder = Encoder::create(
            &path,
            size,
            size,
            FrameRate::THIRTY,
            &EncodeOptions::default(),
        )
        .expect("the linked FFmpeg encodes h264");
        for index in 0..frames {
            let mut frame = Frame::black(size, size);
            frame.fill([(index * 2).min(255) as u8, 40, 40, 255]);
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");
        path
    }

    /// End to end against the linked FFmpeg: encode a tiny video whose frames
    /// are identifiable by colour, then read arbitrary frames back out of
    /// order.
    #[test]
    fn random_access_returns_the_right_frames() {
        let path = counting_video("random", 64, 90);

        let pool = ReaderPool::new(64 * 1024 * 1024, 4);
        let rate = FrameRate::THIRTY;
        let red_at = |pool: &ReaderPool, index: i64| -> i64 {
            let frame = pool
                .frame_at(
                    &path,
                    rate.time_of_frame(index),
                    64,
                    64,
                    false,
                    None,
                    None,
                    None,
                    false,
                )
                .expect("frame decodes");
            i64::from(frame.pixel(32, 32).expect("in bounds")[0])
        };

        // Forward, backward, far jump, and revisit - the scrub shapes.
        let near = |got: i64, index: i64| (got - index * 2).abs() <= 8;
        let at_10 = red_at(&pool, 10);
        assert!(near(at_10, 10), "frame 10 read {at_10}");
        let at_50 = red_at(&pool, 50);
        assert!(near(at_50, 50), "frame 50 read {at_50}");
        let back_at_20 = red_at(&pool, 20);
        assert!(near(back_at_20, 20), "backward to 20 read {back_at_20}");
        let again_at_50 = red_at(&pool, 50);
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
        let past_end = red_at(&pool, 300);
        assert!(
            near(past_end, 89),
            "past the end read {past_end}, wanted the last frame"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// The scrub benchmark. One pass over the footage decodes it; a second
    /// pass with an effect on costs no decode, only a filter; a third pass
    /// with the crop changed costs no decode either; and the fourth pass,
    /// back to the first pass's knobs, costs nothing at all. Timings are
    /// printed for a person; the counts are what is asserted.
    /// A request at any size with nothing to treat is served the source
    /// frame at its level, fitted by no one here: no treated frame is made
    /// and every size asked for is the same cached frame. With a chain it
    /// is fitted and treated as before, `any_size` notwithstanding.
    #[test]
    fn an_untreated_frame_at_any_size_is_the_source_itself() {
        let path = counting_video("any-size", 64, 30);
        let pool = ReaderPool::new(64 * 1024 * 1024, 2);
        let time = FrameRate::THIRTY.time_of_frame(5);
        let before = pool.stats();
        let small = pool
            .frame(&FrameRequest::new(&path, time, 20, 20).at_any_size(true))
            .expect("decodes");
        let larger = pool
            .frame(&FrameRequest::new(&path, time, 30, 30).at_any_size(true))
            .expect("decodes");
        assert!(Arc::ptr_eq(&small, &larger), "one cached source frame");
        assert!(
            small.width() >= 30 && small.height() >= 30,
            "the level covers"
        );
        assert_eq!(pool.stats().since(before).treated, 0);

        let treated = pool
            .frame(
                &FrameRequest::new(&path, time, 20, 20)
                    .at_any_size(true)
                    .filtered(Some("negate")),
            )
            .expect("treats");
        assert_eq!((treated.width(), treated.height()), (20, 20));
        assert_eq!(pool.stats().since(before).treated, 1);
    }

    #[test]
    fn a_scrub_over_covered_ground_never_decodes_again() {
        // The first half of the file only: the mp4 muxer leaves the last
        // frame's duration unwritten, so the probed rate of a short
        // synthetic file runs a hair fast and the second half's indices
        // land one frame late. Real footage is thousands of frames, where
        // the hair is nothing; a benchmark wants exact counts.
        let frames: u32 = 50;
        let path = counting_video("scrub", 64, 120);
        let pool = ReaderPool::new(64 * 1024 * 1024, 4);
        let rate = FrameRate::THIRTY;
        // Forward and back again, at half size: what a monitor asks.
        let route: Vec<i64> = (0..i64::from(frames))
            .chain((0..i64::from(frames)).rev())
            .collect();

        let pass = |label: &str, pre: Option<&str>, chain: Option<&str>| -> (CacheStats, i64) {
            let before = pool.stats();
            let started = std::time::Instant::now();
            let mut sample = 0;
            for &index in &route {
                let request = FrameRequest::new(&path, rate.time_of_frame(index), 32, 32)
                    .prefiltered(pre)
                    .filtered(chain);
                let frame = pool.frame(&request).expect("frame decodes");
                assert_eq!((frame.width(), frame.height()), (32, 32));
                if index == 20 {
                    sample = i64::from(frame.pixel(16, 16).expect("in bounds")[0]);
                }
            }
            let stats = pool.stats().since(before);
            println!(
                "{label}: {} requests in {:?}; {stats:?}",
                route.len(),
                started.elapsed(),
            );
            (stats, sample)
        };

        let (first, plain) = pass("plain", None, None);
        assert!(
            first.decoded >= u64::from(frames),
            "the first pass decodes the footage: {first:?}"
        );
        assert!((plain - 40).abs() <= 8, "frame 20 read red {plain}");
        assert_eq!(
            first.source_hits,
            u64::from(frames),
            "the way back is source hits: {first:?}"
        );

        // An effect knob turned: every frame is a source hit and a filter,
        // and the pixels are the effect's.
        let (effected, negated) = pass("negate", None, Some("negate"));
        assert_eq!(
            effected.decoded, 0,
            "an effect must not decode: {effected:?}"
        );
        assert_eq!(effected.source_hits, u64::from(frames), "{effected:?}");
        assert_eq!(effected.treated, u64::from(frames), "{effected:?}");
        assert_eq!(
            effected.treated_hits,
            u64::from(frames),
            "the way back is treated hits: {effected:?}"
        );
        assert!(negated > 180, "negated red is high, read {negated}");

        // The crop changed: the same, through the crop.
        let (cropped, _) = pass(
            "crop",
            Some("crop=w=floor(iw*0.5/2)*2:h=floor(ih*0.5/2)*2:x=0:y=0"),
            Some("negate"),
        );
        assert_eq!(cropped.decoded, 0, "a crop must not decode: {cropped:?}");
        assert_eq!(cropped.source_hits, u64::from(frames), "{cropped:?}");

        // Back to plain: the level is the size asked for, so every request
        // is the source frame itself - and the very same frames.
        let (again, plain_again) = pass("plain again", None, None);
        assert_eq!(again.decoded, 0, "{again:?}");
        assert_eq!(again.treated, 0, "{again:?}");
        assert_eq!(again.source_hits, route.len() as u64, "{again:?}");
        assert_eq!(plain_again, plain);

        let _ = std::fs::remove_file(&path);
    }

    /// A container that claims one frame more than it holds: the request
    /// for that frame gets the last real one, and asking again costs a
    /// lookup, not another seek and walk from the keyframe.
    #[test]
    fn a_request_past_the_last_frame_is_remembered_under_its_index() {
        let path = counting_video("past-end", 32, 30);
        let pool = ReaderPool::new(64 * 1024 * 1024, 1);
        let rate = FrameRate::THIRTY;
        // Straight to the end and past it, twice over.
        let past = FrameRequest::new(&path, rate.time_of_frame(200), 32, 32);
        let first = pool.frame(&past).expect("the last frame stands in");
        let before = pool.stats();
        let again = pool.frame(&past).expect("still");
        assert!(Arc::ptr_eq(&first, &again), "the same remembered frame");
        assert_eq!(pool.stats().since(before).decoded, 0, "no second walk");
        let _ = std::fs::remove_file(&path);
    }

    /// A proxy that has gone from disk is forgotten on the first request
    /// that asks for it, and the original serves the frame.
    #[test]
    fn a_proxy_that_vanished_falls_back_to_the_original() {
        let path = counting_video("proxy-gone", 32, 10);
        let pool = ReaderPool::new(64 * 1024 * 1024, 2);
        let gone =
            std::env::temp_dir().join(format!("concat-no-such-proxy-{}.mp4", std::process::id()));
        pool.adopt_proxy(&path, gone.clone());
        assert_eq!(pool.proxy_of(&path), Some(gone));
        let frame = pool
            .frame(
                &FrameRequest::new(&path, FrameRate::THIRTY.time_of_frame(3), 32, 32)
                    .from_proxy(true),
            )
            .expect("the original serves the frame");
        assert_eq!((frame.width(), frame.height()), (32, 32));
        assert!(
            pool.proxy_of(&path).is_none(),
            "the dead proxy is forgotten"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A request that says so reads the adopted proxy, under its own key;
    /// a request that does not reads the original.
    #[test]
    fn a_proxy_is_read_when_asked_for_and_the_original_otherwise() {
        let original = counting_video("proxied", 64, 30);
        let proxy = counting_video("proxy", 32, 30);
        let pool = ReaderPool::new(64 * 1024 * 1024, 4);
        pool.adopt_proxy(&original, proxy.clone());
        let rate = FrameRate::THIRTY;

        let request = FrameRequest::new(&original, rate.time_of_frame(10), 16, 16);
        let from_original = pool.source(&request).expect("original decodes");
        assert_eq!(
            (from_original.width(), from_original.height()),
            (16, 16),
            "64 halved twice covers 16"
        );
        let from_proxy = pool
            .source(&request.clone().from_proxy(true))
            .expect("proxy decodes");
        assert_eq!((from_proxy.width(), from_proxy.height()), (16, 16));
        let stats = pool.stats();
        assert!(stats.decoded >= 2, "both files were read: {stats:?}");
        assert_eq!(pool.proxy_of(&original), Some(proxy.clone()));

        pool.clear();
        assert_eq!(pool.proxy_of(&original), None, "cleared with the rest");
        let _ = std::fs::remove_file(&original);
        let _ = std::fs::remove_file(&proxy);
    }

    /// A JPEG still through a pool that cannot cache: every request has to
    /// decode afresh from the same warm reader. FFmpeg's image demuxer
    /// reads a single JPEG once and never again after a seek, so this
    /// fails the moment a still is sought - which is every request, unless
    /// stills are read repeating and never sought.
    #[test]
    fn a_jpeg_still_is_served_again_and_again_without_a_seek() {
        let path = std::env::temp_dir().join("concat-pool-still.jpg");
        let mut frame = Frame::black(64, 64);
        frame.fill([200, 30, 30, 255]);
        let bytes = crate::encode::jpeg(&frame, 2).expect("the linked FFmpeg encodes jpeg");
        std::fs::write(&path, bytes).expect("writes the still");

        let pool = ReaderPool::new(1, 4);
        for attempt in 0..3 {
            let got = pool
                .frame(
                    &FrameRequest::new(&path, FrameRate::THIRTY.time_of_frame(attempt), 64, 64)
                        .as_still(true),
                )
                .unwrap_or_else(|error| panic!("request {attempt} decodes: {error}"));
            let red = got.pixel(32, 32).expect("in bounds")[0];
            assert!(
                red > 150,
                "request {attempt} read red {red}, wanted the still"
            );
        }

        let _ = std::fs::remove_file(&path);
    }
}
