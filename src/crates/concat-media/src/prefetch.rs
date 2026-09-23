// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Decoding ahead of the playhead, and the one queue everything else
//! decodes on.
//!
//! The reader pool answers the frame under the playhead; this is what
//! makes that answer a cache hit. A [`Prefetcher`] owns the pool and a few
//! worker threads, and the transport tells it where the playhead is and
//! which way it is going - a [`Cursor`] - together with the instants ahead
//! of it and the frames each needs. The workers decode those into the
//! pool, nearest first, within a small budget, and hold what they decoded
//! until the playhead has passed it; a request the playhead has already
//! passed is dropped unrun, and a cursor that moves supersedes the
//! instants queued for the last one.
//!
//! The same workers take everything else that decodes for a picture on
//! screen - filmstrip tiles, the bin's thumbnails and waveforms, a proxy
//! being written - at lower priorities, so an import of twenty files is
//! twenty jobs waiting their turn on the same few threads and never twenty
//! decoders at once (#52). One worker always stays clear of the low
//! priorities, so the artwork of a big import cannot make playback wait
//! behind it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use concat_core::frame::Frame;

use crate::pool::{FrameRequest, ReaderPool};

/// How many frame instants the prefetcher decodes ahead of the cursor.
/// A quarter of a second at thirty: enough that a decode never lands
/// after its frame is due, small enough that a seek wastes little.
pub const AHEAD: u32 = 8;

/// What a job is for, most urgent first. A worker takes the most urgent
/// job waiting; among jobs of one priority, the first queued.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Priority {
    /// The frames the playhead is about to reach.
    Playback,
    /// A clip's filmstrip, drawn on the timeline.
    Filmstrip,
    /// The bin's thumbnails and waveforms.
    Artwork,
    /// A proxy being written: minutes of work nobody is waiting on.
    Proxy,
}

impl Priority {
    /// Every priority, most urgent first.
    pub const ALL: [Priority; 4] = [
        Priority::Playback,
        Priority::Filmstrip,
        Priority::Artwork,
        Priority::Proxy,
    ];

    /// Whether a job of this priority is background work: what the
    /// reserved worker never takes, and what runs one at a time for a
    /// proxy.
    fn background(self) -> bool {
        matches!(self, Priority::Artwork | Priority::Proxy)
    }
}

/// Which way the transport is going.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Time increasing: play, or a scrub to the right.
    Forward,
    /// Time decreasing: shuttle, or a scrub to the left.
    Backward,
}

/// Where the playhead is and how it moves, as the transport reports it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cursor {
    /// The playhead, in timeline seconds.
    pub time: f64,
    /// Which way the next frame lies.
    pub direction: Direction,
    /// Timeline seconds a wall second: one for play, two for double
    /// speed, zero for a scrub or a pause. What tells a reader of proxies
    /// whether the picture is moving.
    pub rate: f64,
}

impl Cursor {
    /// Whether `time` is behind the cursor: already passed in its
    /// direction, by more than a frame's slack.
    fn passed(self, time: f64, slack: f64) -> bool {
        match self.direction {
            Direction::Forward => time < self.time - slack,
            Direction::Backward => time > self.time + slack,
        }
    }
}

/// One instant ahead of the cursor and the frames it is made of.
#[derive(Clone, Debug)]
pub struct Moment {
    /// The instant, in timeline seconds.
    pub time: f64,
    /// Every source frame the composite at that instant needs.
    pub frames: Vec<FrameRequest>,
}

type Job = Box<dyn FnOnce() + Send>;

/// The queue the workers take from: one lane per priority.
struct Queue {
    lanes: [VecDeque<Job>; Priority::ALL.len()],
    /// Background jobs running now; see [`Priority::background`].
    background_running: usize,
    /// Proxy jobs running now: at most one.
    proxy_running: usize,
    /// Jobs of any priority running now.
    running: usize,
    closed: bool,
}

struct Lane {
    queue: Mutex<Queue>,
    ready: Condvar,
    /// Fires whenever a job finishes, for [`Prefetcher::drain`].
    done: Condvar,
    workers: usize,
}

impl Lane {
    /// The most urgent job a worker may take now, honouring the reserve:
    /// background work never takes the last free worker, and a proxy runs
    /// alone among proxies.
    fn take(&self, queue: &mut Queue) -> Option<(Priority, Job)> {
        let reserve = if self.workers > 1 { 1 } else { 0 };
        for priority in Priority::ALL {
            if priority.background() && queue.background_running + reserve >= self.workers {
                continue;
            }
            if priority == Priority::Proxy && queue.proxy_running > 0 {
                continue;
            }
            if let Some(job) = queue.lanes[priority as usize].pop_front() {
                return Some((priority, job));
            }
        }
        None
    }
}

/// What the playback jobs share with the prefetcher: the cursor as last
/// reported, and the frames decoded ahead of it, held until passed.
struct Shared {
    pool: Arc<ReaderPool>,
    cursor: Mutex<Option<Cursor>>,
    /// Bumped on every cursor: a job made for an older one is stale.
    generation: AtomicU64,
    /// Frames decoded ahead, by instant, held so the pool's eviction
    /// cannot take them before they are shown; released once passed.
    pinned: Mutex<Vec<(f64, Vec<Arc<Frame>>)>>,
    /// A frame's worth of slack, in seconds, when deciding what is passed.
    slack: Mutex<f64>,
}

/// The scheduler: owns the readers, decodes ahead of the transport, and
/// runs everything else that decodes for the screen, in priority order,
/// on a few threads. See the module docs.
pub struct Prefetcher {
    shared: Arc<Shared>,
    lane: Arc<Lane>,
}

impl Prefetcher {
    /// A prefetcher over `pool` with `workers` threads, one at least.
    pub fn new(pool: Arc<ReaderPool>, workers: usize) -> Self {
        let workers = workers.max(1);
        let lane = Arc::new(Lane {
            queue: Mutex::new(Queue {
                lanes: Default::default(),
                background_running: 0,
                proxy_running: 0,
                running: 0,
                closed: false,
            }),
            ready: Condvar::new(),
            done: Condvar::new(),
            workers,
        });
        for index in 0..workers {
            let lane = Arc::clone(&lane);
            if let Err(error) = std::thread::Builder::new()
                .name(format!("concat-decode-{index}"))
                .spawn(move || work(&lane))
            {
                log::error!("decode worker {index} could not start: {error}");
            }
        }
        Self {
            shared: Arc::new(Shared {
                pool,
                cursor: Mutex::new(None),
                generation: AtomicU64::new(0),
                pinned: Mutex::new(Vec::new()),
                slack: Mutex::new(1.0 / 30.0),
            }),
            lane,
        }
    }

    /// A prefetcher over `pool` with [`Prefetcher::default_workers`].
    pub fn with_defaults(pool: Arc<ReaderPool>) -> Self {
        Self::new(pool, Self::default_workers())
    }

    /// How many decoders run at once: a quarter of the machine's threads,
    /// two at least and four at most. A decoder is single-threaded on the
    /// walk from a keyframe, so this is roughly the share of the machine
    /// decoding may take while the window and the mix keep the rest.
    pub fn default_workers() -> usize {
        std::thread::available_parallelism().map_or(2, |threads| (threads.get() / 4).clamp(2, 4))
    }

    /// The pool the frames land in.
    pub fn pool(&self) -> &Arc<ReaderPool> {
        &self.shared.pool
    }

    /// The cursor as last reported.
    pub fn cursor(&self) -> Option<Cursor> {
        *lock(&self.shared.cursor)
    }

    /// Moves the cursor and queues `ahead`, nearest first, to be decoded
    /// into the pool. Instants queued for the last cursor are forgotten;
    /// frames held for instants now behind this cursor are released; an
    /// instant behind the cursor by the time a worker reaches it is
    /// skipped. `frame_seconds` is how long one output frame lasts, the
    /// slack in "behind".
    pub fn advance(&self, cursor: Cursor, frame_seconds: f64, ahead: Vec<Moment>) {
        let generation = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *lock(&self.shared.cursor) = Some(cursor);
        *lock(&self.shared.slack) = frame_seconds.max(0.0);
        lock(&self.shared.pinned).retain(|(time, _)| !cursor.passed(*time, frame_seconds));

        let mut queue = lock(&self.lane.queue);
        if queue.closed {
            return;
        }
        queue.lanes[Priority::Playback as usize].clear();
        for moment in ahead {
            let shared = Arc::clone(&self.shared);
            queue.lanes[Priority::Playback as usize].push_back(Box::new(move || {
                if shared.generation.load(Ordering::Acquire) != generation {
                    return;
                }
                let slack = *lock(&shared.slack);
                if lock(&shared.cursor).is_some_and(|now| now.passed(moment.time, slack)) {
                    return;
                }
                let frames: Vec<Arc<Frame>> = moment
                    .frames
                    .iter()
                    .filter_map(|request| shared.pool.frame(request).ok())
                    .collect();
                if shared.generation.load(Ordering::Acquire) == generation {
                    lock(&shared.pinned).push((moment.time, frames));
                }
            }));
        }
        drop(queue);
        self.lane.ready.notify_all();
    }

    /// Queues `job` at `priority`; it runs on a worker when its turn comes.
    pub fn submit(&self, priority: Priority, job: impl FnOnce() + Send + 'static) {
        let mut queue = lock(&self.lane.queue);
        if queue.closed {
            return;
        }
        queue.lanes[priority as usize].push_back(Box::new(job));
        drop(queue);
        self.lane.ready.notify_all();
    }

    /// Jobs waiting at `priority`, not counting any running.
    pub fn pending(&self, priority: Priority) -> usize {
        lock(&self.lane.queue).lanes[priority as usize].len()
    }

    /// Frames decoded ahead of the cursor and not yet passed.
    pub fn held_ahead(&self) -> usize {
        lock(&self.shared.pinned).len()
    }

    /// The instants held ahead, in timeline seconds, in no order.
    pub fn held_instants(&self) -> Vec<f64> {
        lock(&self.shared.pinned)
            .iter()
            .map(|(time, _)| *time)
            .collect()
    }

    /// Blocks until every queued job has run. For tests and for a caller
    /// that must know the pool is warm; the window never waits here.
    pub fn drain(&self) {
        let mut queue = lock(&self.lane.queue);
        while queue.running > 0 || queue.lanes.iter().any(|lane| !lane.is_empty()) {
            queue = self
                .lane
                .done
                .wait(queue)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

impl Drop for Prefetcher {
    fn drop(&mut self) {
        let mut queue = lock(&self.lane.queue);
        queue.closed = true;
        for lane in &mut queue.lanes {
            lane.clear();
        }
        drop(queue);
        self.lane.ready.notify_all();
    }
}

/// One worker: takes the most urgent job it may, runs it, repeats, and
/// leaves when the prefetcher is dropped.
fn work(lane: &Lane) {
    loop {
        let (priority, job) = {
            let mut queue = lock(&lane.queue);
            loop {
                if queue.closed {
                    return;
                }
                if let Some(taken) = lane.take(&mut queue) {
                    queue.running += 1;
                    if taken.0.background() {
                        queue.background_running += 1;
                    }
                    if taken.0 == Priority::Proxy {
                        queue.proxy_running += 1;
                    }
                    break taken;
                }
                queue = lane
                    .ready
                    .wait(queue)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        let finished = Finished { lane, priority };
        // A job that panics must not take the counters with it: `running`
        // would never come down, `drain` would wait forever and the reserve
        // would stay spent for the life of the process. The guard counts
        // the job as finished whatever it did (audit 2026-09-23, #2).
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
            log::error!("a {priority:?} job panicked; the worker carries on");
        }
        drop(finished);
    }
}

/// A running job's place in the counters, given back when dropped, so it
/// is given back on a panic too.
struct Finished<'a> {
    lane: &'a Lane,
    priority: Priority,
}

impl Drop for Finished<'_> {
    fn drop(&mut self) {
        let mut queue = lock(&self.lane.queue);
        queue.running -= 1;
        if self.priority.background() {
            queue.background_running -= 1;
        }
        if self.priority == Priority::Proxy {
            queue.proxy_running -= 1;
        }
        drop(queue);
        // A finished job frees a worker for a job the reserve was holding
        // back, and is what a drain waits for.
        self.lane.ready.notify_all();
        self.lane.done.notify_all();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_core::time::FrameRate;
    use std::sync::atomic::AtomicUsize;

    /// A job that panics is counted as finished: the worker carries on,
    /// the next job runs, and a drain returns rather than waiting on a
    /// counter nothing will ever bring down.
    #[test]
    fn a_panicking_job_does_not_take_the_worker_with_it() {
        let pool = Arc::new(ReaderPool::new(1024, 1));
        let prefetcher = Prefetcher::new(pool, 1);
        prefetcher.submit(Priority::Artwork, || panic!("a bad job"));
        let ran = Arc::new(AtomicUsize::new(0));
        let mark = Arc::clone(&ran);
        prefetcher.submit(Priority::Artwork, move || {
            mark.fetch_add(1, Ordering::SeqCst);
        });
        prefetcher.drain();
        assert_eq!(ran.load(Ordering::SeqCst), 1, "the job after the panic ran");
        let queue = lock(&prefetcher.lane.queue);
        assert_eq!(
            (queue.running, queue.background_running, queue.proxy_running),
            (0, 0, 0),
            "the counters came back down"
        );
    }

    /// Jobs run most urgent first, and a background job never takes the
    /// last worker: with one worker and a reserve of none, everything
    /// runs; the order is by priority, then by arrival.
    #[test]
    fn jobs_run_in_priority_order() {
        let pool = Arc::new(ReaderPool::new(1024, 1));
        let prefetcher = Prefetcher::new(pool, 1);
        let order = Arc::new(Mutex::new(Vec::new()));
        // Hold the worker so the rest queue up behind, then release.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        {
            let gate = Arc::clone(&gate);
            prefetcher.submit(Priority::Filmstrip, move || {
                let (open, bell) = &*gate;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = bell.wait(open).unwrap();
                }
            });
        }
        for (priority, name) in [
            (Priority::Artwork, "art-1"),
            (Priority::Proxy, "proxy"),
            (Priority::Filmstrip, "strip"),
            (Priority::Artwork, "art-2"),
            (Priority::Playback, "play"),
        ] {
            let order = Arc::clone(&order);
            prefetcher.submit(priority, move || order.lock().unwrap().push(name));
        }
        {
            let (open, bell) = &*gate;
            *open.lock().unwrap() = true;
            bell.notify_all();
        }
        prefetcher.drain();
        assert_eq!(
            *order.lock().unwrap(),
            vec!["play", "strip", "art-1", "art-2", "proxy"]
        );
    }

    /// With two workers, one is kept clear of background work: two
    /// artwork jobs never run together, and a playback job queued while
    /// one artwork job runs takes the free worker at once.
    #[test]
    fn background_work_leaves_one_worker_free() {
        let pool = Arc::new(ReaderPool::new(1024, 1));
        let prefetcher = Prefetcher::new(pool, 2);
        let running = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        for _ in 0..6 {
            let running = Arc::clone(&running);
            let most = Arc::clone(&most);
            prefetcher.submit(Priority::Artwork, move || {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(15));
                running.fetch_sub(1, Ordering::SeqCst);
            });
        }
        let played = Arc::new(AtomicUsize::new(0));
        {
            let played = Arc::clone(&played);
            prefetcher.submit(Priority::Playback, move || {
                played.store(1, Ordering::SeqCst);
            });
        }
        let started = std::time::Instant::now();
        while played.load(Ordering::SeqCst) == 0 {
            assert!(
                started.elapsed() < std::time::Duration::from_millis(500),
                "playback waited behind the artwork"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        prefetcher.drain();
        assert_eq!(most.load(Ordering::SeqCst), 1, "artwork ran one at a time");
    }

    /// A cursor that turns round keeps what is now ahead of it and lets
    /// go of what is now behind: shuttling back over a play does not
    /// throw the frames just played away, and does not hold what lies
    /// the other way.
    #[test]
    fn a_reversed_cursor_keeps_what_is_ahead_of_it_now() {
        let path = crate::pool::tests::counting_video("prefetch-reverse", 32, 90);
        let pool = Arc::new(ReaderPool::new(64 * 1024 * 1024, 2));
        let prefetcher = Prefetcher::new(Arc::clone(&pool), 1);
        let rate = FrameRate::THIRTY;
        let fps = rate.fps().as_f64();
        let moment = |index: i64| Moment {
            time: index as f64 / fps,
            frames: vec![FrameRequest::new(&path, rate.time_of_frame(index), 16, 16)],
        };
        prefetcher.advance(
            Cursor {
                time: 10.0 / fps,
                direction: Direction::Forward,
                rate: 1.0,
            },
            1.0 / fps,
            (11..19).map(moment).collect(),
        );
        prefetcher.drain();
        assert_eq!(prefetcher.held_ahead(), 8);
        // Turned round at 14: 15.. are behind it now and go; 11..=14 stay
        // (14 within a frame's slack), and 13 down to 6 are asked for.
        prefetcher.advance(
            Cursor {
                time: 14.0 / fps,
                direction: Direction::Backward,
                rate: -1.0,
            },
            1.0 / fps,
            (6..14).rev().map(moment).collect(),
        );
        prefetcher.drain();
        let held = prefetcher.held_instants();
        let slack = 1.0 / fps + 1e-6;
        assert!(
            held.iter().all(|time| *time <= 14.0 / fps + slack),
            "held behind a backward cursor: {held:?}"
        );
        assert!(held.len() >= 8, "the way back is decoded: {held:?}");
        // Nothing the cursor has behind it is ever queued again: an
        // instant past it is skipped, not decoded.
        let before = pool.stats();
        prefetcher.advance(
            Cursor {
                time: 5.0 / fps,
                direction: Direction::Backward,
                rate: -1.0,
            },
            1.0 / fps,
            vec![moment(30)],
        );
        prefetcher.drain();
        assert_eq!(
            pool.stats().since(before).decoded,
            0,
            "30 is behind a backward cursor at 5"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// The cursor's instants are decoded ahead into the pool, held until
    /// passed, and a moved cursor drops what it has passed.
    #[test]
    fn instants_ahead_are_decoded_and_released_once_passed() {
        let path = crate::pool::tests::counting_video("prefetch", 32, 90);
        let pool = Arc::new(ReaderPool::new(64 * 1024 * 1024, 2));
        let prefetcher = Prefetcher::new(Arc::clone(&pool), 1);
        let rate = FrameRate::THIRTY;
        let fps = rate.fps().as_f64();
        let moments = |from: i64| -> Vec<Moment> {
            (from + 1..from + 1 + i64::from(AHEAD))
                .map(|index| Moment {
                    time: index as f64 / fps,
                    frames: vec![FrameRequest::new(&path, rate.time_of_frame(index), 16, 16)],
                })
                .collect()
        };
        let cursor = |index: i64| Cursor {
            time: index as f64 / fps,
            direction: Direction::Forward,
            rate: 1.0,
        };

        prefetcher.advance(cursor(0), 1.0 / fps, moments(0));
        prefetcher.drain();
        assert_eq!(prefetcher.held_ahead(), AHEAD as usize);
        let warmed = pool.stats();
        assert!(warmed.decoded >= u64::from(AHEAD), "{warmed:?}");

        // The pull for frame 5 is a hit.
        pool.frame(&FrameRequest::new(&path, rate.time_of_frame(5), 16, 16))
            .expect("decodes");
        assert_eq!(pool.stats().since(warmed).decoded, 0, "frame 5 was ahead");

        // The cursor at 5: the instants more than a frame behind it are
        // released, 4..=8 stay, and 6..=13 join them.
        prefetcher.advance(cursor(5), 1.0 / fps, moments(5));
        prefetcher.drain();
        let held = prefetcher.held_ahead();
        assert!(
            (AHEAD as usize + 4..=AHEAD as usize + 6).contains(&held),
            "held {held}: the passed instants were released"
        );

        // A cursor far ahead: nothing held behind it survives, and an
        // instant queued for it that it has already passed is skipped -
        // asked for at a size nothing else uses, it is a fresh decode
        // afterwards, so it cannot have been prefetched.
        let mut stale = moments(20);
        stale.insert(
            0,
            Moment {
                time: 2.0 / fps,
                frames: vec![FrameRequest::new(&path, rate.time_of_frame(2), 8, 8)],
            },
        );
        prefetcher.advance(cursor(20), 1.0 / fps, stale);
        prefetcher.drain();
        assert_eq!(prefetcher.held_ahead(), AHEAD as usize);
        let before = pool.stats();
        pool.frame(&FrameRequest::new(&path, rate.time_of_frame(2), 8, 8))
            .expect("decodes");
        assert!(
            pool.stats().since(before).decoded > 0,
            "the passed instant was never decoded"
        );

        let _ = std::fs::remove_file(&path);
    }
}
