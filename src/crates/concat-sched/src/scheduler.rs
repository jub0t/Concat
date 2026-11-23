// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The scheduler itself: the queues, the threads, and the one rule that
//! says what starts next.
//!
//! Everything that decides is in [`Core::pump`], which runs under the one
//! lock after anything changes - a job handed in, a job ended, a job
//! standing aside, a resource growing. Nothing of the caller's ever runs
//! under that lock: not a job, not a callback, not an event. So the lock
//! is held for a few list operations at a time, and a decision is made by
//! the time the call that caused it returns - which is also what lets the
//! tests ask for a snapshot straight after a submit and find the answer
//! there, with nothing to wait for.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::budget::{Budget, Lane, Machine};
use crate::cancel::{CancelReason, Cancelled};
use crate::job::{
    Coalesce, End, Ending, InputKey, Job, JobHandle, JobId, JobInfo, JobKey, JobShared, JobState,
    Never, Outcome, Owner, Refusal, Resource, Said, Sink, Task, Ticket, Typed,
};
use crate::memory::FailureMemory;
use crate::watch::{Event, LaneLoad, LaneSnapshot, ResourceUse, Snapshot};
use crate::{JobContext, lock};

/// Where the events go.
type EventSink = Arc<dyn Fn(&Event) + Send + Sync>;

/// What a new thread runs first.
type ThreadHook = Arc<dyn Fn(Lane) + Send + Sync>;

/// How a scheduler is set up.
pub struct Config {
    budget: Budget,
    queue_limits: [usize; Lane::COUNT],
    failures: usize,
    events: Option<EventSink>,
    thread_start: Option<ThreadHook>,
    shutdown_bound: Duration,
}

impl Config {
    /// The most jobs that wait on each lane unless
    /// [`Config::queue_limit`] says otherwise, in [`Lane::ALL`] order.
    ///
    /// The frame and the save replace what is waiting, so their lines are
    /// short by nature. The lanes an import fills - a card, a waveform, a
    /// probe and an analysis per file - hold a few thousand, which is a
    /// folder nobody has yet dragged in. Past the limit a job is refused
    /// ([`Refusal::QueueFull`]) and its caller is told; nothing grows
    /// without anyone knowing.
    pub const QUEUE_LIMITS: [usize; Lane::COUNT] = [64, 64, 256, 4096, 4096, 4096];

    /// How many failed inputs are remembered unless
    /// [`Config::failure_memory`] says otherwise.
    pub const FAILURES: usize = 1024;

    /// How long a drop waits for the threads unless
    /// [`Config::shutdown_bound`] says otherwise.
    pub const SHUTDOWN_BOUND: Duration = Duration::from_secs(5);

    /// A scheduler with this budget, [checked](Budget::checked).
    pub fn new(budget: Budget) -> Config {
        Config {
            budget: budget.checked(),
            queue_limits: Config::QUEUE_LIMITS,
            failures: Config::FAILURES,
            events: None,
            thread_start: None,
            shutdown_bound: Config::SHUTDOWN_BOUND,
        }
    }

    /// A scheduler sized to `machine`; see [`Budget::for_machine`].
    pub fn for_machine(machine: Machine) -> Config {
        Config::new(Budget::for_machine(machine))
    }

    /// The most jobs that may wait on `lane`. The jobs running are not
    /// counted, and at least one may always wait.
    pub fn queue_limit(mut self, lane: Lane, limit: usize) -> Config {
        self.queue_limits[lane.index()] = limit.max(1);
        self
    }

    /// How many failed inputs are remembered; the oldest goes when one
    /// more arrives. None at all switches the memory off.
    pub fn failure_memory(mut self, limit: usize) -> Config {
        self.failures = limit;
        self
    }

    /// Where each [`Event`] is sent. See [`Event`] for which thread calls
    /// it and what it may do.
    pub fn events(mut self, sink: impl Fn(&Event) + Send + Sync + 'static) -> Config {
        self.events = Some(Arc::new(sink));
        self
    }

    /// What each of the scheduler's threads runs once as it starts, told
    /// its lane.
    ///
    /// This is where the host tells the system what the thread is for -
    /// a background lane's threads marked as background work, so a
    /// machine with efficiency cores puts them there and the system
    /// favours the frame's. That takes a call per platform, which is why
    /// it is the host's and not done here.
    pub fn on_thread_start(mut self, hook: impl Fn(Lane) + Send + Sync + 'static) -> Config {
        self.thread_start = Some(Arc::new(hook));
        self
    }

    /// How long dropping the scheduler waits for its threads before it
    /// leaves the ones still busy behind; see [`Scheduler::shutdown`].
    pub fn shutdown_bound(mut self, bound: Duration) -> Config {
        self.shutdown_bound = bound;
        self
    }
}

/// A job waiting on a lane.
struct Queued {
    job: Arc<JobShared>,
    task: Box<dyn Task>,
    rank: u8,
    /// Its place among jobs of its rank: lower goes first.
    sequence: u64,
    most: usize,
    least: usize,
    needs: Vec<(Resource, u64)>,
    /// Whether it must wait for a running job with its key to end: it
    /// replaced that job.
    exclusive: bool,
    input: Option<InputKey>,
}

/// A job chosen to start, waiting only for a thread of its lane to wake.
/// It holds its cores and its resources already.
struct Ready {
    job: Arc<JobShared>,
    task: Box<dyn Task>,
    width: usize,
    needs: Vec<(Resource, u64)>,
    input: Option<InputKey>,
}

/// A job a thread has.
struct Run {
    job: Arc<JobShared>,
    /// The cores it holds now: what it was granted, or nothing while it
    /// stands aside.
    width: usize,
    /// The cores it was granted when it started, and wants back.
    granted: usize,
    needs: Vec<(Resource, u64)>,
    input: Option<InputKey>,
    /// Standing aside at a checkpoint.
    parked: bool,
    /// Told to carry on; its cores are counted as held again.
    resume: bool,
    /// Where its thread waits while it stands aside.
    wake: Arc<Condvar>,
}

#[derive(Default)]
struct LaneState {
    /// The lane's threads that did start.
    threads: usize,
    /// The jobs waiting, by rank and then by sequence.
    queue: VecDeque<Queued>,
    ready: VecDeque<Ready>,
    /// The jobs running, oldest first.
    running: Vec<Run>,
}

/// How many jobs have a key.
#[derive(Default)]
struct KeyUse {
    /// Waiting or running.
    live: usize,
    /// Running.
    running: usize,
}

struct State {
    budget: Budget,
    lanes: [LaneState; Lane::COUNT],
    /// The cores each lane's jobs hold.
    held: [usize; Lane::COUNT],
    resources: Vec<ResourceUse>,
    keys: HashMap<JobKey, KeyUse>,
    memory: FailureMemory,
    queue_limits: [usize; Lane::COUNT],
    /// Shut down: nothing more is taken and nothing more starts.
    closing: bool,
    /// The threads that have not yet left.
    alive: usize,
    sequence: u64,
}

impl State {
    /// The cores `lane` may hold as things stand: the pool, less what the
    /// lanes above hold, less what is kept for the lanes below.
    ///
    /// What the lanes below hold beyond that is not subtracted. A lane
    /// never waits for one below it: it starts, and the one below is
    /// asked to stand aside.
    fn allowance(&self, lane: Lane) -> usize {
        let mut cores = self.budget.pool;
        for other in Lane::SHARING {
            if other < lane {
                cores = cores.saturating_sub(self.held[other.index()]);
            } else if other > lane {
                cores = cores.saturating_sub(self.budget.lane(other).reserve);
            }
        }
        cores
    }

    /// The cores `lane` could take for one more job.
    fn free(&self, lane: Lane) -> usize {
        self.allowance(lane).saturating_sub(self.held[lane.index()])
    }

    /// The threads of `lane` with nothing to do.
    fn idle_threads(&self, lane: Lane) -> usize {
        let lane = &self.lanes[lane.index()];
        lane.threads
            .saturating_sub(lane.running.len() + lane.ready.len())
    }

    fn capacity(&self, resource: Resource) -> u64 {
        self.resources
            .iter()
            .find(|held| held.resource == resource)
            .map_or(0, |held| held.capacity)
    }

    /// Whether everything in `needs` is free now, and none of it is
    /// spoken for by a job further up the line.
    fn fits(&self, needs: &[(Resource, u64)], claimed: &[Resource]) -> bool {
        needs.iter().all(|(resource, amount)| {
            !claimed.contains(resource)
                && self
                    .resources
                    .iter()
                    .find(|held| held.resource == *resource)
                    .is_some_and(|held| held.held.saturating_add(*amount) <= held.capacity)
        })
    }

    fn take(&mut self, needs: &[(Resource, u64)]) {
        for (resource, amount) in needs {
            if let Some(held) = self
                .resources
                .iter_mut()
                .find(|held| held.resource == *resource)
            {
                held.held = held.held.saturating_add(*amount);
            }
        }
    }

    fn give_back(&mut self, needs: &[(Resource, u64)]) {
        for (resource, amount) in needs {
            if let Some(held) = self
                .resources
                .iter_mut()
                .find(|held| held.resource == *resource)
            {
                held.held = held.held.saturating_sub(*amount);
            }
        }
    }

    fn key_running(&self, key: Option<&JobKey>) -> bool {
        key.and_then(|key| self.keys.get(key))
            .is_some_and(|used| used.running > 0)
    }

    fn key_live(&self, key: &JobKey) -> bool {
        self.keys.get(key).is_some_and(|used| used.live > 0)
    }

    fn key_added(&mut self, key: Option<&JobKey>) {
        if let Some(key) = key {
            self.keys.entry(key.clone()).or_default().live += 1;
        }
    }

    fn key_started(&mut self, key: Option<&JobKey>) {
        if let Some(used) = key.and_then(|key| self.keys.get_mut(key)) {
            used.running += 1;
        }
    }

    /// A job with `key` is gone, from the queue or from its thread.
    fn key_left(&mut self, key: Option<&JobKey>, was_running: bool) {
        let Some(key) = key else {
            return;
        };
        let Some(used) = self.keys.get_mut(key) else {
            return;
        };
        used.live = used.live.saturating_sub(1);
        if was_running {
            used.running = used.running.saturating_sub(1);
        }
        if used.live == 0 {
            self.keys.remove(key);
        }
    }

    /// Every job waiting or running, with no order promised.
    fn jobs(&self) -> impl Iterator<Item = &Arc<JobShared>> {
        self.lanes.iter().flat_map(|lane| {
            lane.running
                .iter()
                .map(|run| &run.job)
                .chain(lane.ready.iter().map(|ready| &ready.job))
                .chain(lane.queue.iter().map(|queued| &queued.job))
        })
    }

    /// Takes out of every lane's line the waiting jobs that `wanted`
    /// picks. They are the caller's to end, once the lock is let go.
    ///
    /// Cancelling several jobs always starts here, under the lock, and
    /// never by cancelling their tokens one after another: the first to
    /// stop would free a thread, and the next in line - one of the very
    /// jobs being cancelled - would be started on it before its own
    /// cancel arrived.
    fn take_waiting(&mut self, wanted: impl Fn(&JobShared) -> bool) -> Vec<Queued> {
        let mut taken = Vec::new();
        for lane in &mut self.lanes {
            let mut at = 0;
            while at < lane.queue.len() {
                if !wanted(&lane.queue[at].job) {
                    at += 1;
                    continue;
                }
                match lane.queue.remove(at) {
                    Some(queued) => taken.push(queued),
                    None => break,
                }
            }
        }
        for queued in &taken {
            self.key_left(queued.job.key.as_ref(), false);
        }
        taken
    }

    /// The jobs a thread has, or has been chosen for, that `wanted` picks.
    fn started_where(&self, wanted: impl Fn(&JobShared) -> bool) -> Vec<Arc<JobShared>> {
        self.lanes
            .iter()
            .flat_map(|lane| {
                lane.running
                    .iter()
                    .map(|run| &run.job)
                    .chain(lane.ready.iter().map(|ready| &ready.job))
            })
            .filter(|job| wanted(job))
            .cloned()
            .collect()
    }

    /// The oldest job waiting or running with `key`.
    fn first_with_key(&self, key: &JobKey) -> Option<JobId> {
        self.jobs()
            .filter(|job| job.key.as_ref() == Some(key))
            .map(|job| job.id)
            .min()
    }

    /// Sets, for every job holding cores, whether it is to stand aside.
    ///
    /// A lane holding more than its [allowance](State::allowance) has
    /// cores to give back, and its newest jobs are looked at first: the
    /// oldest is the nearest to done. A job is asked when at least half
    /// of what it holds is wanted above. Standing aside is all or
    /// nothing - a job cannot hand back three of its eight threads - so
    /// holding an eight-core analysis for the sake of one core would idle
    /// seven; below half, the machine is left that much over-full
    /// instead, which is the smaller waste.
    ///
    /// Asked afresh every time, so a job is only held if the lane above
    /// still wants its cores when it reaches a checkpoint: a frame's
    /// worth of decoding ahead comes and goes without stopping an
    /// analysis that checks once a second.
    fn ask_aside(&self) {
        for lane in Lane::SHARING {
            let index = lane.index();
            let mut over = self.held[index].saturating_sub(self.allowance(lane));
            let state = &self.lanes[index];
            let holding = state
                .ready
                .iter()
                .rev()
                .map(|ready| (&ready.job, ready.width))
                .chain(state.running.iter().rev().map(|run| (&run.job, run.width)));
            for (job, width) in holding {
                if width == 0 {
                    continue;
                }
                let ask = over > 0 && over.saturating_mul(2) >= width;
                job.aside.store(ask, Ordering::Relaxed);
                if ask {
                    over = over.saturating_sub(width);
                }
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        let lanes = Lane::ALL.map(|lane| {
            let state = &self.lanes[lane.index()];
            LaneSnapshot {
                lane,
                workers: state.threads,
                cores: self.held[lane.index()],
                running: state
                    .running
                    .iter()
                    .map(|run| run.job.info())
                    .chain(state.ready.iter().map(|ready| ready.job.info()))
                    .collect(),
                queued: state.queue.iter().map(|queued| queued.job.info()).collect(),
            }
        });
        Snapshot {
            lanes,
            pool: self.budget.pool,
            cores: Lane::SHARING
                .iter()
                .map(|lane| self.held[lane.index()])
                .sum(),
            resources: self.resources.clone(),
            failures: self.memory.len(),
        }
    }
}

/// Notes that each resource in `needs` is spoken for: a job that needs
/// it is waiting, and nothing further down the line may take it first.
fn claim(claimed: &mut Vec<Resource>, needs: &[(Resource, u64)]) {
    for (resource, _) in needs {
        if !claimed.contains(resource) {
            claimed.push(*resource);
        }
    }
}

/// What the scheduler's handle, its threads and its running jobs share.
pub(crate) struct Core {
    budget: Budget,
    state: Mutex<State>,
    /// Where each lane's threads wait for a job.
    work: [Condvar; Lane::COUNT],
    /// Told each time a thread leaves.
    left: Condvar,
    events: Option<EventSink>,
    next_id: AtomicU64,
    shutdown_bound: Duration,
}

impl Core {
    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.state)
    }

    /// Sends an event, keeping a panic in the host's function from the
    /// thread it is called on.
    fn emit(&self, event: Event) {
        let Some(events) = &self.events else {
            return;
        };
        if catch_unwind(AssertUnwindSafe(|| events(&event))).is_err() {
            log::error!(
                "sched: the events function panicked on job {}",
                event.job().id
            );
        }
    }

    /// Starts everything that can start now. The rule, lane by lane from
    /// the most urgent:
    ///
    /// 1. A job standing aside gets its cores back, oldest first, as soon
    ///    as the lane has that many free. If all that is in its way by
    ///    then is younger jobs of its own lane, nothing more is started
    ///    on the lane until it is back: it was there first, and a stream
    ///    of small jobs must not keep it out for good.
    /// 2. Otherwise the jobs waiting are looked at in order - rank, then
    ///    age - for as long as the lane has a thread idle:
    ///    - one that replaced a job still running waits for it to stop,
    ///      and is stepped over;
    ///    - one whose resources are not all free is stepped over, and
    ///      what it needs is spoken for: no job after it, on this lane or
    ///      a lower one, is given any of it first;
    ///    - on a lane that shares the cores, one that cannot have the
    ///      cores it asks for at least stops the lane there. Nothing
    ///      behind it overtakes it for cores, or a wide job would wait
    ///      for ever behind a stream of narrow ones.
    ///
    /// Then every lane holding more than it may is asked for the
    /// difference back ([`State::ask_aside`]).
    fn pump(&self, state: &mut State) {
        if state.closing {
            return;
        }
        let mut claimed: Vec<Resource> = Vec::new();
        for lane in Lane::ALL {
            let index = lane.index();
            // A job standing aside that the lanes above no longer keep
            // out, and that only its own lane's younger jobs do.
            let mut elder_waits = false;
            if lane.shares_cores() {
                for at in 0..state.lanes[index].running.len() {
                    let run = &state.lanes[index].running[at];
                    if !run.parked || run.resume || run.job.token.is_cancelled() {
                        continue;
                    }
                    let granted = run.granted;
                    if state.free(lane) < granted {
                        elder_waits |= state.allowance(lane) >= granted;
                        continue;
                    }
                    state.held[index] += granted;
                    let run = &mut state.lanes[index].running[at];
                    run.width = granted;
                    run.resume = true;
                    run.job.set_state(JobState::Running);
                    run.wake.notify_all();
                }
            }

            let mut at = 0;
            while !elder_waits
                && at < state.lanes[index].queue.len()
                && state.idle_threads(lane) > 0
            {
                let queued = &state.lanes[index].queue[at];
                if queued.exclusive && state.key_running(queued.job.key.as_ref()) {
                    at += 1;
                    continue;
                }
                if !state.fits(&queued.needs, &claimed) {
                    claim(&mut claimed, &queued.needs);
                    at += 1;
                    continue;
                }
                let share = state.budget.lane(lane);
                let width = if lane.shares_cores() {
                    let free = state.free(lane);
                    let least = queued.least.min(state.budget.entitlement(lane)).max(1);
                    if free < least {
                        claim(&mut claimed, &queued.needs);
                        break;
                    }
                    queued.most.min(share.width).min(free)
                } else {
                    queued.most.min(share.width)
                }
                .max(1);
                let Some(queued) = state.lanes[index].queue.remove(at) else {
                    break;
                };
                state.take(&queued.needs);
                if lane.shares_cores() {
                    state.held[index] += width;
                }
                state.key_started(queued.job.key.as_ref());
                queued.job.started(width);
                state.lanes[index].ready.push_back(Ready {
                    job: queued.job,
                    task: queued.task,
                    width: if lane.shares_cores() { width } else { 0 },
                    needs: queued.needs,
                    input: queued.input,
                });
            }
            if !state.lanes[index].ready.is_empty() {
                self.work[index].notify_all();
            }
        }
        state.ask_aside();
    }

    /// Takes a job in: queues it, or ends it at once if it cannot be.
    fn accept(self: &Arc<Self>, task: Box<dyn Task>, said: Said) -> Ticket {
        let id = JobId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let lane = said.lane;
        let index = lane.index();
        let (key, coalesce) = match said.key {
            Some((key, coalesce)) => (Some(key), coalesce),
            None => (None, Coalesce::RunBoth),
        };
        let job = Arc::new(JobShared::new(
            id,
            lane,
            said.label,
            said.owner,
            key.clone(),
        ));

        // Before anyone else can have the token, so it cannot be missed.
        // Weak, or the scheduler would be kept by the jobs it keeps.
        let core = Arc::downgrade(self);
        job.token.on_cancel(move |reason| {
            if let Some(core) = core.upgrade() {
                core.cancelled(id, lane, reason);
            }
        });

        /// What became of the job under the lock.
        enum Verdict {
            Queued,
            FailedBefore,
            Refused(Refusal),
        }

        let mut task = Some(task);
        let mut replaced: Vec<Queued> = Vec::new();
        let mut stopping: Vec<Arc<JobShared>> = Vec::new();
        let verdict = {
            let mut state = self.lock();
            'verdict: {
                if state.closing {
                    break 'verdict Verdict::Refused(Refusal::ShutDown);
                }
                if state.lanes[index].threads == 0 {
                    break 'verdict Verdict::Refused(Refusal::NoWorker);
                }
                if let Some(kept) = said.input.as_ref().and_then(|key| state.memory.recall(key))
                    && task.as_mut().is_some_and(|task| task.recall(kept.as_ref()))
                {
                    break 'verdict Verdict::FailedBefore;
                }
                if let Some((resource, _)) = said
                    .needs
                    .iter()
                    .find(|(resource, amount)| *amount > state.capacity(*resource))
                {
                    break 'verdict Verdict::Refused(Refusal::TooLarge(*resource));
                }

                state.sequence += 1;
                let mut sequence = state.sequence;
                let limit = state.queue_limits[index];
                match (&key, coalesce) {
                    (Some(key), Coalesce::KeepFirst) if state.key_live(key) => {
                        if let Some(first) = state.first_with_key(key) {
                            break 'verdict Verdict::Refused(Refusal::Duplicate(first));
                        }
                    }
                    (Some(key), Coalesce::Replace) if state.key_live(key) => {
                        let same = |job: &JobShared| job.key.as_ref() == Some(key);
                        // Looked at before anything is dropped: a job
                        // refused for want of room replaces nothing.
                        let here = state.lanes[index]
                            .queue
                            .iter()
                            .filter(|queued| same(&queued.job))
                            .count();
                        if state.lanes[index].queue.len() - here >= limit {
                            break 'verdict Verdict::Refused(Refusal::QueueFull);
                        }
                        replaced = state.take_waiting(same);
                        stopping = state.started_where(same);
                        // Its place in the line is the place of the one
                        // it replaces there.
                        sequence = replaced
                            .iter()
                            .filter(|dropped| dropped.job.lane == lane)
                            .map(|dropped| dropped.sequence)
                            .fold(sequence, u64::min);
                    }
                    _ => {}
                }
                if state.lanes[index].queue.len() >= limit {
                    break 'verdict Verdict::Refused(Refusal::QueueFull);
                }
                let Some(task) = task.take() else {
                    break 'verdict Verdict::Refused(Refusal::ShutDown);
                };

                let queue = &mut state.lanes[index].queue;
                let at = queue
                    .iter()
                    .position(|queued| (queued.rank, queued.sequence) > (said.rank, sequence))
                    .unwrap_or(queue.len());
                queue.insert(
                    at,
                    Queued {
                        job: Arc::clone(&job),
                        task,
                        rank: said.rank,
                        sequence,
                        most: said.most,
                        least: said.least,
                        needs: said.needs,
                        exclusive: key.is_some() && coalesce == Coalesce::Replace,
                        input: said.input,
                    },
                );
                state.key_added(key.as_ref());
                self.pump(&mut state);
                Verdict::Queued
            }
        };

        // The lock is let go; now what belongs to the caller's thread.
        for running in stopping {
            running.token.cancel_because(CancelReason::Replaced);
        }
        self.end_unrun(replaced, CancelReason::Replaced);
        match (verdict, task) {
            (Verdict::FailedBefore, Some(task)) => {
                self.conclude(&job, task, End::FailedBefore, None);
            }
            (Verdict::Refused(refusal), Some(mut task)) => {
                let end = task.settle(Never::Refused(refusal));
                self.conclude(&job, task, end, None);
            }
            _ => {}
        }
        Ticket { job }
    }

    /// Ends, as cancelled for `reason`, jobs already taken out of the
    /// line ([`State::take_waiting`]). Called with the lock let go: each
    /// job's callback and event run here.
    fn end_unrun(&self, waiting: Vec<Queued>, reason: CancelReason) {
        for mut queued in waiting {
            queued.job.token.cancel_because(reason);
            let end = queued.task.settle(Never::Cancelled(reason));
            self.conclude(&queued.job, queued.task, end, None);
        }
    }

    /// A job's token was cancelled. One still waiting is taken out of the
    /// line and ended here, on the thread that cancelled it, so it never
    /// runs and nobody waits for a thread to find that out; one standing
    /// aside is woken to see for itself.
    fn cancelled(&self, id: JobId, lane: Lane, reason: CancelReason) {
        let index = lane.index();
        let waiting = {
            let mut state = self.lock();
            let at = state.lanes[index]
                .queue
                .iter()
                .position(|queued| queued.job.id == id);
            match at.and_then(|at| state.lanes[index].queue.remove(at)) {
                Some(queued) => {
                    state.key_left(queued.job.key.as_ref(), false);
                    self.pump(&mut state);
                    Some(queued)
                }
                None => {
                    if let Some(run) = state.lanes[index]
                        .running
                        .iter()
                        .find(|run| run.job.id == id && run.parked)
                    {
                        run.wake.notify_all();
                    }
                    None
                }
            }
        };
        if let Some(mut queued) = waiting {
            let end = queued.task.settle(Never::Cancelled(reason));
            self.conclude(&queued.job, queued.task, end, None);
        }
    }

    /// A job's thread is done with it: what it held goes back, a failure
    /// is remembered, and whatever was waiting on any of that starts.
    fn finish(&self, job: &Arc<JobShared>, task: Box<dyn Task>, ending: Ending, ran: bool) {
        let index = job.lane.index();
        {
            let mut state = self.lock();
            let at = state.lanes[index]
                .running
                .iter()
                .position(|run| run.job.id == job.id);
            if let Some(at) = at {
                let run = state.lanes[index].running.remove(at);
                state.held[index] = state.held[index].saturating_sub(run.width);
                state.give_back(&run.needs);
                state.key_left(job.key.as_ref(), true);
                if let (Some(input), Some(kept)) = (run.input, ending.kept) {
                    state.memory.remember(input, kept);
                }
            }
            job.aside.store(false, Ordering::Relaxed);
            self.pump(&mut state);
        }
        let ran = if ran { job.running_for() } else { None };
        self.conclude(job, task, ending.end, ran);
    }

    /// The end of every job, run or not: its outcome is handed on, its
    /// event sent, and then - last, so that whoever was waiting for the
    /// job finds all of that done - it is marked over.
    fn conclude(&self, job: &Arc<JobShared>, task: Box<dyn Task>, end: End, ran: Option<Duration>) {
        if catch_unwind(AssertUnwindSafe(move || task.deliver())).is_err() {
            log::error!(
                "sched: the callback of {} job {} panicked on being told how it ended",
                job.label,
                job.id
            );
        }
        job.set_state(JobState::Ended);
        self.emit(Event::Finished {
            job: job.info(),
            end: end.clone(),
            ran,
        });
        job.finish(end);
    }

    /// A job at a checkpoint has been asked to stand aside: gives its
    /// cores back and blocks until it has them again, or is cancelled.
    pub(crate) fn stand_aside(&self, job: &Arc<JobShared>) -> Result<(), Cancelled> {
        let index = job.lane.index();
        let wake = {
            let mut state = self.lock();
            // Asked again under the lock: the lane above may have
            // finished between the job's look at the flag and here.
            if !job.aside.load(Ordering::Relaxed) {
                return Ok(());
            }
            let Some(run) = state.lanes[index]
                .running
                .iter_mut()
                .find(|run| run.job.id == job.id)
            else {
                return Ok(());
            };
            let width = std::mem::take(&mut run.width);
            run.parked = true;
            run.resume = false;
            let wake = Arc::clone(&run.wake);
            state.held[index] = state.held[index].saturating_sub(width);
            job.aside.store(false, Ordering::Relaxed);
            job.set_state(JobState::Paused);
            self.pump(&mut state);
            wake
        };
        self.emit(Event::Paused(job.info()));

        let resumed = {
            let mut state = self.lock();
            loop {
                let Some(run) = state.lanes[index]
                    .running
                    .iter_mut()
                    .find(|run| run.job.id == job.id)
                else {
                    break false;
                };
                if run.resume {
                    run.parked = false;
                    run.resume = false;
                    break true;
                }
                if job.token.is_cancelled() {
                    // It leaves to stop, holding no cores while it does.
                    run.parked = false;
                    break false;
                }
                state = wake
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        job.set_state(JobState::Running);
        if resumed {
            self.emit(Event::Resumed(job.info()));
            Ok(())
        } else {
            Err(Cancelled)
        }
    }

    /// One of a lane's threads: takes the jobs chosen for the lane, one
    /// after another, until the scheduler shuts down.
    fn work(self: Arc<Self>, lane: Lane, hook: Option<ThreadHook>) {
        if let Some(hook) = hook
            && catch_unwind(AssertUnwindSafe(|| hook(lane))).is_err()
        {
            log::error!("sched: the thread-start function panicked on the {lane} lane");
        }
        let index = lane.index();
        loop {
            let taken = {
                let mut state = self.lock();
                loop {
                    if let Some(ready) = state.lanes[index].ready.pop_front() {
                        state.lanes[index].running.push(Run {
                            job: Arc::clone(&ready.job),
                            width: ready.width,
                            granted: ready.width,
                            needs: ready.needs,
                            input: ready.input,
                            parked: false,
                            resume: false,
                            wake: Arc::new(Condvar::new()),
                        });
                        break Some((ready.job, ready.task));
                    }
                    if state.closing {
                        break None;
                    }
                    state = self.work[index]
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
            };
            let Some((job, mut task)) = taken else {
                break;
            };
            // Cancelled between being chosen and being taken: it is
            // ended as it would have been in the queue, unrun.
            match job.token.reason() {
                Some(reason) => {
                    let end = task.settle(Never::Cancelled(reason));
                    self.finish(&job, task, Ending { end, kept: None }, false);
                }
                None => {
                    self.emit(Event::Started(job.info()));
                    let ctx = JobContext {
                        job: Arc::clone(&job),
                        core: Arc::clone(&self),
                    };
                    let ending = task.run(&ctx);
                    self.finish(&job, task, ending, true);
                }
            }
        }
        let mut state = self.lock();
        state.alive = state.alive.saturating_sub(1);
        self.left.notify_all();
    }
}

/// The jobs a cancel reached, to wait on.
///
/// What [`Scheduler::cancel_owner`] and [`Scheduler::cancel_key`] return.
/// The jobs that were only waiting are over already; the ones that were
/// running stop at their next check, and [`Stopping::wait`] is how the
/// caller knows they have.
#[derive(Debug)]
pub struct Stopping {
    jobs: Vec<Ticket>,
}

impl Stopping {
    /// How many jobs were cancelled.
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// Whether there was nothing to cancel.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// The jobs cancelled.
    pub fn jobs(&self) -> &[Ticket] {
        &self.jobs
    }

    /// Blocks until every one of the jobs is over - stopped, its outcome
    /// handed on, what it held given back - or `timeout` has passed.
    /// `Err` is the jobs still going then: the ones that are not checking
    /// their token.
    pub fn wait(&self, timeout: Duration) -> Result<(), Vec<JobInfo>> {
        let deadline = Instant::now().checked_add(timeout);
        let late: Vec<JobInfo> = self
            .jobs
            .iter()
            .filter(|ticket| ticket.job.wait_until(deadline).is_none())
            .map(Ticket::info)
            .collect();
        if late.is_empty() { Ok(()) } else { Err(late) }
    }
}

/// What a shutdown left behind.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ShutdownReport {
    /// The jobs that were still running when the wait ran out: cancelled,
    /// and not checking. Their threads are left to finish on their own.
    pub ignored: Vec<JobInfo>,
}

impl ShutdownReport {
    /// Whether every job stopped and every thread was joined.
    pub fn is_clean(&self) -> bool {
        self.ignored.is_empty()
    }
}

/// The scheduler: hand it [`Job`]s and it runs them, each on its lane.
///
/// One for the process. It is `Sync`; share it behind an `Arc`. Dropping
/// it [shuts it down](Scheduler::shutdown).
pub struct Scheduler {
    core: Arc<Core>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Scheduler {
    /// Starts the lanes' threads, as many as the budget says, named
    /// `concat-frame-0`, `concat-save-0` and so on.
    ///
    /// A thread the system refuses to start is said in the log and done
    /// without; a lane left with none refuses its jobs
    /// ([`Refusal::NoWorker`]).
    pub fn new(config: Config) -> Scheduler {
        let core = Arc::new(Core {
            budget: config.budget,
            state: Mutex::new(State {
                budget: config.budget,
                lanes: Default::default(),
                held: [0; Lane::COUNT],
                resources: Vec::new(),
                keys: HashMap::new(),
                memory: FailureMemory::new(config.failures),
                queue_limits: config.queue_limits,
                closing: false,
                alive: 0,
                sequence: 0,
            }),
            work: Default::default(),
            left: Condvar::new(),
            events: config.events,
            next_id: AtomicU64::new(1),
            shutdown_bound: config.shutdown_bound,
        });
        let mut threads = Vec::with_capacity(config.budget.threads());
        {
            // Held while the threads are started, so the count of them is
            // whole before any of them reads it.
            let mut state = core.lock();
            for lane in Lane::ALL {
                for index in 0..config.budget.lane(lane).workers {
                    let worker = Arc::clone(&core);
                    let hook = config.thread_start.clone();
                    let started = std::thread::Builder::new()
                        .name(format!("concat-{}-{index}", lane.name()))
                        .spawn(move || worker.work(lane, hook));
                    match started {
                        Ok(thread) => {
                            threads.push(thread);
                            state.lanes[lane.index()].threads += 1;
                            state.alive += 1;
                        }
                        Err(error) => {
                            log::error!("sched: {lane} thread {index} could not start: {error}");
                        }
                    }
                }
            }
        }
        Scheduler {
            core,
            threads: Mutex::new(threads),
        }
    }

    /// The budget the scheduler was started with.
    pub fn budget(&self) -> &Budget {
        &self.core.budget
    }

    /// Hands a job in and returns the hand on it.
    ///
    /// Always a handle: a job that cannot be taken - the scheduler shut
    /// down, the queue full, a duplicate, an input that failed before -
    /// is over by the time this returns, and the handle says how.
    pub fn submit<T, E>(&self, job: Job<T, E>) -> JobHandle<T, E>
    where
        T: Send + 'static,
        E: Send + 'static,
    {
        let slot = Arc::new(Mutex::new(None));
        let (task, said) = Typed::boxed(job, Sink::Slot(Arc::clone(&slot)));
        JobHandle::new(self.core.accept(task, said), slot)
    }

    /// Hands a job in with what to do when it is over.
    ///
    /// `then` is called exactly once, with how the job ended, on the
    /// thread that ended it: the worker's for a job that ran, and for one
    /// that never did, the thread that cancelled it, replaced it or
    /// handed it in - which may be this one, before this returns. No lock
    /// is held, so it may use the scheduler; it should be quick, and the
    /// usual one only posts the outcome to the thread it belongs on.
    pub fn submit_then<T, E>(
        &self,
        job: Job<T, E>,
        then: impl FnOnce(Outcome<T, E>) + Send + 'static,
    ) -> Ticket
    where
        T: Send + 'static,
        E: Send + 'static,
    {
        let (task, said) = Typed::boxed(job, Sink::Then(Box::new(then)));
        self.core.accept(task, said)
    }

    /// Cancels everything `owner` has, waiting or running, as
    /// [`CancelReason::Owner`]: the project is closing.
    ///
    /// The waiting jobs are over when this returns. The running ones have
    /// been asked; [`Stopping::wait`] on what this returns blocks until
    /// they have stopped. Jobs the owner is given after this are not
    /// touched.
    pub fn cancel_owner(&self, owner: Owner) -> Stopping {
        self.cancel_where(CancelReason::Owner, |job| job.owner == Some(owner))
    }

    /// Cancels every job with `key`, waiting or running, as
    /// [`CancelReason::Requested`].
    pub fn cancel_key(&self, key: &JobKey) -> Stopping {
        self.cancel_where(CancelReason::Requested, |job| job.key.as_ref() == Some(key))
    }

    fn cancel_where(&self, reason: CancelReason, wanted: impl Fn(&JobShared) -> bool) -> Stopping {
        let (waiting, started) = {
            let mut state = self.core.lock();
            let waiting = state.take_waiting(&wanted);
            let started = state.started_where(&wanted);
            // What stood behind them in the line may have room now.
            self.core.pump(&mut state);
            (waiting, started)
        };
        let jobs = started
            .iter()
            .chain(waiting.iter().map(|queued| &queued.job))
            .map(|job| Ticket {
                job: Arc::clone(job),
            })
            .collect();
        // With the lock let go: a cancel runs the job's hooks, and ending
        // a job runs its callback.
        for job in &started {
            job.token.cancel_because(reason);
        }
        self.core.end_unrun(waiting, reason);
        Stopping { jobs }
    }

    /// Says how much of `resource` there is. A job needing it that was
    /// waiting for room starts if there is now enough; less than is held
    /// takes nothing from the jobs holding it, and only keeps new ones
    /// waiting.
    pub fn set_capacity(&self, resource: Resource, capacity: u64) {
        let mut state = self.core.lock();
        match state
            .resources
            .iter_mut()
            .find(|held| held.resource == resource)
        {
            Some(held) => held.capacity = capacity,
            None => state.resources.push(ResourceUse {
                resource,
                capacity,
                held: 0,
            }),
        }
        self.core.pump(&mut state);
    }

    /// Whether a job run on `input` would be answered with a remembered
    /// failure.
    pub fn failed_before(&self, input: &InputKey) -> bool {
        self.core.lock().memory.recall(input).is_some()
    }

    /// Forgets that `input` failed, so the next job run on it is run: the
    /// person asked again, or what made it fail has changed. Returns
    /// whether there was a failure to forget.
    pub fn forget(&self, input: &InputKey) -> bool {
        self.core.lock().memory.forget(input)
    }

    /// Forgets every remembered failure: the network came back.
    pub fn forget_all(&self) {
        self.core.lock().memory.clear();
    }

    /// Everything waiting and running, at this moment.
    pub fn snapshot(&self) -> Snapshot {
        self.core.lock().snapshot()
    }

    /// How busy each lane is, in [`Lane::ALL`] order, without the jobs:
    /// six small numbers and no allocation.
    pub fn load(&self) -> [LaneLoad; Lane::COUNT] {
        let state = self.core.lock();
        Lane::ALL.map(|lane| {
            let lanes = &state.lanes[lane.index()];
            LaneLoad {
                running: lanes.running.len() + lanes.ready.len(),
                queued: lanes.queue.len(),
                cores: state.held[lane.index()],
            }
        })
    }

    /// Stops everything: no more jobs are taken, the waiting ones end as
    /// [`CancelReason::Shutdown`], the running ones are asked to stop,
    /// and the threads are joined.
    ///
    /// It waits for the threads no longer than `bound`. A job still
    /// running then has ignored its token; it is named in the report and
    /// its thread is left to end when the job does, since a thread cannot
    /// be stopped from outside. Calling this again reports what is still
    /// running then.
    ///
    /// Called from inside a job, it does not wait for that job, which
    /// will be in the report.
    pub fn shutdown(&self, bound: Duration) -> ShutdownReport {
        let core = &self.core;
        let (waiting, running) = {
            let mut state = core.lock();
            state.closing = true;
            let waiting = state.take_waiting(|_| true);
            let running = state.started_where(|_| true);
            // Every thread looks again: at a job chosen for it that it
            // must now end unrun, or at the way out.
            for threads in &core.work {
                threads.notify_all();
            }
            (waiting, running)
        };
        for job in &running {
            job.token.cancel_because(CancelReason::Shutdown);
        }
        core.end_unrun(waiting, CancelReason::Shutdown);

        let me = std::thread::current().id();
        let mut threads = lock(&self.threads);
        let mine = threads
            .iter()
            .filter(|thread| thread.thread().id() == me)
            .count();
        let deadline = Instant::now().checked_add(bound);
        let (all_left, ignored) = {
            let mut state = core.lock();
            loop {
                if state.alive <= mine {
                    break;
                }
                state = match deadline {
                    None => core
                        .left
                        .wait(state)
                        .unwrap_or_else(|poisoned| poisoned.into_inner()),
                    Some(deadline) => {
                        let left = deadline.saturating_duration_since(Instant::now());
                        if left.is_zero() {
                            break;
                        }
                        core.left
                            .wait_timeout(state, left)
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .0
                    }
                };
            }
            let ignored: Vec<JobInfo> = state.jobs().map(|job| job.info()).collect();
            (state.alive <= mine, ignored)
        };
        for thread in threads.drain(..) {
            if thread.thread().id() == me {
                continue;
            }
            // A thread that has left its loop is a few instructions from
            // its end, so joining it is not a wait. One still in a job is
            // let go of instead.
            if (all_left || thread.is_finished()) && thread.join().is_err() {
                log::error!("sched: a worker thread ended in a panic");
            }
        }
        ShutdownReport { ignored }
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        let report = self.shutdown(self.core.shutdown_bound);
        for job in &report.ignored {
            log::warn!(
                "sched: {} job {} on the {} lane was still running {} after its cancel; its thread is left behind",
                job.label,
                job.id,
                job.lane,
                self.core.shutdown_bound.as_secs_f32()
            );
        }
    }
}
